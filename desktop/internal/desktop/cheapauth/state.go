// Package cheapauth presents the optional CheapRouter sign-in and account
// state on its settings page. Network and credential mutations stay in the
// live Runtime's background workers.
package cheapauth

import (
	"fmt"
	"strings"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
)

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
	Stage                  Stage
	Endpoint               string
	LoginURL               string
	Loopback               bool
	Exchanging             bool
	LoggingOut             bool
	Pasted                 string
	Failure                string
	AccountFailure         string
	MigrationFailure       string
	MigrationFailureDetail string
	Account                core.User
	ModelCount             int

	Groups           []core.Group
	Catalog          []core.Model
	GroupsLoading    bool
	GroupsError      string
	SelectedGroupID  *int64
	LaneBindings     map[string]int64
	AutoBindFailures map[string]string
	SwitchingGroup   bool
	SwitchingLane    string
	GroupError       string

	Start           func(manual bool)
	SubmitPaste     func()
	Cancel          func()
	StopSync        func()
	Retry           func()
	RetryMigration  func()
	Logout          func()
	RefreshGroups   func()
	SelectGroup     func(groupID *int64)
	SelectLaneGroup func(lane string, groupID *int64)
}

func NewState() *State { return &State{Endpoint: core.PrimaryEndpoint} }

// SignedIn reports whether credentials are established, whatever the gateway's state.
func (s *State) SignedIn() bool {
	return s != nil && (s.Stage == SyncingGateway || s.Stage == Ready || s.Stage == GatewayFailed)
}
func (s *State) Busy() bool {
	return s != nil && (s.Stage == Starting || s.Stage == SyncingGateway)
}

// Lanes returns the routing lanes available in the current groups and bindings.
func (s *State) Lanes() []string {
	if s == nil {
		return nil
	}
	return core.AvailableLanes(s.Groups, s.Catalog, s.LaneBindings)
}

// GroupsForLane returns the routing groups that belong to lane.
func (s *State) GroupsForLane(lane string) []core.Group {
	if s == nil {
		return nil
	}
	return core.GroupsForLane(lane, s.Groups, s.Catalog)
}

// BoundGroupID returns the bound group ID for lane, or nil when unbound.
func (s *State) BoundGroupID(lane string) *int64 {
	if s == nil || len(s.LaneBindings) == 0 {
		return nil
	}
	key := core.NormalizeLane(lane)
	if key == "" {
		key = strings.ToLower(strings.TrimSpace(lane))
	}
	if id, ok := s.LaneBindings[key]; ok && id > 0 {
		copied := id
		return &copied
	}
	return nil
}

// BoundGroupName returns the display name of the group bound to lane, or ""
// when lane is unbound.
func (s *State) BoundGroupName(lane string) string {
	id := s.BoundGroupID(lane)
	if id == nil {
		return ""
	}
	for _, g := range s.Groups {
		if g.ID == *id && strings.TrimSpace(g.Name) != "" {
			return g.Name
		}
	}
	return fmt.Sprintf("#%d", *id)
}

// ActiveGroupName returns a concise summary of bound groups for the sidebar
// footer: the group name when one lane is bound, or a count when multiple
// lanes are bound.
func (s *State) ActiveGroupName() string {
	if s == nil {
		return ""
	}
	if len(s.LaneBindings) > 0 {
		var names []string
		for _, lane := range s.Lanes() {
			if name := s.BoundGroupName(lane); name != "" {
				names = append(names, name)
			}
		}
		if len(names) == 1 {
			return names[0]
		}
		if len(names) > 1 {
			return fmt.Sprintf("已绑定 %d 个分组", len(names))
		}
	}
	if s.SelectedGroupID == nil {
		return ""
	}
	for _, g := range s.Groups {
		if g.ID == *s.SelectedGroupID && g.Name != "" {
			return g.Name
		}
	}
	return ""
}
