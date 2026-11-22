package cheapauth

import (
	"fmt"
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// OriginStatusBadge renders a compact speed test latency or failure pill for a
// candidate gateway origin. Shared by Settings and the account popover submenu.
func OriginStatusBadge(c *ui.Context, res *core.CandidateResult, testing, compact bool) {
	if res == nil {
		if testing {
			kit.StatusPill(c, i18n.T(c, "gateway_node_testing"), nil, kit.Neutral, compact)
		}
		return
	}
	if res.Invalid != nil {
		kit.StatusPill(c, i18n.T(c, "gateway_node_invalid_url"), nil, kit.Negative, compact)
		return
	}
	if res.Result == nil {
		return
	}
	switch res.Result.Verdict {
	case core.ProbeOK:
		tone := kit.Positive
		switch core.ClassifyLatency(res.Result.LatencyMS) {
		case core.LatencySlow:
			tone = kit.Caution
		case core.LatencyVerySlow:
			tone = kit.Negative
		}
		kit.StatusPill(c, i18n.T(c, "gateway_node_latency", "ms", res.Result.LatencyMS), nil, tone, compact)
	case core.ProbeUnauthorized:
		kit.StatusPill(c, i18n.T(c, "gateway_node_unauthorized"), nil, kit.Caution, compact)
	case core.ProbeHTTPError:
		kit.StatusPill(c, i18n.T(c, "gateway_node_http", "status", res.Result.Status), nil, kit.Caution, compact)
	default:
		kit.StatusPill(c, i18n.T(c, "gateway_node_unreachable"), nil, kit.Negative, compact)
	}
}

func gatewayOriginsSection(c *ui.Context, p *theme.Palette, state *State) {
	candidates := state.OriginCandidates()
	active := state.ActiveOrigin()
	busy := state.OriginSwitching || state.LoggingOut || state.Stage == SyncingGateway

	ui.Box(c).FillWidth().Height(1).Background(p.Border)
	ui.Column(c.Key("gateway-origins")).FillWidth().Gap(10).Children(func() {
		ui.Row(c.Key("gateway-origins-header")).FillWidth().AlignItems(ui.Center).Gap(8).Children(func() {
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
				ui.Text(c, i18n.T(c, "gateway_origin_title")).FontSize(14).FontWeight(600).TextColor(p.Foreground)
				ui.Text(c, i18n.T(c, "gateway_origin_detail")).FontSize(12).TextColor(p.MutedForeground)
			})
			testLabel := i18n.T(c, "gateway_node_test_all")
			if state.OriginTesting {
				testLabel = i18n.T(c, "gateway_node_testing")
			}
			kit.SecondaryBusy(c.Key("origin-test-all"), testLabel, state.OriginTesting).
				Disabled(state.OriginTesting || busy || state.TestOrigins == nil).
				OnClick(func() {
					if state.TestOrigins != nil {
						state.TestOrigins()
					}
				})
		})

		if state.OriginSwitching {
			ui.Row(c.Key("origin-switching")).Gap(6).AlignItems(ui.Center).Children(func() {
				kit.Busy(c)
				ui.Text(c, i18n.T(c, "gateway_origin_switching")).FontSize(12).TextColor(p.MutedForeground)
			})
		}

		if fb := state.OriginConfig.RestoreTarget(); fb != "" {
			ui.Text(c, i18n.T(c, "gateway_origin_fallback_hint", "url", fb)).FontSize(12).TextColor(p.MutedForeground)
		}

		if state.OriginError != "" {
			ui.Column(c.Key("origin-error")).FillWidth().Padding(8).Radius(p.Radius).
				Background(p.Danger.Alpha(0.12)).Border(1, p.Danger.Alpha(0.35)).Children(func() {
				ui.Text(c, state.OriginError).FontSize(12).TextColor(p.Danger)
			})
		}

		for i, origin := range candidates {
			nodeURL := origin
			inUse := active == nodeURL
			name, detail := state.OriginDisplayLabel(c, nodeURL)
			res := state.OriginResultFor(nodeURL)
			removable := state.OriginConfig.IsRemovable(nodeURL)

			border := p.Border
			bg := p.Surface
			if inUse {
				border = p.Primary
				bg = p.Primary.Alpha(0.08)
			}

			ui.Row(c.Key(fmt.Sprintf("origin-row-%d", i))).FillWidth().Padding(10, 12).
				Radius(p.Radius).Border(1, border).Background(bg).AlignItems(ui.Center).Gap(10).Children(func() {
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
					ui.Text(c, name).FontSize(13).FontWeight(500).SingleLine().TextColor(p.Foreground)
					ui.Text(c, nodeURL).FontSize(12).SingleLine().TextColor(p.MutedForeground)
					if detail != "" {
						ui.Text(c, detail).FontSize(11).SingleLine().TextColor(p.MutedForeground)
					}
				})
				OriginStatusBadge(c, res, state.OriginTesting, false)
				if inUse {
					kit.StatusPill(c, i18n.T(c, "gateway_node_active"), icons.Check, kit.Info, false)
				} else {
					kit.Secondary(c.Key(fmt.Sprintf("origin-use-%d", i)), i18n.T(c, "gateway_node_use")).
						Height(28).PaddingX(10).
						Disabled(busy || state.SelectOrigin == nil).
						OnClick(func() {
							if state.SelectOrigin != nil {
								state.SelectOrigin(nodeURL)
							}
						})
				}
				if removable {
					kit.Ghost(c.Key(fmt.Sprintf("origin-remove-%d", i)), i18n.T(c, "gateway_node_remove"), false).
						Height(28).PaddingX(8).
						Disabled(busy || state.RemoveOrigin == nil).
						OnClick(func() {
							if state.RemoveOrigin != nil {
								state.RemoveOrigin(nodeURL)
							}
						})
				}
			})
		}

		// Add custom gateway node row.
		ui.Row(c.Key("origin-add-row")).FillWidth().AlignItems(ui.Center).Gap(8).Children(func() {
			input := ui.TextInput(c.Key("origin-input"), &state.OriginInput).
				Label(i18n.T(c, "gateway_origin_placeholder")).
				Placeholder(i18n.T(c, "gateway_origin_placeholder")).
				Disabled(busy || state.AddOrigin == nil).
				Grow(1).Shrink(1).MinWidth(0)
			hasInput := strings.TrimSpace(state.OriginInput) != ""
			submit := func() {
				if !hasInput || busy || state.AddOrigin == nil {
					return
				}
				state.AddOrigin(strings.TrimSpace(state.OriginInput))
			}
			if input.Submitted() {
				submit()
			}
			kit.Secondary(c.Key("origin-add"), i18n.T(c, "gateway_node_add")).
				Disabled(!hasInput || busy || state.AddOrigin == nil).
				OnClick(submit)
		})

		// Auto-select toggle row.
		autoOn := state.OriginConfig.AutoSelect
		ui.Row(c.Key("origin-auto-row")).FillWidth().AlignItems(ui.Center).Gap(10).Children(func() {
			sw := ui.Switch(c.Key("origin-auto-switch"), &autoOn).
				Label(i18n.T(c, "gateway_origin_auto_select")).
				Disabled(state.LoggingOut || state.SetOriginAuto == nil)
			if sw.Changed() && state.SetOriginAuto != nil {
				state.SetOriginAuto(autoOn)
			}
			ui.Text(c, i18n.T(c, "gateway_origin_auto_select")).FontSize(13).TextColor(p.Foreground)
		})
	})
}
