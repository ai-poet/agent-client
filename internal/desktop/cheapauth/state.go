// Package cheapauth presents the optional CheapRouter sign-in and account
// state on its settings page. Network and credential mutations stay in the
// live Runtime's background workers.
package cheapauth

import (
	"fmt"
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/egoist/mygo/ui"
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
	AffectedRoles          int
	MigratingRoles         bool
	MigrateRoles           func()
	Recharge               *RechargeDialog
	OpenRecharge           func()
	OpenPlans              func()

	Groups             []core.Group
	Catalog            []core.Model
	GroupsLoading      bool
	GroupsError        string
	GroupStatuses      map[int64]core.GroupStatus
	GroupStatusLoading bool
	GroupStatusLoaded  bool
	GroupStatusError   string
	SelectedGroupID    *int64
	LaneBindings       map[string]int64
	AutoBindFailures   map[string]string
	SwitchingGroup     bool
	SwitchingLane      string
	GroupError         string

	OriginConfig    core.GatewayOriginConfig
	OriginResults   []core.CandidateResult
	OriginTesting   bool
	OriginSwitching bool
	OriginInput     string
	OriginError     string

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
	SelectOrigin    func(origin string)
	AddOrigin       func(raw string)
	RemoveOrigin    func(origin string)
	TestOrigins     func()
	SetOriginAuto   func(on bool)
}

func NewState() *State {
	return &State{
		Endpoint:     core.PrimaryEndpoint,
		OriginConfig: core.DefaultGatewayOriginConfig(),
	}
}

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

// GroupsForLane returns the routing groups that belong to lane, stably ordered
// by subscription/standard/welfare tier.
func (s *State) GroupsForLane(lane string) []core.Group {
	if s == nil {
		return nil
	}
	return core.SortGroups(core.GroupsForLane(lane, s.Groups, s.Catalog))
}

// GroupSectionsForLane groups lane's routing groups into ordered sections
// (订阅 -> 常驻 -> 福利).
func (s *State) GroupSectionsForLane(lane string) []core.GroupSection {
	if s == nil {
		return nil
	}
	return core.GroupSections(core.GroupsForLane(lane, s.Groups, s.Catalog))
}

// GroupStatusFailed reports whether the latest /group-status fetch failed.
func (s *State) GroupStatusFailed() bool {
	return s != nil && strings.TrimSpace(s.GroupStatusError) != ""
}

// GroupStatusFor looks up the latest runtime status for groupID.
func (s *State) GroupStatusFor(groupID int64) *core.GroupStatus {
	if s == nil || groupID <= 0 || len(s.GroupStatuses) == 0 {
		return nil
	}
	if status, ok := s.GroupStatuses[groupID]; ok {
		copied := status
		return &copied
	}
	return nil
}

// DescribeGroup returns the canonical presentation view (tags, rate, online
// rate / degraded status) for g.
func (s *State) DescribeGroup(g core.Group) core.GroupPresentation {
	return core.DescribeGroup(g, s.GroupStatusFor(g.ID), s.GroupStatusFailed())
}

// BoundGroup returns the group currently bound to lane, or false when unbound.
func (s *State) BoundGroup(lane string) (core.Group, bool) {
	id := s.BoundGroupID(lane)
	if id == nil {
		return core.Group{}, false
	}
	for _, g := range s.Groups {
		if g.ID == *id {
			return g, true
		}
	}
	return core.Group{ID: *id, Name: fmt.Sprintf("#%d", *id)}, true
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

// ActiveOrigin returns the business API origin currently routed to: the
// explicitly selected gateway node when valid, otherwise the login endpoint.
func (s *State) ActiveOrigin() string {
	if s == nil {
		return core.PrimaryEndpoint
	}
	if strings.TrimSpace(s.Endpoint) != "" {
		return s.OriginConfig.ResolveAPIOrigin(s.Endpoint)
	}
	return s.OriginConfig.Origin()
}

// OriginCandidates returns candidate origins ordered by the latest speed test
// results (available fastest first, then failed, then untested).
func (s *State) OriginCandidates() []string {
	if s == nil {
		return core.DefaultGatewayOriginConfig().EffectiveCandidates()
	}
	return core.SortOriginsByResults(s.OriginConfig.EffectiveCandidates(), s.OriginResults)
}

// OriginResultFor returns the latest speed test outcome for origin, if any.
func (s *State) OriginResultFor(origin string) *core.CandidateResult {
	if s == nil {
		return nil
	}
	for i := range s.OriginResults {
		if s.OriginResults[i].URL == origin {
			copied := s.OriginResults[i]
			return &copied
		}
	}
	return nil
}

// OriginDisplayLabel returns the localized display name and optional secondary
// description for a candidate origin.
func (s *State) OriginDisplayLabel(c *ui.Context, origin string) (name, detail string) {
	if s == nil {
		return shortOriginHost(origin), ""
	}
	label := s.OriginConfig.NodeLabel(origin)
	switch label.Kind {
	case core.NodeLabelPrimary:
		return i18n.T(c, "gateway_node_primary"), ""
	case core.NodeLabelServer:
		name = strings.TrimSpace(label.Name)
		if name == "" {
			name = shortOriginHost(origin)
		}
		return name, strings.TrimSpace(label.Description)
	case core.NodeLabelBuiltInCDN:
		return i18n.T(c, "gateway_node_cdn"), i18n.T(c, "gateway_node_cdn_detail")
	default:
		if s.OriginConfig.IsRemovable(origin) {
			return i18n.T(c, "gateway_node_custom"), ""
		}
		return shortOriginHost(origin), ""
	}
}

// ActiveOriginShortLabel returns a compact label for the active node suitable
// for popover menus and summary rows.
func (s *State) ActiveOriginShortLabel(c *ui.Context) string {
	origin := s.ActiveOrigin()
	if origin == "" {
		return ""
	}
	label := s.OriginConfig.NodeLabel(origin)
	switch label.Kind {
	case core.NodeLabelPrimary:
		return i18n.T(c, "gateway_node_primary")
	case core.NodeLabelServer:
		if name := strings.TrimSpace(label.Name); name != "" {
			return name
		}
	case core.NodeLabelBuiltInCDN:
		return i18n.T(c, "gateway_node_cdn")
	}
	return shortOriginHost(origin)
}

func shortOriginHost(origin string) string {
	if _, rest, ok := strings.Cut(origin, "://"); ok && rest != "" {
		return rest
	}
	return origin
}
