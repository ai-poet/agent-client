// Package cheapauth presents the optional CheapRouter sign-in and account
// state on its settings page. Network and credential mutations stay in the
// live Runtime's background workers.
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
	AccountFailure         string
	MigrationFailure       string
	MigrationFailureDetail string
	Account                core.User
	ModelCount             int

	Start          func(manual bool)
	SubmitPaste    func()
	Cancel         func()
	StopSync       func()
	Retry          func()
	RetryMigration func()
	Logout         func()
}

func NewState() *State { return &State{Endpoint: core.PrimaryEndpoint} }

// SignedIn reports whether credentials are established, whatever the gateway's state.
func (s *State) SignedIn() bool {
	return s != nil && (s.Stage == SyncingGateway || s.Stage == Ready || s.Stage == GatewayFailed)
}
func (s *State) Busy() bool {
	return s != nil && (s.Stage == Starting || s.Stage == SyncingGateway)
}
