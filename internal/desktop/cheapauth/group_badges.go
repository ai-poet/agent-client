package cheapauth

import (
	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// TagSpec maps a canonical group tag kind onto its semantic palette tone and
// icon, shared by the lane panel, model selector hints, and account page.
func TagSpec(kind core.GroupTagKind) (kit.Tone, *ui.SVG) {
	switch kind {
	case core.GroupTagSubscription:
		return kit.Info, icons.Crown
	case core.GroupTagWelfare:
		return kit.Caution, icons.Zap
	default:
		return kit.Neutral, icons.Shield
	}
}

// HealthSpec maps a normalized group health state onto its semantic palette
// tone and icon, pairing color with an icon and text for accessibility.
func HealthSpec(health core.GroupHealth) (kit.Tone, *ui.SVG) {
	switch health {
	case core.GroupHealthUp:
		return kit.Positive, icons.CircleCheck
	case core.GroupHealthDegraded:
		return kit.Caution, icons.TriangleAlert
	case core.GroupHealthDown:
		return kit.Negative, icons.CircleX
	case core.GroupHealthUnknown:
		return kit.Neutral, icons.CircleQuestion
	default:
		return kit.Neutral, nil
	}
}

// GroupTagPill renders one canonical group badge ("订阅", "常驻", or "福利").
func GroupTagPill(c *ui.Context, tag core.GroupTag, compact bool) ui.Element {
	tone, icon := TagSpec(tag.Kind)
	return kit.StatusPill(c, tag.Label, icon, tone, compact)
}

// GroupTagsRow renders all canonical badges for a group inline.
func GroupTagsRow(c *ui.Context, tags []core.GroupTag, compact bool) {
	for _, tag := range tags {
		GroupTagPill(c, tag, compact)
	}
}

// GroupStatusBadge renders the online rate or degraded/unknown status readout.
func GroupStatusBadge(c *ui.Context, status core.GroupStatusView, compact bool) ui.Element {
	tone, icon := HealthSpec(status.Health)
	return kit.StatusIndicator(c, status.ShortText(), icon, tone, compact)
}

// StatusUnknownBadge renders a compact "状态未知" degradation notice with an
// optional tooltip carrying the underlying fetch failure.
func StatusUnknownBadge(c *ui.Context, tip string, compact bool) ui.Element {
	el := kit.StatusPill(c, "状态未知", icons.CircleQuestion, kit.Neutral, compact)
	if tip != "" {
		kit.Tooltip(c, el, tip)
	}
	return el
}

// GroupSectionHeader draws a category/subscription tier heading inside a lane's
// group list ("订阅", "常驻", or "福利" with its "限时 · 波动" note).
func GroupSectionHeader(c *ui.Context, key string, tier core.GroupTier) {
	p := theme.Of(c)
	var icon *ui.SVG
	titleColor := p.MutedForeground
	switch tier {
	case core.GroupTierSubscription:
		icon = icons.Crown
		titleColor = p.Primary
	case core.GroupTierWelfare:
		icon = icons.Zap
		titleColor = p.Warning
	default:
		icon = icons.Shield
	}
	ui.Row(c.Key(key)).FillWidth().Padding(4, 6).Gap(5).AlignItems(ui.Center).Children(func() {
		if icon != nil {
			ui.Icon(c, icon).FontSize(11).Shrink(0).TextColor(titleColor)
		}
		ui.Text(c, tier.Title()).FontSize(11).FontWeight(600).SingleLine().Shrink(0).TextColor(titleColor)
		if sub := tier.Subtitle(); sub != "" {
			ui.Text(c, sub).FontSize(10).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
		}
	})
}
