package trace

import (
	"bytes"
	"encoding/json"
	"fmt"
	"maps"
	"regexp"
	"slices"
	"strings"
	"unicode/utf8"
)

func marker(label string, b []byte) string {
	return fmt.Sprintf("<%s: %d chars, sha256:%s>", label, utf8.RuneCount(b), digest(b))
}

// Bound container/depth overhead independently of the capture's byte limit.
func decodeJSON(b []byte, v any) bool {
	if !utf8.Valid(b) || !json.Valid(b) {
		return false
	}
	depth, n := 0, 0
	quoted, escape := false, false
	for _, c := range b {
		if quoted {
			if escape {
				escape = false
			} else if c == '\\' {
				escape = true
			} else if c == '"' {
				quoted = false
			}
			continue
		}
		switch c {
		case '"':
			quoted = true
		case '{', '[':
			depth++
			n++
		case '}', ']':
			depth--
		case ',':
			n++
		}
		if depth > 96 || n > 65536 {
			return false
		}
	}
	d := json.NewDecoder(bytes.NewReader(b))
	d.UseNumber()
	return d.Decode(v) == nil
}

var fingerprint = regexp.MustCompile(`^<(system prompt|tool schemas): [0-9]{1,9} chars, sha256:[a-f0-9]{64}>$`)

type promptSet map[string]bool
type echoes struct {
	*strings.Replacer
	spans *strings.Replacer
}

func (p promptSet) replacer() *echoes {
	keys := make([]string, 0, len(p))
	for k := range p {
		keys = append(keys, k)
	}
	slices.SortFunc(keys, func(a, b string) int { return len(b) - len(a) })
	pairs := make([]string, 0, 2*len(keys))
	for _, k := range keys {
		pairs = append(pairs, k, "*")
	}
	direct := strings.NewReplacer(pairs...)
	for i := 0; i < len(pairs); i += 2 {
		pairs[i+1] = strings.Repeat("\x00", len(pairs[i]))
	}
	return &echoes{direct, strings.NewReplacer(pairs...)}
}

func structural(key string, v any) bool {
	value, _ := v.(string)
	switch key {
	case "role":
		return value == "system" || value == "developer"
	case "type":
		return value == "text" || value == "input_text" || value == "output_text" || value == "ephemeral"
	}
	return false
}

func (p promptSet) collect(v any) error {
	switch x := v.(type) {
	case string:
		if x != "" {
			p[x] = true
			// Some providers embed a serialized request in an error string.
			quoted, _ := json.Marshal(x)
			p[string(quoted[1:len(quoted)-1])] = true
			if len(p) > 4096 {
				return tooLarge
			}
		}
	case map[string]any:
		for k, val := range x {
			// Provider block discriminators are structure, not instruction text.
			if structural(k, val) {
				continue
			}
			if err := p.collect(val); err != nil {
				return err
			}
		}
	case []any:
		for _, val := range x {
			if err := p.collect(val); err != nil {
				return err
			}
		}
	}
	return nil
}

var separators = strings.NewReplacer("_", "", "-", "")

func normalized(s string) string { return strings.ToLower(separators.Replace(s)) }

// Preserve provider content-block framing, fingerprinting decoded text rather
// than its JSON spelling. Unrecognized protected blocks are masked wholesale.
func protect(v any, label string) any {
	if text, ok := v.(string); ok {
		return marker(label, []byte(text))
	}
	if label == "system prompt" {
		switch x := v.(type) {
		case []any:
			for i, e := range x {
				x[i] = protect(e, label)
			}
			return x
		case map[string]any:
			for k, val := range x {
				if structural(k, val) {
					continue
				}
				x[k] = protect(val, label)
			}
			return x
		}
	}
	data, _ := json.Marshal(v)
	return marker(label, data)
}
func (p promptSet) request(body []byte) ([]byte, bool, error) {
	var root map[string]any
	if !decodeJSON(body, &root) || root == nil {
		return body, false, nil
	}
	known := false
	for k, v := range root {
		switch normalized(k) {
		case "messages", "contents":
			_, ok := v.([]any)
			known = known || ok
		case "input":
			switch v.(type) {
			case string, []any:
				known = true
			}
		case "system", "instructions", "systeminstruction", "systeminstructions":
			known = true
		}
	}
	if !known {
		return body, false, nil
	}
	var visit func(any) error
	visit = func(v any) error {
		switch x := v.(type) {
		case []any:
			for _, e := range x {
				if err := visit(e); err != nil {
					return err
				}
			}
		case map[string]any:
			role, _ := x["role"].(string)
			protected := strings.EqualFold(role, "system") || strings.EqualFold(role, "developer")
			for k, val := range x {
				label := ""
				switch normalized(k) {
				case "system", "instructions", "systeminstruction", "systeminstructions", "systemprompt", "developerinstructions":
					label = "system prompt"
				case "tools", "functions", "functiondeclarations":
					label = "tool schemas"
				default:
					if protected && k != "role" {
						label = "system prompt"
					}
				}
				if label != "" {
					// Tool names and schema enums (e.g. "read" or "auto") are also
					// meaningful diagnostic values outside their definitions.
					if label == "system prompt" {
						if err := p.collect(val); err != nil {
							return err
						}
					}
					x[k] = protect(val, label)
				} else if err := visit(val); err != nil {
					return err
				}
			}
		}
		return nil
	}
	if err := visit(root); err != nil {
		return nil, false, err
	}
	b, err := json.Marshal(root)
	return b, true, err
}

func cleanJSON(v any, s *echoes) any {
	switch x := v.(type) {
	case string:
		if len(x) < 140 && strings.HasPrefix(x, "<") && fingerprint.MatchString(x) {
			return x
		}
		return s.Replace(x)
	case map[string]any:
		out := make(map[string]any, len(x))
		for k, val := range x {
			out[s.Replace(k)] = cleanJSON(val, s)
		}
		return out
	case []any:
		for i, val := range x {
			x[i] = cleanJSON(val, s)
		}
	}
	return v
}

// Keep provider JSON and SSE evidence, not gzip headers or uninterpretable binary.
func safeEvidence(b []byte, s *echoes) []byte {
	trim := bytes.TrimSpace(b)
	if !utf8.Valid(b) {
		return []byte(marker("body omitted", b))
	}
	if bytes.HasPrefix(trim, []byte("{")) || bytes.HasPrefix(trim, []byte("[")) {
		var v any
		if decodeJSON(b, &v) {
			out, _ := json.Marshal(cleanJSON(v, s))
			return out
		}
		return []byte(marker("body omitted", b))
	}
	if bytes.HasPrefix(trim, []byte("data:")) || bytes.Contains(b, []byte("\ndata:")) || bytes.Contains(b, []byte("\rdata:")) {
		return cleanEvents(b, s)
	}
	return []byte(s.Replace(string(b)))
}

// Reassemble common streamed text channels for echo detection. A protected
// string may straddle multiple valid JSON data frames; in that case suppress
// the channel's fragments but retain HTTP/SSE event and usage evidence.
func cleanEvents(b []byte, s *echoes) []byte {
	normalized := bytes.ReplaceAll(bytes.ReplaceAll(b, []byte("\r\n"), []byte("\n")), []byte("\r"), []byte("\n"))
	if bytes.Count(normalized, []byte("\n")) > 65536 {
		return []byte(marker("body omitted", b))
	}
	lines := bytes.Split(normalized, []byte("\n"))
	groups := map[string]*strings.Builder{}
	masks := map[string]string{}
	offsets := map[string]int{}
	var walk func(any, bool)
	walk = func(v any, apply bool) {
		switch x := v.(type) {
		case []any:
			for _, e := range x {
				walk(e, apply)
			}
		case map[string]any:
			for _, k := range slices.Sorted(maps.Keys(x)) {
				val := x[k]
				if str, ok := val.(string); ok {
					switch k {
					case "content", "text", "delta", "thinking", "arguments", "partial_json", "output_text", "reasoning_content":
						if !apply {
							g := groups[k]
							if g == nil {
								g = &strings.Builder{}
								groups[k] = g
							}
							g.WriteString(str)
						} else if mask, ok := masks[k]; ok {
							i := offsets[k]
							offsets[k] += len(str)
							part := mask[i : i+len(str)]
							if strings.ContainsRune(part, 0) {
								var out strings.Builder
								for j := 0; j < len(part); j++ {
									if part[j] != 0 {
										out.WriteByte(str[j])
									} else if j == 0 || part[j-1] != 0 {
										out.WriteByte('*')
									}
								}
								x[k] = out.String()
							}
						}
					}
				} else {
					walk(val, apply)
				}
			}
		}
	}
	// Parse one frame at a time in both passes; a compact SSE file can expand
	// into many objects. Do not retain an unbounded decoded event tree.
	for _, apply := range []bool{false, true} {
		for i, line := range lines {
			payload, ok := bytes.CutPrefix(line, []byte("data:"))
			if !ok {
				if apply {
					lines[i] = []byte(s.Replace(string(line)))
				}
				continue
			}
			payload = bytes.TrimSpace(payload)
			if bytes.Equal(payload, []byte("[DONE]")) {
				continue
			}
			var v any
			if !decodeJSON(payload, &v) {
				if apply {
					lines[i] = []byte("data: " + marker("frame omitted", payload))
				}
				continue
			}
			walk(v, apply)
			if apply {
				data, _ := json.Marshal(cleanJSON(v, s))
				lines[i] = append([]byte("data: "), data...)
			}
		}
		if !apply {
			for k, g := range groups {
				str := g.String()
				masked := s.spans.Replace(str)
				if masked != str {
					masks[k] = masked
				}
			}
		}
	}
	return bytes.Join(lines, []byte("\n"))
}
