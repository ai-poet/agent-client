// Package cheapauth presents CheapRouter sign-in and account state. Network
// and credential mutations stay in the live Runtime's background workers.
package cheapauth

import core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"

type Stage uint8

const (
	SignedOut Stage = iota
	Starting
	WaitingForCode
	SyncingGateway
	Ready
	GatewayFailed
)

// State is UI-thread-only presentation state. It never stores OAuth tokens,
// the PKCE verifier, or a gateway API key.
type State struct {
	Stage          Stage
	Endpoint       string
	LoginURL       string
	Loopback       bool
	Exchanging     bool
	LoggingOut     bool
	Pasted         string
	Failure        string
	AccountFailure string
	Account        core.User
	ModelCount     int

	Start       func(manual bool)
	SubmitPaste func()
	Cancel      func()
	StopSync    func()
	Retry       func()
	Logout      func()
}

func NewState() *State { return &State{Endpoint: core.PrimaryEndpoint} }

func (s *State) RequiresLogin() bool {
	return s == nil || s.Stage == SignedOut || s.Stage == Starting || s.Stage == WaitingForCode
}
func (s *State) Busy() bool {
	return s != nil && (s.Stage == Starting || s.Stage == SyncingGateway)
}
