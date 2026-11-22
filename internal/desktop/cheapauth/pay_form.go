package cheapauth

import (
	"fmt"
	"math"
	"strings"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

func rechargeForm(c *ui.Context, d *RechargeDialog) {
	cfg := d.Config
	if cfg.UserDisplayName != "" {
		ui.Text(c, cfg.UserDisplayName).FontWeight(600)
	}
	if cfg.UserBalance != nil {
		ui.Text(c, i18n.T(c, "pay_current_balance", "balance", usd(*cfg.UserBalance)))
	}
	if d.Plan != nil {
		planSummary(c, *d.Plan)
		ui.Text(c, i18n.T(c, "pay_estimated_charge", "amount", cny(planCharge(d))))
	} else if cfg.BalanceDisabled {
		ui.Text(c, i18n.T(c, "recharge_disabled"))
	} else {
		rechargeAmount(c, d)
	}
	if d.Plan != nil || !cfg.BalanceDisabled {
		ui.Text(c, i18n.T(c, "recharge_method")).FontWeight(600)
		count := 0
		ui.Row(c.Key("method-chips")).Gap(6).Wrap().Children(func() {
			for i, method := range cfg.EnabledPaymentTypes {
				if !d.offered(method) {
					continue
				}
				count++
				label := core.PaymentLabel(method, strings.HasPrefix(i18n.Match(c.Locale()), "zh"))
				selected := method == d.Method
				kit.Ghost(c.Key(i), label, selected).Height(28).Role(ui.RoleToggleButton).Checked(selected).
					Disabled(d.Busy).OnClick(func() { d.Method = method; d.Message = "" })
			}
		})
		if limit, ok := cfg.MethodLimits[d.Method]; ok && !limit.Available {
			ui.Text(c, i18n.T(c, "pay_method_unavailable"))
		}
		if core.IsStripe(d.Method) {
			ui.Text(c, i18n.T(c, "pay_stripe_hint"))
		}
		if count == 0 {
			ui.Text(c, i18n.T(c, "recharge_empty"))
		}
		if fee := cfg.FeeRate(d.Method); fee > 0 {
			ui.Text(c, i18n.T(c, "pay_method_fee", "rate", fee))
		}

	}
	if d.Plan == nil {
		if d.BrowsePlans != nil {
			kit.Secondary(c.Key("plans"), i18n.T(c, "plans_browse_link")).Disabled(d.Busy).OnClick(d.BrowsePlans)
		}
		if d.Redeem != nil {
			label := i18n.T(c, "pay_redeem_code")
			kit.Field(c, "redeem", label, "", func() { ui.TextInput(c.Key("redeem-input"), &d.RedeemCode).Label(label).Disabled(d.Busy).FillWidth() })
			kit.Secondary(c.Key("redeem-code"), i18n.T(c, "pay_redeem")).Disabled(d.Busy || strings.TrimSpace(d.RedeemCode) == "").OnClick(func() { d.Redeem(strings.TrimSpace(d.RedeemCode)) })
		}
	}
}

func planCharge(d *RechargeDialog) float64 {
	price := d.Plan.Price
	return (math.Round(price*100) + max(0, math.Ceil(price*d.Config.FeeRate(d.Method)-1e-7))) / 100
}

func usd(amount float64) string { return fmt.Sprintf("$%.2f", amount) }
func cny(amount float64) string { return fmt.Sprintf("\u00a5%.2f", amount) }

func rechargeAmount(c *ui.Context, d *RechargeDialog) {
	cfg := d.Config
	label := i18n.T(c, "recharge_amount")
	kit.Field(c, "amount", label, "", func() { ui.TextInput(c.Key("value"), &d.Amount).Label(label).Disabled(d.Busy).FillWidth() })
	floor, ceiling := cfg.EffectiveMin(d.Method), cfg.EffectiveMax(d.Method)

	if cfg.MaxDailyAmount > 0 {
		ui.Text(c, i18n.T(c, "pay_max_daily", "amount", usd(cfg.MaxDailyAmount))).FontSize(12)
	}

	if remaining := cfg.MethodLimits[d.Method].Remaining; remaining != nil {
		ui.Text(c, i18n.T(c, "pay_remaining_limit", "amount", usd(*remaining))).FontSize(12)
	}
	now := time.Now()
	for i, rule := range cfg.Promotions {
		ui.Column(c.Key(fmt.Sprintf("promo-%d", i))).Gap(2).Children(func() {
			if rule.Name != "" {
				ui.Text(c, rule.Name).FontWeight(600)
			}
			line := core.DescribePromotion(rule, c.Locale())
			if !rule.Available || !core.IsInWindow(rule, now) {
				line = i18n.T(c, "pay_promo_unavailable", "tier", line)
				ui.Text(c, line).Opacity(.55)
			} else {
				ui.Text(c, line).FontWeight(600)
			}
			if rule.Description != "" {
				ui.Text(c, rule.Description).FontSize(12)
			}
		})
	}
	active := core.ActivePromotions(cfg.Promotions, now)
	var thresholds []float64
	for _, tier := range active {
		thresholds = append(thresholds, tier.MinAmount)
	}
	n, valid := parseAmount(d.Amount)
	ui.Row(c.Key("quick-amounts")).Wrap().Gap(4).Children(func() {
		for _, v := range core.MergeQuickAmounts([]float64{10, 20, 50, 100, 200, 500}, thresholds, floor, ceiling) {
			label := "$" + core.FormatAmount(v)
			if p := core.PickBestPromotion(v, active); p != nil {
				label += " +$" + core.FormatAmount(p.Bonus)
			}
			kit.Ghost(c.Key(core.ToCents(v)), label, valid && n == v).Disabled(d.Busy).OnClick(func() { d.Amount = core.FormatAmount(v) })
		}
	})
	if hint := core.NextPromotionHint(n, active); hint != nil {
		ui.Text(c, i18n.T(c, "pay_promo_hint", "more", core.FormatAmount(hint.NeedMore), "threshold", core.FormatAmount(hint.Threshold), "bonus", core.FormatAmount(hint.Bonus), "name", hint.Rule.Name))
	}
	credited := "--"
	if valid {
		credited = usd(n)
	}
	ui.Text(c, i18n.T(c, "pay_credited_range", "credited", credited, "min", usd(floor), "max", usd(ceiling)))
	if valid {
		if p := core.PickBestPromotion(n, active); p != nil {
			ui.Text(c, i18n.T(c, "pay_promo_bonus_line", "name", p.Rule.Name, "bonus", usd(p.Bonus)))
			ui.Text(c, i18n.T(c, "pay_promo_total_line", "total", usd(n+p.Bonus))).FontWeight(600)
		}
		base, format := n, usd
		if rate := cfg.BalanceCreditCNYPerUSD; rate != nil && *rate > 0 {
			base *= *rate
			format = cny
			ui.Text(c, i18n.T(c, "pay_settlement_rate", "amount", cny(base)))
		}
		ui.Text(c, i18n.T(c, "pay_estimated_charge", "amount", format(base*(1+cfg.FeeRate(d.Method)/100))))
	}
}
