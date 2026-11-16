// Package daemon implements the existing client-daemon wire protocol and
// process boundary. It deliberately contains no UI code.
package daemon

import "fmt"

const (
	// ProtocolVersion must match crates/client-protocol's PROTOCOL_VERSION.
	ProtocolVersion = 6
	// MaxWireMessageBytes must match the Rust client's WebSocket read limit.
	MaxWireMessageBytes = 48 * 1024 * 1024

	DaemonAddressEnv = "WAKU_DAEMON_ADDRESS"
	DaemonTokenEnv   = "WAKU_DAEMON_TOKEN"
	DaemonPathEnv    = "WAKU_DAEMON_PATH"
	AppExecutableEnv = "WAKU_APP_EXECUTABLE"
)

// ProtocolError reports a malformed or incompatible daemon handshake.
type ProtocolError struct {
	Message string
}

func (e *ProtocolError) Error() string { return e.Message }

func protocolErrorf(format string, args ...any) error {
	return &ProtocolError{Message: fmt.Sprintf(format, args...)}
}

// ReplayCursor is included so later phases can resume sequenced event streams
// without changing the hello message. The shell currently sends an empty list.
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
