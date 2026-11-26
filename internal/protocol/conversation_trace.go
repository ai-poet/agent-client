package protocol

import "encoding/json"

// After is the opaque cursor of the preceding page. A body read instead
// names a sidecar from a returned record, relative to this session alone.
type TraceQuery struct {
	Session      SessionID `json:"session"`
	After        string    `json:"after,omitempty"`
	Limit        uint16    `json:"limit,omitempty"`
	Body         string    `json:"body,omitempty"`
	PreviewBytes uint32    `json:"preview_bytes,omitempty"`
	StatusOnly   bool      `json:"status_only,omitempty"`
}

const TracePreviewLimit = 2 << 20

type TraceStatus struct {
	Recording bool `json:"recording"`
	Failed    int  `json:"failed"`
}

type TracePage struct {
	Session   SessionID         `json:"session"`
	Records   []json.RawMessage `json:"records,omitempty"`
	Next      string            `json:"next,omitempty"`
	Tail      string            `json:"tail,omitempty"` // resume after the last record when a watch changes
	Body      string            `json:"body,omitempty"`
	Content   []byte            `json:"content,omitempty"`
	Truncated bool              `json:"truncated,omitempty"`
	Status    *TraceStatus      `json:"status,omitempty"`
}
