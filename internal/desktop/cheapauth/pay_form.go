package cheapauth

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

func rechargeForm(c *ui.Context, d *RechargeDialog) {
	p := theme.Of(c)
	cfg := d.Config
	if d.Plan != nil {
		ui.Column(c.Key("layer-plan")).FillWidth().Padding(10, 12).Radius(p.Radius).Background(p.Muted).Gap(4).Children(func() {
			planSummary(c, *d.Plan)
		})
	} else if cfg.BalanceDisabled {
		ui.Text(c, i18n.T(c, "recharge_disabled")).FontSize(13).TextColor(p.MutedForeground)
	} else {
		rechargeAmount(c, d)
	}

	if d.Plan != nil || !cfg.BalanceDisabled {
		ui.Column(c.Key("layer-method")).FillWidth().Gap(8).Children(func() {
			ui.Text(c, i18n.T(c, "recharge_method")).FontSize(13).FontWeight(600).TextColor(p.PopoverForeground)
			count := 0
			ui.Row(c.Key("method-chips")).FillWidth().Gap(8).Wrap().Children(func() {
				for i, method := range cfg.EnabledPaymentTypes {
					if !d.offered(method) {
						continue
					}
					count++
					m := method
					available := d.methodAvailable(m)
					label := core.PaymentLabel(m, strings.HasPrefix(i18n.Match(c.Locale()), "zh"))
					meta := methodChipMeta(c, d, m, available)
					selected := m == d.Method
					border := p.Border
					bg := p.Surface
					fg := p.PopoverForeground
					if selected {
						border = p.Primary
						bg = p.Primary.Alpha(0.10)
					} else if !available {
						fg = p.MutedForeground
					}
					chip := ui.ButtonBase(c.Key(i)).Label(label).Padding(7, 10).Radius(p.Radius).
						Border(1, border).Background(bg).Cursor(ui.CursorPointer).
						Role(ui.RoleToggleButton).Checked(selected).Disabled(d.Busy || !available)
					if meta != "" {
						chip.Description(meta)
					}
					if !selected && available && chip.Hovered() {
						chip.Background(p.Hover)
					}
					chip.Children(func() {
						ui.Column(c).Gap(2).AlignItems(ui.Start).Children(func() {
							ui.Text(c, label).FontSize(13).FontWeight(500).SingleLine().TextColor(fg)
							if meta != "" {
								ui.Text(c, meta).FontSize(10).SingleLine().TextColor(p.MutedForeground)
							}
						})
					}).OnClick(func() {
						d.Method = m
						d.Message = ""
					})
				}
			})
			if count == 0 {
				ui.Text(c, i18n.T(c, "recharge_empty")).FontSize(12).TextColor(p.MutedForeground)
			}
			if core.IsStripe(d.Method) {
				ui.Text(c, i18n.T(c, "pay_stripe_hint")).FontSize(12).TextColor(p.MutedForeground)
			}
		})
	}

	if d.Plan == nil && d.Redeem != nil {
		ui.Column(c.Key("layer-redeem")).FillWidth().Gap(8).Children(func() {
			label := i18n.T(c, "pay_redeem_code")
			toggle := ui.ButtonBase(c.Key("redeem-toggle")).Label(label).Height(24).PaddingX(4).Gap(4).
				Radius(p.Radius).Cursor(ui.CursorPointer).AlignItems(ui.Center).Expanded(d.redeemExpanded)
			if toggle.Hovered() {
				toggle.Background(p.Hover)
			}
			toggle.Children(func() {
				chevron := icons.ChevronRight
				if d.redeemExpanded {
					chevron = icons.ChevronDown
				}
				ui.Icon(c, chevron).FontSize(12).TextColor(p.MutedForeground)
				ui.Text(c, label).FontSize(12).TextColor(p.MutedForeground)
			}).OnClick(func() { d.redeemExpanded = !d.redeemExpanded })
			if d.redeemExpanded {
				ui.Row(c.Key("redeem-row")).FillWidth().Gap(8).AlignItems(ui.Center).Children(func() {
					ui.TextInput(c.Key("redeem-input"), &d.RedeemCode).Label(label).Disabled(d.Busy).Grow(1)
					kit.Secondary(c.Key("redeem-code"), i18n.T(c, "pay_redeem")).
						Disabled(d.Busy || strings.TrimSpace(d.RedeemCode) == "").
						OnClick(func() { d.Redeem(strings.TrimSpace(d.RedeemCode)) })
				})
			}
		})
	}
}

func methodChipMeta(c *ui.Context, d *RechargeDialog, method string, available bool) string {
	if !available {
		return i18n.T(c, "pay_chip_unavailable")
	}
	cfg := d.Config
	var parts []string
	if fee := cfg.FeeRate(method); fee > 0 {
		parts = append(parts, i18n.T(c, "pay_chip_fee", "rate", formatPercent(fee)))
	}
	if d.Plan == nil && !core.IsStripe(method) {
		floor, ceiling := cfg.EffectiveMin(method), cfg.EffectiveMax(method)
		if floor > 0 || ceiling > 0 {
			parts = append(parts, i18n.T(c, "pay_chip_limit", "min", "$"+core.FormatAmount(floor), "max", "$"+core.FormatAmount(ceiling)))
		}
		if rem := cfg.MethodLimits[method].Remaining; rem != nil {
			parts = append(parts, i18n.T(c, "pay_chip_remaining", "amount", "$"+core.FormatAmount(*rem)))
		}
	}
	return strings.Join(parts, " · ")
}

func formatPercent(rate float64) string {
	return strconv.FormatFloat(math.Round(rate*100)/100, 'f', -1, 64)
}

func planCharge(d *RechargeDialog) float64 {
	price := d.Plan.Price
	return (math.Round(price*100) + max(0, math.Ceil(price*d.Config.FeeRate(d.Method)-1e-7))) / 100
}

func balanceChargeText(d *RechargeDialog) (string, bool) {
	if d == nil || d.Config == nil {
		return "", false
	}
	n, ok := parseAmount(d.Amount)
	if !ok {
		return "", false
	}
	base, format := n, usd
	if rate := d.Config.BalanceCreditCNYPerUSD; rate != nil && *rate > 0 {
		base *= *rate
		format = cny
	}
	fee := d.Config.FeeRate(d.Method)
	total := (math.Round(base*100) + max(0, math.Ceil(base*fee-1e-7))) / 100
	return format(total), true
}

func usd(amount float64) string { return fmt.Sprintf("$%.2f", amount) }
func cny(amount float64) string { return fmt.Sprintf("\u00a5%.2f", amount) }

func rechargeAmount(c *ui.Context, d *RechargeDialog) {
	p := theme.Of(c)
	cfg := d.Config
	floor, ceiling := cfg.EffectiveMin(d.Method), cfg.EffectiveMax(d.Method)
	now := time.Now()
	active := core.ActivePromotions(cfg.Promotions, now)
	var thresholds []float64
	for _, tier := range active {
		thresholds = append(thresholds, tier.MinAmount)
	}
	n, valid := parseAmount(d.Amount)

	ui.Column(c.Key("layer-amount")).FillWidth().Gap(10).Children(func() {
		label := i18n.T(c, "recharge_amount")
		kit.Field(c, "amount", label, "", func() {
			ui.TextInput(c.Key("value"), &d.Amount).Label(label).Disabled(d.Busy).FillWidth()
		})

		ui.Row(c.Key("quick-amounts")).FillWidth().Wrap().Gap(6).Children(func() {
			for _, v := range core.MergeQuickAmounts([]float64{10, 20, 50, 100, 200, 500}, thresholds, floor, ceiling) {
				val := v
				chipLabel := "$" + core.FormatAmount(val)
				if promo := core.PickBestPromotion(val, active); promo != nil {
					chipLabel += " +$" + core.FormatAmount(promo.Bonus)
				}
				selected := valid && core.ToCents(n) == core.ToCents(val)
				border := p.Border
				bg := p.Surface
				if selected {
					border = p.Primary
					bg = p.Primary.Alpha(0.10)
				}
				chip := ui.ButtonBase(c.Key(core.ToCents(val))).Label(chipLabel).Height(28).PaddingX(10).
					Radius(p.Radius).Border(1, border).Background(bg).Center().Cursor(ui.CursorPointer).
					Role(ui.RoleToggleButton).Checked(selected).Disabled(d.Busy)
				if !selected && chip.Hovered() {
					chip.Background(p.Hover)
				}
				chip.Children(func() {
					ui.Text(c, chipLabel).FontSize(12).FontWeight(500).SingleLine().TextColor(p.PopoverForeground)
				}).OnClick(func() {
					d.Amount = core.FormatAmount(val)
					d.Message = ""
				})
			}
		})

		summary := i18n.T(c, "pay_estimated_credit", "total", "--")
		if valid {
			if best := core.PickBestPromotion(n, active); best != nil && best.Bonus > 0 {
				total := float64(core.ToCents(n)+core.ToCents(best.Bonus)) / 100
				summary = i18n.T(c, "pay_estimated_credit_bonus", "total", usd(total), "bonus", usd(best.Bonus))
			} else {
				summary = i18n.T(c, "pay_estimated_credit", "total", usd(n))
			}
		}
		ui.Text(c, summary).FontSize(13).FontWeight(500).TextColor(p.PopoverForeground)

		if len(cfg.Promotions) > 0 {
			toggleLabel := i18n.T(c, "pay_more_promos")
			chevron := icons.ChevronDown
			if d.promosExpanded {
				toggleLabel = i18n.T(c, "pay_hide_promos")
				chevron = icons.ChevronUp
			}
			toggle := ui.ButtonBase(c.Key("promo-toggle")).Label(toggleLabel).Height(24).PaddingX(4).Gap(4).
				Radius(p.Radius).Cursor(ui.CursorPointer).AlignItems(ui.Center).Expanded(d.promosExpanded)
			if toggle.Hovered() {
				toggle.Background(p.Hover)
			}
			toggle.Children(func() {
				ui.Text(c, toggleLabel).FontSize(12).TextColor(p.MutedForeground)
				ui.Icon(c, chevron).FontSize(12).TextColor(p.MutedForeground)
			}).OnClick(func() { d.promosExpanded = !d.promosExpanded })

			if d.promosExpanded {
				ui.Column(c.Key("promo-rules")).FillWidth().Padding(8, 10).Radius(p.Radius).Background(p.Muted).Gap(6).Children(func() {
					for i, rule := range cfg.Promotions {
						ui.Column(c.Key(fmt.Sprintf("promo-%d", i))).Gap(2).Children(func() {
							if rule.Name != "" {
								ui.Text(c, rule.Name).FontSize(12).FontWeight(600)
							}
							line := core.DescribePromotion(rule, c.Locale())
							if !rule.Available || !core.IsInWindow(rule, now) {
								line = i18n.T(c, "pay_promo_unavailable", "tier", line)
								ui.Text(c, line).FontSize(12).Opacity(.55)
							} else {
								ui.Text(c, line).FontSize(12).FontWeight(500)
							}
							if rule.Description != "" {
								ui.Text(c, rule.Description).FontSize(11).TextColor(p.MutedForeground)
							}
						})
					}
					if hint := core.NextPromotionHint(n, active); hint != nil {
						ui.Text(c, i18n.T(c, "pay_promo_hint", "more", core.FormatAmount(hint.NeedMore), "threshold", core.FormatAmount(hint.Threshold), "bonus", core.FormatAmount(hint.Bonus), "name", hint.Rule.Name)).
							FontSize(11).TextColor(p.MutedForeground)
					}
					if cfg.MaxDailyAmount > 0 {
						ui.Text(c, i18n.T(c, "pay_max_daily", "amount", usd(cfg.MaxDailyAmount))).
							FontSize(11).TextColor(p.MutedForeground)
					}
				})
			}
		}
	})
}
