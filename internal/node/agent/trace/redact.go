package trace

import (
	"bytes"
	"encoding/json"
	"net/http"
	"regexp"
	"slices"
	"strings"
	"unicode/utf8"
)

const mask = "[REDACTED]"

var credentialShape = regexp.MustCompile(`(?is)-----BEGIN [A-Z0-9 -]+-----.*?(?:-----END [A-Z0-9 -]+-----|$)|\b(?:Bearer|Basic)[ \t]+[^\s,;"']+|\b(?:sk-|gh[pousr]_|github_pat_|glpat-|xox[baprs]-|AKIA|ASIA|AIza|ya29\.|hf_|npm_)[a-zA-Z0-9_./+=-]+|\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+`)
var assignment = regexp.MustCompile(`(?i)\b(api[_-]?key|authorization|(?:access_|refresh_)?token|secret|password|cookie|session)["']?\s*[:=]\s*[^\r\n]+`)

func scrub(s string, secrets []string) string {
	for _, v := range secrets {
		if v != "" {
			s = strings.ReplaceAll(s, v, mask)
		}
	}
	return assignment.ReplaceAllString(credentialShape.ReplaceAllString(s, mask), "$1="+mask)
}

// CredentialShaped is also used by the observation ledger, which can exist
// without private capture and must never retain a credential as a gateway ID.
func CredentialShaped(s string) bool {
	return credentialShape.MatchString(s) || assignment.MatchString(s)
}

func keySensitive(key string) bool {
	k := strings.ToLower(strings.ReplaceAll(key, "-", "_"))
	switch k {
	case "apikey", "api_key", "authorization", "token", "secret", "password", "cookie", "session":
		return true
	}
	for _, suffix := range []string{"_api_key", "_token", "_secret", "_password", "_cookie", "_session"} {
		if strings.HasSuffix(k, suffix) {
			return true
		}
	}
	switch k {
	case "accesstoken", "refreshtoken", "idtoken", "clientsecret", "sessiontoken", "privatekey", "private_key", "proxy_authorization", "set_cookie", "secretaccesskey", "secret_access_key", "x_goog_api_key":
		return true
	}
	return false
}

type headerSnapshot struct {
	Headers http.Header `json:"headers,omitempty"`
	Dropped []string    `json:"dropped_headers,omitempty"`
	Body    string      `json:"body,omitempty"`
}

func headers(h http.Header, secrets []string) headerSnapshot {
	out := headerSnapshot{Headers: make(http.Header)}
	for k, v := range h {
		name := strings.ToLower(k)
		switch name {
		case "content-type", "content-length", "user-agent", "accept", "anthropic-version", "anthropic-beta", "x-request-id", "request-id", "cf-ray", "x-oneapi-request-id":
			for _, x := range v {
				out.Headers.Add(name, scrub(x, secrets))
			}
		default:
			out.Dropped = append(out.Dropped, scrub(name, secrets))
		}
	}
	slices.Sort(out.Dropped)
	return out
}

// JSON is already validated. Splice replacements into the original rather than
// serializing decoded objects: ordering, number spellings and whitespace are evidence.
type jsonEditor struct {
	p        []byte
	i, last  int
	out      bytes.Buffer
	secrets  []string
	channels map[string]*strings.Builder
}

func (e *jsonEditor) space() {
	for e.i < len(e.p) && e.p[e.i] <= ' ' {
		e.i++
	}
}
func (e *jsonEditor) str() string {
	start := e.i
	e.i++
	for e.p[e.i] != '"' {
		if e.p[e.i] == '\\' {
			e.i++
		}
		e.i++
	}
	e.i++
	var s string
	_ = json.Unmarshal(e.p[start:e.i], &s)
	return s
}
func (e *jsonEditor) replace(start int, v string) {
	e.out.Write(e.p[e.last:start])
	raw, _ := json.Marshal(v)
	e.out.Write(raw)
	// Preserve data-line boundaries when a masked object spans SSE lines.
	for _, c := range e.p[start:e.i] {
		if c == '\n' {
			e.out.WriteByte('\n')
		}
	}
	e.last = e.i
}
func (e *jsonEditor) value(edit, hide bool, key string) {
	e.space()
	start := e.i
	inside := edit && !hide
	switch e.p[e.i] {
	case '{':
		e.i++
		e.space()
		for e.p[e.i] != '}' {
			kStart := e.i
			k := e.str()
			if inside {
				if v := scrub(k, e.secrets); v != k {
					e.replace(kStart, v)
				}
			}
			e.space()
			e.i++
			e.value(inside, keySensitive(k), k)
			e.space()
			if e.p[e.i] != ',' {
				break
			}
			e.i++
			e.space()
		}
		e.i++
	case '[':
		e.i++
		e.space()
		for e.p[e.i] != ']' {
			e.value(inside, false, key)
			e.space()
			if e.p[e.i] != ',' {
				break
			}
			e.i++
			e.space()
		}
		e.i++
	case '"':
		v := e.str()
		if inside {
			safe := scrub(v, e.secrets)
			if safe != v {
				e.replace(start, safe)
			}
			if e.channels != nil {
				switch key {
				case "content", "text", "delta", "thinking", "arguments", "partial_json", "output_text", "reasoning_content":
					b := e.channels[key]
					if b == nil {
						b = new(strings.Builder)
						e.channels[key] = b
					}
					b.WriteString(safe)
				}
			}
		}
	default:
		for e.i < len(e.p) && !strings.ContainsRune(" \r\n\t,]}", rune(e.p[e.i])) {
			e.i++
		}
		if inside && scrub(string(e.p[start:e.i]), e.secrets) != string(e.p[start:e.i]) {
			e.replace(start, mask)
		}
	}
	if edit && hide {
		e.replace(start, mask)
	}
}
func redactJSON(p []byte, secrets []string) []byte {
	return editJSON(p, secrets, nil)
}

func editJSON(p []byte, secrets []string, channels map[string]*strings.Builder) []byte {
	e := jsonEditor{p: p, secrets: secrets, channels: channels}
	e.value(true, false, "")
	if e.last == 0 {
		return p
	}
	e.out.Write(p[e.last:])
	return e.out.Bytes()
}

// SanitizeBody applies the current credential policy to both new captures
// and historical evidence being prepared for sharing.
func SanitizeBody(p []byte, contentType string, secrets []string) ([]byte, bool) {
	ct := strings.ToLower(contentType)
	if strings.HasPrefix(ct, "text/event-stream") || bytes.HasPrefix(bytes.TrimSpace(p), []byte("data:")) || bytes.Contains(p, []byte("\ndata:")) || bytes.Contains(p, []byte("\rdata:")) {
		return redactSSE(p, secrets)
	}
	return payload(p, secrets)
}

func payload(p []byte, secrets []string) ([]byte, bool) {
	if !utf8.Valid(p) {
		return nil, false
	}
	if json.Valid(p) {
		return redactJSON(p, secrets), true
	}
	t := bytes.TrimSpace(p)
	if len(t) > 0 && (t[0] == '{' || t[0] == '[' || t[0] == '"') && !bytes.Equal(t, []byte("[DONE]")) {
		return nil, false
	}
	return []byte(scrub(string(p), secrets)), true
}

// Multiple data fields in one SSE event are one payload joined by LF. Walk
// each event twice to preserve framing without allocating a slice per line.
func redactSSE(p []byte, secrets []string) ([]byte, bool) {
	if !utf8.Valid(p) {
		return nil, false
	}
	var out, joined bytes.Buffer
	channels := map[string]*strings.Builder{}
	for len(p) > 0 {
		event := p
		joined.Reset()
		count := 0
		for len(p) > 0 {
			line, _, rest := sseLine(p)
			p = rest
			if len(line) == 0 {
				break
			}
			if k := dataStart(line); k >= 0 {
				if count > 0 {
					joined.WriteByte('\n')
				}
				joined.Write(line[k:])
				count++
			}
		}
		event = event[:len(event)-len(p)]
		var safe []byte
		if count > 0 {
			var ok bool
			if p := joined.Bytes(); utf8.Valid(p) && json.Valid(p) {
				safe, ok = editJSON(p, secrets, channels), true
			} else {
				safe, ok = payload(joined.Bytes(), secrets)
			}
			if !ok || bytes.Count(safe, []byte("\n")) != count-1 {
				return nil, false
			}
		}
		for len(event) > 0 {
			line, end, rest := sseLine(event)
			event = rest
			if k := dataStart(line); k >= 0 {
				part, next, _ := bytes.Cut(safe, []byte("\n"))
				safe = next
				out.Write(line[:k])
				out.Write(part)
			} else {
				// A private-key block in SSE comments or extension fields can
				// span multiple lines outside data. Do not capture its fragments.
				if bytes.Contains(line, []byte("-----BEGIN ")) {
					return nil, false
				}
				out.WriteString(scrub(string(line), secrets))
			}
			out.Write(end)
			if out.Len() > maxBody {
				return nil, false
			}
		}
	}
	// A provider can split a credential across valid JSON delta frames.
	// No single frame was unsafe, but their concatenated text may be. Do
	// not retain reconstructible fragments when that happens.
	for _, b := range channels {
		if v := b.String(); scrub(v, secrets) != v {
			return nil, false
		}
	}
	return out.Bytes(), true
}
func dataStart(line []byte) int {
	if bytes.Equal(line, []byte("data")) {
		return 4
	}
	if !bytes.HasPrefix(line, []byte("data:")) {
		return -1
	}
	if len(line) > 5 && line[5] == ' ' {
		return 6
	}
	return 5
}
func sseLine(p []byte) (line, end, rest []byte) {
	i := bytes.IndexAny(p, "\r\n")
	if i < 0 {
		return p, nil, nil
	}
	n := 1
	if p[i] == '\r' && i+1 < len(p) && p[i+1] == '\n' {
		n++
	}
	return p[:i], p[i : i+n], p[i+n:]
}
