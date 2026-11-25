package trace

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"slices"
	"strings"
)

type segment struct {
	kind, role, tool, path, text string
	slot                         int
	breakpoint                   bool
}

func previousSegments(root *os.Root, name, api string) []segment {
	part, ok := strings.CutPrefix(name, "bodies/")
	if !ok || filepath.Base(part) != part || !component(part) || !strings.HasSuffix(part, ".req.gz") {
		return nil
	}
	info, err := root.Lstat(name)
	if err != nil || !info.Mode().IsRegular() || info.Size() > maxBody+65536 {
		return nil
	}
	f, err := root.Open(name)
	if err != nil {
		return nil
	}
	defer f.Close()
	z, err := gzip.NewReader(f)
	if err != nil {
		return nil
	}
	defer z.Close()
	b, err := io.ReadAll(io.LimitReader(z, maxBody+1))
	if err != nil {
		return nil
	}
	return segments(api, b)
}
func (s segment) same(o segment) bool {
	return s.kind == o.kind && s.role == o.role && s.slot == o.slot && s.text == o.text
}

type object map[string]json.RawMessage

func obj(raw json.RawMessage) object { var o object; _ = json.Unmarshal(raw, &o); return o }
func array(raw json.RawMessage) []json.RawMessage {
	var a []json.RawMessage
	_ = json.Unmarshal(raw, &a)
	return a
}
func field(o object, k string) string { var s string; _ = json.Unmarshal(o[k], &s); return s }

var identifier = regexp.MustCompile(`^[a-zA-Z0-9_.-]{1,80}$`)

func identifierOrUnknown(s string) string {
	if identifier.MatchString(s) {
		return s
	}
	return "unknown"
}

// Cache markers and Responses item IDs can move without changing the
// rendered prompt. Preserve the wire order of every other object member.
func segmentText(raw json.RawMessage, stripID bool) (string, bool) {
	var buf bytes.Buffer
	if json.Compact(&buf, raw) != nil {
		return "", false
	}
	data := buf.Bytes()
	if len(data) == 0 || data[0] != '{' {
		return buf.String(), false
	}
	o := obj(data)
	_, marked := o["cache_control"]
	if !marked && (!stripID || o["id"] == nil) {
		return buf.String(), false
	}
	d := json.NewDecoder(bytes.NewReader(data))
	_, _ = d.Token()
	var parts []string
	for d.More() {
		start := int(d.InputOffset())
		if data[start] == ',' {
			start++
		}
		k, _ := d.Token()
		var value json.RawMessage
		_ = d.Decode(&value)
		if k != "cache_control" && !(stripID && k == "id") {
			parts = append(parts, string(data[start:d.InputOffset()]))
		}
	}
	return "{" + strings.Join(parts, ",") + "}", marked
}

func segments(api string, body []byte) []segment {
	if len(body) > maxBody || !analysisShape(body) {
		return nil
	}
	root := obj(body)
	key := "messages"
	switch api {
	case "responses":
		key = "input"
	case "gemini":
		key = "contents"
	case "chat_completions", "deepseek", "anthropic":
	default:
		return nil
	}
	if root[key] == nil {
		return nil
	}
	var out []segment
	calls := map[string]string{}
	add := func(kind, role, name, path string, slot int, raw json.RawMessage, stripID bool) {
		if len(out) > maxSegments || len(raw) == 0 || bytes.Equal(raw, []byte("null")) {
			return
		}
		t, br := segmentText(raw, stripID)
		out = append(out, segment{kind: kind, role: role, tool: identifierOrUnknown(name), path: path, slot: slot, text: t, breakpoint: br})
	}
	tools := func(raw json.RawMessage, path string) {
		for i, v := range array(raw) {
			o := obj(v)
			name := field(o, "name")
			if name == "" {
				name = field(obj(o["function"]), "name")
			}
			add("tool-def", "", name, fmt.Sprintf("%s[%d]", path, i), 0, v, false)
		}
	}
	if api == "gemini" {
		// Preserve the adapter's actual JSON member order. This is structural
		// evidence, not a claim about Gemini's internal cache serialization.
		d := json.NewDecoder(bytes.NewReader(body))
		_, _ = d.Token()
		parts := func(raw json.RawMessage, base, role, kind string) {
			for j, part := range array(obj(raw)["parts"]) {
				o := obj(part)
				k, name := kind, ""
				switch {
				case o["functionCall"] != nil:
					k, name = "tool-call", field(obj(o["functionCall"]), "name")
				case o["functionResponse"] != nil:
					k, name = "tool-result", field(obj(o["functionResponse"]), "name")
				case o["inlineData"] != nil || o["fileData"] != nil:
					k = "image"
				case string(o["thought"]) == "true" || o["thoughtSignature"] != nil:
					k = "thinking"
				}
				add(k, role, name, fmt.Sprintf("%s.parts[%d]", base, j), j, part, false)
			}
		}
		for d.More() {
			key, _ := d.Token()
			var raw json.RawMessage
			if d.Decode(&raw) != nil {
				return nil
			}
			switch key {
			case "contents":
				for i, c := range array(raw) {
					parts(c, fmt.Sprintf("contents[%d]", i), field(obj(c), "role"), "message")
				}
			case "systemInstruction":
				parts(raw, "systemInstruction", "system", "system")
			case "tools":
				for i, tool := range array(raw) {
					path := fmt.Sprintf("tools[%d]", i)
					if decl := obj(tool)["functionDeclarations"]; decl != nil {
						tools(decl, path+".functionDeclarations")
					} else {
						add("tool-def", "", "", path, 0, tool, false)
					}
				}
			case "generationConfig", "toolConfig", "cachedContent":
				k := key.(string)
				add("options", k, "", k, 0, raw, false)
			}
		}
		if len(out) > maxSegments {
			return nil
		}
		return out
	}
	tools(root["tools"], "tools")
	content := func(raw json.RawMessage, path, kind, role string) {
		if len(raw) == 0 {
			return
		}
		if raw[0] != '[' {
			add(kind, role, "", path, 0, raw, false)
			return
		}
		for i, part := range array(raw) {
			o := obj(part)
			k, name := kind, ""
			switch field(o, "type") {
			case "tool_result":
				k, name = "tool-result", calls[field(o, "tool_use_id")]
			case "server_tool_use", "tool_use":
				k, name = "tool-call", field(o, "name")
				calls[field(o, "id")] = name
			case "thinking", "redacted_thinking":
				k = "thinking"
			case "image", "image_url", "document", "input_image", "input_file", "input_audio", "file":
				k = "image"
			}
			add(k, role, name, fmt.Sprintf("%s[%d]", path, i), i, part, false)
		}
	}
	content(root["system"], "system", "system", "system")
	content(root["instructions"], "instructions", "system", "system")
	addOptions := func() {
		for _, k := range []string{"tool_choice", "thinking", "reasoning", "reasoning_effort", "response_format", "text", "parallel_tool_calls"} {
			add("options", k, "", k, 0, root[k], false)
		}
	}
	pendingOptions := api == "chat_completions" || api == "deepseek"
	if !pendingOptions {
		addOptions()
	}
	items := array(root[key])
	if key == "input" && len(root[key]) > 0 && root[key][0] == '"' {
		content(root[key], key, "message", "user")
	}
	for i, item := range items {
		o := obj(item)
		path := fmt.Sprintf("%s[%d]", key, i)
		role := field(o, "role")
		if pendingOptions && role != "system" && role != "developer" {
			addOptions()
			pendingOptions = false
		}
		kind := "message"
		switch role {
		case "system", "developer":
			kind = "system"
		case "tool":
			add("tool-result", role, calls[field(o, "tool_call_id")], path, 0, item, false)
			continue
		}
		switch field(o, "type") {
		case "additional_tools":
			tools(o["tools"], path+".tools")
			continue
		case "function_call", "custom_tool_call":
			name := field(o, "name")
			calls[field(o, "call_id")] = name
			add("tool-call", "assistant", name, path, 0, item, true)
			continue
		case "function_call_output", "custom_tool_call_output":
			add("tool-result", "tool", calls[field(o, "call_id")], path, 0, item, true)
			continue
		case "reasoning":
			add("thinking", "assistant", "", path, 0, item, true)
			continue
		}
		if o["content"] == nil && key == "input" {
			add("other", role, "", path, 0, item, true)
		}
		content(o["content"], path+".content", kind, role)
		for _, k := range []string{"reasoning", "reasoning_content"} {
			add("thinking", role, "", path+"."+k, 0, o[k], false)
		}
		for j, call := range array(o["tool_calls"]) {
			c := obj(call)
			name := field(obj(c["function"]), "name")
			calls[field(c, "id")] = name
			add("tool-call", role, name, fmt.Sprintf("%s.tool_calls[%d]", path, j), j, call, false)
		}
	}
	if pendingOptions {
		addOptions()
	}
	if len(out) > maxSegments {
		return nil
	}
	return out
}

// Bound decoder allocation on densely packed arrays/objects without rejecting
// large opaque strings (for example, one attachment). Never compare a prefix
// of a request whose structure exceeded the analysis budget.
func analysisShape(body []byte) bool {
	in, escape := false, false
	n := 0
	for _, b := range body {
		if in {
			switch {
			case escape:
				escape = false
			case b == '\\':
				escape = true
			case b == '"':
				in = false
			}
			continue
		}
		switch b {
		case '"':
			in = true
		case ',', ':', '{', '[':
			n++
			if n > 65536 {
				return false
			}
		}
	}
	return true
}

func onlyKeyOrder(a, b segment) bool {
	if a.kind != b.kind || a.role != b.role || a.slot != b.slot || a.text == b.text {
		return false
	}
	decode := func(s string) any {
		d := json.NewDecoder(strings.NewReader(s))
		d.UseNumber()
		var v any
		if d.Decode(&v) != nil {
			return nil
		}
		return v
	}
	x, y := decode(a.text), decode(b.text)
	if x == nil || y == nil {
		return false
	}
	if reflect.DeepEqual(x, y) {
		return differentKeyOrder(a.text, b.text)
	}
	if a.kind != "tool-call" {
		return false
	}
	// Chat and Responses both send JSON arguments inside a JSON string.
	// Do not reinterpret other strings (IDs, prompts, custom tool inputs).
	args := func(v any) (map[string]any, string) {
		o, _ := v.(map[string]any)
		if fn, ok := o["function"].(map[string]any); ok {
			o = fn
		}
		s, _ := o["arguments"].(string)
		return o, s
	}
	ox, sx := args(x)
	oy, sy := args(y)
	var cx, cy bytes.Buffer
	if !analysisShape([]byte(sx)) || !analysisShape([]byte(sy)) || json.Compact(&cx, []byte(sx)) != nil || json.Compact(&cy, []byte(sy)) != nil || cx.String() == cy.String() {
		return false
	}
	vx, vy := decode(sx), decode(sy)
	if !reflect.DeepEqual(vx, vy) || !differentKeyOrder(cx.String(), cy.String()) {
		return false
	}
	delete(ox, "arguments")
	delete(oy, "arguments")
	return reflect.DeepEqual(x, y)
}

// Inputs are compact, valid JSON. Semantic equality alone does not show
// reordering: escaped and literal spellings of a string can compare equal.
func differentKeyOrder(a, b string) bool {
	keys := func(s string) []string {
		var seq []string
		d := json.NewDecoder(strings.NewReader(s))
		d.UseNumber()
		for {
			tok, err := d.Token()
			if err != nil {
				break
			}
			if key, ok := tok.(string); ok && d.InputOffset() < int64(len(s)) && s[d.InputOffset()] == ':' {
				seq = append(seq, key)
			}
		}
		return seq
	}
	return !slices.Equal(keys(a), keys(b))
}

var volatilePatterns = []struct {
	name    string
	pattern *regexp.Regexp
}{
	{"timestamp", regexp.MustCompile(`\b(?:\d{4}[-/.]\d{1,2}[-/.]\d{1,2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?|1[6-9]\d{8}(?:\d{3})?)\b`)},
	{"uuid", regexp.MustCompile(`(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b`)},
	{"temp-path", regexp.MustCompile(`(?i)(?:/tmp/|\\+temp\\+)[^"'\s]*`)},
}
var skillLine = regexp.MustCompile(`\\n- ([A-Za-z0-9_.-]{1,80}): `)

func divergence(old, cur []segment, i int, compacted bool) (cause, path, volatile string) {
	a := old[i]
	path = a.path
	if compacted || i >= len(cur) {
		return "history:truncated", path, ""
	}
	b := cur[i]
	cause = "history:shape"
	switch {
	case a.kind == "tool-def" || b.kind == "tool-def":
		names := func(s []segment) []string {
			var n []string
			for _, v := range s {
				if v.kind == "tool-def" {
					n = append(n, v.tool)
				}
			}
			slices.Sort(n)
			return n
		}
		switch {
		case !slices.Equal(names(old), names(cur)):
			return "tool-defs:set", path, ""
		case a.tool != b.tool:
			return "tool-defs:order", path, ""
		default:
			cause = "tool-def:" + a.tool
		}
	case a.kind == "system":
		cause = "system"
	case a.kind == "options" || b.kind == "options":
		cause = "options"
		if b.kind == "options" {
			path = b.path
		}
	case a.kind == "thinking" || b.kind == "thinking":
		cause = "thinking"
	case a.kind == b.kind:
		switch a.kind {
		case "tool-result", "tool-call":
			cause = a.kind + ":" + a.tool
		case "message", "image":
			cause = "message:" + identifierOrUnknown(a.role)
		}
	}
	x, y := a.text, b.text
	lo := 0
	for lo < min(len(x), len(y)) && x[lo] == y[lo] {
		lo++
	}
	suffix := 0
	for suffix < min(len(x), len(y))-lo && x[len(x)-1-suffix] == y[len(y)-1-suffix] {
		suffix++
	}
	if cause == "system" && strings.Contains(x, "<skills_instructions>") && strings.Contains(y, "<skills_instructions>") {
		names := func(s string) []string {
			var n []string
			for _, m := range skillLine.FindAllStringSubmatch(s, -1) {
				n = append(n, m[1])
			}
			slices.Sort(n)
			return n
		}
		if !slices.Equal(names(x), names(y)) {
			cause = "skills:set"
		} else if start := strings.LastIndex(y[:lo], `\n- `); start >= 0 && !strings.Contains(y[start+2:lo], `\n`) {
			if m := skillLine.FindStringSubmatch(y[start:]); m != nil {
				cause = "skill:" + m[1]
			}
		}
	}
	for _, p := range volatilePatterns {
		for _, s := range []string{x, y} {
			for _, m := range p.pattern.FindAllStringIndex(s, -1) {
				if m[0] < len(s)-suffix && m[1] > lo {
					return cause + ":volatile", path, p.name
				}
			}
		}
	}
	return cause, path, ""
}
