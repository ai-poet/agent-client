package cheapauth

import (
	"fmt"
	"math"
	"slices"
	"strings"
)

// GroupTier orders and groups routing groups by category (welfare vs standard)
// and subscription_type (subscription vs pay-as-you-go).
type GroupTier uint8

const (
	GroupTierSubscription GroupTier = iota
	GroupTierStandard
	GroupTierWelfare
)

// Title returns the canonical section heading for the tier.
func (t GroupTier) Title() string {
	switch t {
	case GroupTierSubscription:
		return "订阅"
	case GroupTierWelfare:
		return "福利"
	default:
		return "常驻"
	}
}

// Subtitle returns the optional secondary hint shown on section headers.
func (t GroupTier) Subtitle() string {
	if t == GroupTierWelfare {
		return "限时 · 波动"
	}
	return ""
}

// GroupTagKind classifies a group's display badge for consistent color/icon
// mapping across the lane panel, model selector, and account page.
type GroupTagKind uint8

const (
	GroupTagStandard GroupTagKind = iota
	GroupTagSubscription
	GroupTagWelfare
)

// GroupTag is one badge shown for a routing group.
type GroupTag struct {
	Label string
	Kind  GroupTagKind
}

// GroupSection holds groups belonging to one tier in presentation order.
type GroupSection struct {
	Tier   GroupTier
	Groups []Group
}

// GroupHealth is the normalized availability health state of a group.
type GroupHealth uint8

const (
	GroupHealthNone GroupHealth = iota
	GroupHealthUp
	GroupHealthDegraded
	GroupHealthDown
	GroupHealthUnknown
)

// GroupStatusView is the shared status projection for a group.
type GroupStatusView struct {
	Failed      bool
	OnlineRate  string // "" when Failed; "—" when absent; else formatted percentage
	StatusLabel string // "状态未知", "正常", "降级", "异常", or ""
	Health      GroupHealth
}

// ShortText returns the compact status string shown beside a group or model:
// "状态未知" when status loading failed (online rate hidden), "99.85%" or "—"
// when healthy/unreported, and "96.2% · 降级" / "0.0% · 异常" when degraded/down.
func (v GroupStatusView) ShortText() string {
	if v.Failed {
		return "状态未知"
	}
	rate := v.OnlineRate
	if rate == "" {
		rate = "—"
	}
	switch v.Health {
	case GroupHealthDegraded, GroupHealthDown:
		if rate == "—" {
			return v.StatusLabel
		}
		return rate + " · " + v.StatusLabel
	default:
		return rate
	}
}

// GroupPresentation is the single source of truth for displaying a group's
// category/subscription tags, rate multiplier, and online rate across all UI
// surfaces.
type GroupPresentation struct {
	Group  Group
	Tier   GroupTier
	Tags   []GroupTag
	Rate   string
	Status GroupStatusView
}

// DisplayName returns the group's trimmed name or "#id" fallback.
func (p GroupPresentation) DisplayName() string {
	if name := strings.TrimSpace(p.Group.Name); name != "" {
		return name
	}
	if p.Group.ID > 0 {
		return fmt.Sprintf("#%d", p.Group.ID)
	}
	return ""
}

// TagLabels returns the plain text labels of all tags.
func (p GroupPresentation) TagLabels() []string {
	if len(p.Tags) == 0 {
		return nil
	}
	out := make([]string, len(p.Tags))
	for i, tag := range p.Tags {
		out[i] = tag.Label
	}
	return out
}

// CompactSummary joins group name, tags, rate (when present), and status text
// for tooltips, accessibility descriptions, and single-line summaries.
func (p GroupPresentation) CompactSummary() string {
	var parts []string
	if name := p.DisplayName(); name != "" {
		parts = append(parts, name)
	}
	parts = append(parts, p.TagLabels()...)
	if p.Rate != "" {
		parts = append(parts, p.Rate)
	}
	if status := p.Status.ShortText(); status != "" {
		parts = append(parts, status)
	}
	return strings.Join(parts, " · ")
}

// TierOfGroup classifies g into subscription, standard, or welfare.
// Welfare takes precedence so limited-time promo groups are never mixed into
// permanent subscription or standard sections.
func TierOfGroup(g Group) GroupTier {
	if g.IsWelfare() {
		return GroupTierWelfare
	}
	if g.IsSubscription() {
		return GroupTierSubscription
	}
	return GroupTierStandard
}

// GroupTags returns the canonical display tags for g.
func GroupTags(g Group) []GroupTag {
	if g.IsWelfare() {
		tags := []GroupTag{{Label: "福利", Kind: GroupTagWelfare}}
		if g.IsSubscription() {
			tags = append(tags, GroupTag{Label: "订阅", Kind: GroupTagSubscription})
		}
		return tags
	}
	if g.IsSubscription() {
		return []GroupTag{{Label: "订阅", Kind: GroupTagSubscription}}
	}
	return []GroupTag{{Label: "常驻", Kind: GroupTagStandard}}
}

// GroupRateLabel formats the pay-as-you-go multiplier ("×0.50") when applicable.
func GroupRateLabel(g Group) string {
	if g.IsSubscription() || g.RateMultiplier <= 0 || math.IsNaN(g.RateMultiplier) || math.IsInf(g.RateMultiplier, 0) {
		return ""
	}
	return fmt.Sprintf("×%.2f", g.RateMultiplier)
}

// SortGroups returns a copy of groups stably ordered by tier:
// 订阅 -> 常驻 -> 福利 (and within 福利, subscription before pay-as-you-go),
// preserving server order within each bucket.
func SortGroups(groups []Group) []Group {
	if len(groups) == 0 {
		return nil
	}
	out := slices.Clone(groups)
	slices.SortStableFunc(out, func(a, b Group) int {
		ta, tb := TierOfGroup(a), TierOfGroup(b)
		if ta != tb {
			return int(ta) - int(tb)
		}
		if a.IsSubscription() != b.IsSubscription() {
			if a.IsSubscription() {
				return -1
			}
			return 1
		}
		return 0
	})
	return out
}

// GroupSections splits groups into non-empty ordered sections (订阅 / 常驻 / 福利).
func GroupSections(groups []Group) []GroupSection {
	sorted := SortGroups(groups)
	if len(sorted) == 0 {
		return nil
	}
	order := []GroupTier{GroupTierSubscription, GroupTierStandard, GroupTierWelfare}
	buckets := make(map[GroupTier][]Group, 3)
	for _, g := range sorted {
		tier := TierOfGroup(g)
		buckets[tier] = append(buckets[tier], g)
	}
	var sections []GroupSection
	for _, tier := range order {
		if items := buckets[tier]; len(items) > 0 {
			sections = append(sections, GroupSection{Tier: tier, Groups: items})
		}
	}
	return sections
}

// FormatAvailability formats a 0-100 availability percentage matching Rust
// sub2api/model_plaza (two decimals at >= 99%, one decimal below), and returns
// "—" instead of 0% whenever data is absent or non-finite.
func FormatAvailability(value *float64) string {
	if value == nil || math.IsNaN(*value) || math.IsInf(*value, 0) {
		return "—"
	}
	if *value >= 99.0 {
		return fmt.Sprintf("%.2f%%", *value)
	}
	return fmt.Sprintf("%.1f%%", *value)
}

// ResolveGroupStatus builds the display status for one group. When statusFailed
// is true, online rate is hidden and the status degrades to "状态未知".
func ResolveGroupStatus(status *GroupStatus, statusFailed bool) GroupStatusView {
	if statusFailed {
		return GroupStatusView{
			Failed:      true,
			OnlineRate:  "",
			StatusLabel: "状态未知",
			Health:      GroupHealthUnknown,
		}
	}
	if status == nil {
		return GroupStatusView{
			OnlineRate: "—",
			Health:     GroupHealthNone,
		}
	}
	rate := FormatAvailability(status.Availability24h)
	switch status.EffectiveStatus() {
	case "up":
		return GroupStatusView{
			OnlineRate:  rate,
			StatusLabel: "正常",
			Health:      GroupHealthUp,
		}
	case "degraded":
		return GroupStatusView{
			OnlineRate:  rate,
			StatusLabel: "降级",
			Health:      GroupHealthDegraded,
		}
	case "down":
		return GroupStatusView{
			OnlineRate:  rate,
			StatusLabel: "异常",
			Health:      GroupHealthDown,
		}
	default:
		return GroupStatusView{
			OnlineRate: rate,
			Health:     GroupHealthNone,
		}
	}
}

// DescribeGroup produces the canonical presentation view for g.
func DescribeGroup(g Group, status *GroupStatus, statusFailed bool) GroupPresentation {
	return GroupPresentation{
		Group:  g,
		Tier:   TierOfGroup(g),
		Tags:   GroupTags(g),
		Rate:   GroupRateLabel(g),
		Status: ResolveGroupStatus(status, statusFailed),
	}
}

// IndexGroupStatuses maps a /group-status slice by GroupID for O(1) lookup.
func IndexGroupStatuses(items []GroupStatus) map[int64]GroupStatus {
	if len(items) == 0 {
		return nil
	}
	out := make(map[int64]GroupStatus, len(items))
	for _, item := range items {
		if item.GroupID > 0 {
			out[item.GroupID] = item
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}
