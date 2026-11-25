package protocol

import "encoding/json"

// After is the opaque cursor of the preceding page. A body read instead
// names a sidecar from a returned record, relative to this session alone.
type TraceQuery struct {
	Session SessionID `json:"session"`
	After   string    `json:"after,omitempty"`
	Limit   uint16    `json:"limit,omitempty"`
	Body    string    `json:"body,omitempty"`
}

type TracePage struct {
	Session   SessionID         `json:"session"`
	Records   []json.RawMessage `json:"records,omitempty"`
	Next      string            `json:"next,omitempty"`
	Body      string            `json:"body,omitempty"`
	Content   []byte            `json:"content,omitempty"`
	Truncated bool              `json:"truncated,omitempty"`
}
