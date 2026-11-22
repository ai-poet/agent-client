package cheapauth

import (
	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
	"strings"
)

func plansList(c *ui.Context, d *RechargeDialog) {
	if d.Busy && len(d.Plans) == 0 {
		ui.Text(c, i18n.T(c, "plans_loading"))
		return
	}
	if len(d.Plans) == 0 {
		if d.Message == "" {
			ui.Text(c, i18n.T(c, "plans_empty"))
		}
		return
	}
	ui.Text(c, i18n.T(c, "plans_intro"))
	for _, p := range d.Plans {
		ui.Column(c.Key(p.ID)).FillWidth().Gap(6).Children(func() {
			planSummary(c, p)
			if p.Description != "" {
				ui.Text(c, p.Description)
			}
			for _, f := range p.Features {
				ui.Text(c, f)
			}
			if p.Platform != "" {
				ui.Text(c, i18n.T(c, "plans_for_cli", "cli", p.Platform))
			}
			if p.RateMultiplier != nil {
				ui.Text(c, i18n.T(c, "plans_rate", "rate", *p.RateMultiplier))
			}
			if len(p.SupportedModelScopes) > 0 {
				ui.Text(c, i18n.T(c, "plans_scopes", "scopes", strings.Join(p.SupportedModelScopes, ", ")))
			} else if p.DefaultMappedModel != "" {
				ui.Text(c, i18n.T(c, "plans_default_model", "model", p.DefaultMappedModel))
			}
			label := "plans_buy"
			if d.OwnedGroups[p.GroupID] {
				label = "plans_renew"
				ui.Text(c, i18n.T(c, "plans_current"))
			}
			kit.Secondary(c.Key("choose"), i18n.T(c, label)).Disabled(d.ChoosePlan == nil || d.Busy).OnClick(func() { d.ChoosePlan(p) })
		})
	}
}

func planSummary(c *ui.Context, p core.SubscriptionPlan) {
	ui.Text(c, p.Name).FontWeight(600)
	unit := "plans_validity_day"
	switch p.ValidityUnit {
	case "week":
		unit = "plans_validity_week"
	case "month":
		unit = "plans_validity_month"
	}
	ui.Text(c, cny(p.Price)+"  "+i18n.T(c, unit, "count", max(1, p.ValidityDays)))
	if orig := p.DiscountedFrom(); orig != nil {
		ui.Text(c, i18n.T(c, "plans_original_price", "price", cny(*orig)))
	}
	if caps := p.Limits; caps != nil {
		var limits []string
		for _, l := range []struct {
			key string
			n   *float64
		}{{"plans_limit_daily", caps.DailyLimitUSD}, {"plans_limit_weekly", caps.WeeklyLimitUSD}, {"plans_limit_monthly", caps.MonthlyLimitUSD}} {
			if l.n != nil && *l.n > 0 {
				limits = append(limits, i18n.T(c, l.key, "amount", usd(*l.n)))
			}
		}
		if len(limits) == 0 {
			ui.Text(c, i18n.T(c, "plans_unlimited"))
		} else {
			ui.Text(c, strings.Join(limits, " / "))
		}
	}
}
