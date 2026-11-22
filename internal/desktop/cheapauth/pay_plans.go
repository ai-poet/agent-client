package cheapauth

import (
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

func plansList(c *ui.Context, d *RechargeDialog) {
	p := theme.Of(c)
	if d.Busy && len(d.Plans) == 0 {
		ui.Row(c.Key("plans-loading")).Gap(8).AlignItems(ui.Center).Children(func() {
			kit.Busy(c)
			ui.Text(c, i18n.T(c, "plans_loading")).FontSize(13).TextColor(p.MutedForeground)
		})
		return
	}
	if len(d.Plans) == 0 {
		if d.Message == "" {
			ui.Text(c, i18n.T(c, "plans_empty")).FontSize(13).TextColor(p.MutedForeground)
		}
		return
	}
	ui.Text(c, i18n.T(c, "plans_intro")).FontSize(12).TextColor(p.MutedForeground)
	for _, plan := range d.Plans {
		item := plan
		ui.Column(c.Key(item.ID)).FillWidth().Padding(12).Radius(p.Radius).Border(1, p.Border).Background(p.Surface).Gap(6).Children(func() {
			planSummary(c, item)
			if item.Description != "" {
				ui.Text(c, item.Description).FontSize(12).TextColor(p.MutedForeground)
			}
			for _, f := range item.Features {
				ui.Text(c, f).FontSize(12)
			}
			if item.Platform != "" {
				ui.Text(c, i18n.T(c, "plans_for_cli", "cli", item.Platform)).FontSize(12).TextColor(p.MutedForeground)
			}
			if item.RateMultiplier != nil {
				ui.Text(c, i18n.T(c, "plans_rate", "rate", *item.RateMultiplier)).FontSize(12).TextColor(p.MutedForeground)
			}
			if len(item.SupportedModelScopes) > 0 {
				ui.Text(c, i18n.T(c, "plans_scopes", "scopes", strings.Join(item.SupportedModelScopes, ", "))).FontSize(12).TextColor(p.MutedForeground)
			} else if item.DefaultMappedModel != "" {
				ui.Text(c, i18n.T(c, "plans_default_model", "model", item.DefaultMappedModel)).FontSize(12).TextColor(p.MutedForeground)
			}
			label := "plans_buy"
			if d.OwnedGroups[item.GroupID] {
				label = "plans_renew"
				kit.Badge(c, i18n.T(c, "plans_current"), kit.Info)
			}
			kit.Secondary(c.Key("choose"), i18n.T(c, label)).Disabled(d.ChoosePlan == nil || d.Busy).OnClick(func() { d.ChoosePlan(item) })
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
