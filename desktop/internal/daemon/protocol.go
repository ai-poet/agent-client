// Package daemon implements the existing client-daemon wire protocol and
// process boundary. It deliberately contains no UI code.
package daemon

import (
	"encoding/json"
	"fmt"
)

const (
	// ProtocolVersion must match crates/client-protocol's PROTOCOL_VERSION.
	ProtocolVersion = 6
	// MaxWireMessageBytes must match the Rust client's WebSocket read limit.
	MaxWireMessageBytes = 48 * 1024 * 1024

	DaemonAddressEnv = "WAKU_DAEMON_ADDRESS"
	DaemonTokenEnv   = "WAKU_DAEMON_TOKEN"
	DaemonPathEnv    = "WAKU_DAEMON_PATH"
	AppExecutableEnv = "WAKU_APP_EXECUTABLE"

	// NilUUID is used by protocol-wide commands and by an unattached runtime.
	NilUUID = "00000000-0000-0000-0000-000000000000"
)

// ProtocolError reports a malformed or incompatible daemon handshake.
type ProtocolError struct {
	Message string
}

func (e *ProtocolError) Error() string { return e.Message }

func protocolErrorf(format string, args ...any) error {
	return &ProtocolError{Message: fmt.Sprintf(format, args...)}
}

// RPCError is the error outcome returned by client-daemon for a request.
type RPCError struct {
	Message string `json:"message"`
}

func (e *RPCError) Error() string { return e.Message }

// RuntimeEventCursor is the last event incorporated into a hydrated session.
type RuntimeEventCursor struct {
	RuntimeID string `json:"runtime_id"`
	Epoch     string `json:"epoch"`
	Sequence  uint64 `json:"sequence"`
}

// ReplayCursor is the last sequence consumed for one sequenced event stream.
// A reconnect sends all known cursors in hello.resumeFrom.
type ReplayCursor struct {
	SessionID string `json:"sessionId"`
	RuntimeID string `json:"runtimeId"`
	Epoch     string `json:"epoch"`
	Sequence  uint64 `json:"sequence"`
}

type helloMessage struct {
	Type            string         `json:"type"`
	ProtocolVersion uint32         `json:"protocolVersion"`
	Token           string         `json:"token"`
	ClientID        string         `json:"clientId"`
	ResumeFrom      []ReplayCursor `json:"resumeFrom"`
}

type serverHandshake struct {
	Type            string `json:"type"`
	ProtocolVersion uint32 `json:"protocolVersion"`
	DaemonVersion   string `json:"daemonVersion"`
	Message         string `json:"message"`
}

// WireDriverEvent preserves the daemon's kind/payload representation. Its
// payload is decoded by the session projection according to kind.
type WireDriverEvent struct {
	Kind    string          `json:"kind"`
	Payload json.RawMessage `json:"payload"`
}

// SequencedEvent is flattened into ServerMessage's event variant on the wire.
type SequencedEvent struct {
	SessionID string          `json:"sessionId"`
	RuntimeID string          `json:"runtimeId"`
	Epoch     string          `json:"epoch"`
	Sequence  uint64          `json:"sequence"`
	Event     WireDriverEvent `json:"event"`
}

func (e SequencedEvent) cursor() ReplayCursor {
	return ReplayCursor{
		SessionID: e.SessionID,
		RuntimeID: e.RuntimeID,
		Epoch:     e.Epoch,
		Sequence:  e.Sequence,
	}
}

// ResponsePayload contains the response variants used by the workbench. The
// Type discriminator remains available so callers can reject an unexpected
// successful response instead of silently accepting a zero value.
type ResponsePayload struct {
	Type            string         `json:"type"`
	Projects        []Project      `json:"projects"`
	Sessions        []AgentSession `json:"sessions"`
	DefaultCwd      string         `json:"defaultCwd"`
	ProjectlessRoot *string        `json:"projectlessRoot"`
	Session         *AgentSession  `json:"session"`
	RuntimeID       *string        `json:"runtimeId"`
	SupportsSteer   bool           `json:"supportsSteer"`
}

// Project mirrors client-protocol's Project. Its fields intentionally use
// snake_case because that Rust struct does not apply camelCase renaming.
type Project struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Path      string `json:"path"`
	CreatedAt uint64 `json:"created_at"`
}

// AgentSession is the persisted task projection used by list and hydrate.
// Detail-only fields are absent from loadTaskState's lightweight sessions.
type AgentSession struct {
	ID                 string              `json:"id"`
	Title              string              `json:"title"`
	AutoTitle          *string             `json:"auto_title"`
	ProjectID          string              `json:"project_id"`
	Provider           string              `json:"provider"`
	Model              *string             `json:"model"`
	Status             string              `json:"status"`
	CreatedAt          uint64              `json:"created_at"`
	UpdatedAt          uint64              `json:"updated_at"`
	LastReplyAt        *uint64             `json:"last_reply_at"`
	RuntimeEventCursor *RuntimeEventCursor `json:"runtime_event_cursor"`
	Messages           []Message           `json:"messages"`
	TranscriptBlocks   []TranscriptBlock   `json:"transcript_blocks"`
}

// Message mirrors client-protocol's persisted Message, including its
// snake_case fields and the user-visible display_content override.
type Message struct {
	ID             string  `json:"id"`
	TurnID         *string `json:"turn_id"`
	Role           string  `json:"role"`
	Content        string  `json:"content"`
	DisplayContent *string `json:"display_content"`
	CreatedAt      uint64  `json:"created_at"`
	Streaming      bool    `json:"streaming"`
}

func (m Message) VisibleContent() string {
	if m.DisplayContent != nil {
		return *m.DisplayContent
	}
	return m.Content
}

// TranscriptBlock stores ordered non-message work after a message index.
type TranscriptBlock struct {
	AfterMessage int               `json:"after_message"`
	TurnID       *string           `json:"turn_id"`
	Content      TranscriptContent `json:"content"`
}

type TranscriptContent struct {
	Kind string          `json:"kind"`
	Data json.RawMessage `json:"data"`
}

// ActivityItem is the provider-neutral work record embedded in transcript
// blocks and richActivity events.
type ActivityItem struct {
	ID          string               `json:"id"`
	SourceID    *string              `json:"source_id"`
	Kind        string               `json:"kind"`
	Title       string               `json:"title"`
	Detail      *string              `json:"detail"`
	Arguments   *string              `json:"arguments"`
	Output      *string              `json:"output"`
	Failed      bool                 `json:"failed"`
	Complete    bool                 `json:"complete"`
	FileChanges []ActivityFileChange `json:"file_changes"`
}

type ActivityFileChange struct {
	Path      string  `json:"path"`
	Additions *uint64 `json:"additions"`
	Deletions *uint64 `json:"deletions"`
	Status    *string `json:"status"`
	Diff      *string `json:"diff"`
}
