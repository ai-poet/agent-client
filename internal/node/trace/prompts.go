package trace

import (
	"bytes"
	"encoding/json"
	"fmt"
	"unicode/utf8"
)

// Only emit aggregate fingerprints, even if prompts also occur in a tool result
// or an unfamiliar provider extension. Large/unknown payloads retain the bounded
// whole-body fingerprint without building an arbitrary message tree.
func promptMarkers(body []byte) []byte {
	if len(body) > 1<<20 {
		return nil
	}
	type message struct {
		Role    string
		Content json.RawMessage
	}
	var envelope struct {
		System, Instructions, Tools, Functions json.RawMessage
		Messages                               []message
		Input                                  json.RawMessage
	}
	if err := json.Unmarshal(body, &envelope); err != nil {
		return nil
	}
	var out bytes.Buffer
	add := func(kind string, raw json.RawMessage) {
		if len(raw) == 0 || out.Len() > 4096 {
			return
		}
		var s string
		if raw[0] == '"' && json.Unmarshal(raw, &s) == nil {
			raw = []byte(s)
		}
		fmt.Fprintf(&out, "\n<%s: %d chars, sha256:%s>", kind, utf8.RuneCount(raw), digest(raw))
	}
	add("system prompt", envelope.System)
	add("system prompt", envelope.Instructions)
	add("tool schemas", envelope.Tools)
	add("tool schemas", envelope.Functions)
	messages := func(ms []message) {
		for _, m := range ms {
			if m.Role == "developer" || m.Role == "system" {
				add("system prompt", m.Content)
			}
		}
	}
	messages(envelope.Messages)
	if len(envelope.Input) > 0 && envelope.Input[0] == '[' {
		var ms []message
		if json.Unmarshal(envelope.Input, &ms) == nil {
			messages(ms)
		}
	}
	return out.Bytes()
}
