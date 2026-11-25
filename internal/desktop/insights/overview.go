package insights

import (
	"fmt"
	"maps"
	"slices"
	"strconv"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func percent(n, d uint64) string {
	if d == 0 {
		return "-"
	}
	return fmt.Sprintf("%.1f%%", 100*float64(n)/float64(d))
}

func overview(c *ui.Context, r *protocol.UsageReport) {
	t := func(k string) string { return i18n.T(c, k) }
	x := r.Insights
	if x == nil {
		ui.Text(c, t("unavailable"))
		return
	}
	if x.Attempts == 0 {
		ui.Text(c, t("usage_empty"))
		return
	}
	cell := func(k string, value any) {
		ui.Column(c.Key(k)).MinWidth(132).Padding(10).Gap(3).Radius(theme.Of(c).Radius).Background(theme.Of(c).Muted).Children(func() {
			ui.Text(c, t(k)).FontSize(12).TextColor(theme.Of(c).MutedForeground)
			ui.Text(c, fmt.Sprint(value)).FontSize(17)
		})
	}
	ui.Row(c.Key("summary")).Wrap().Gap(8).Children(func() {
		cell("insights_attempts", x.Attempts)
		cell("meter_succeeded", x.Succeeded)
		cell("meter_failed", x.Failed)
		cell("approval_cancelled", x.Cancelled)
		cell("trace_retries", x.Retries)
		cell("insights_failure_rate", percent(x.Failed, x.Failed+x.Cancelled+x.Succeeded))
		cell("meter_cache_hit", percent(x.CacheRead, x.CacheInputTokens))
		cost := "-"
		var covered uint64
		if r.Totals.Cost != nil {
			covered = r.Totals.Cost.Responses
			cost = fmt.Sprintf("$%.6f", float64(r.Totals.Cost.UsdMicros)/1e6)
		}
		cell("usage_cost", cost)
		cell("insights_cost_coverage", percent(covered, x.Attempts))
	})
	ui.Text(c, t("meter_tokens")).FontSize(16)
	if g := tokenMix(c, r.Totals.Tokens, x); len(g) > 0 {
		kit.Treemap(c, "token-mix", g, kit.TreemapOptions{Height: 144})
	} else {
		ui.Text(c, t("meter_state_unknown"))
	}
	for _, row := range []struct {
		k string
		p protocol.UsagePercentiles
	}{{"meter_first_response", x.FirstTokenUs}, {"usage_duration", x.ElapsedUs}} {
		ui.Text(c, t(row.k)+"   "+i18n.T(c, "insights_samples", "count", row.p.Samples)).FontSize(16)
		ui.Row(c.Key(row.k)).Wrap().Gap(12).Children(func() {
			for i, v := range []*uint64{row.p.P50, row.p.P95, row.p.P99} {
				n := "-"
				if v != nil {
					n = i18n.T(c, "usage_duration_ms", "value", fmt.Sprintf("%.1f", float64(*v)/1000))
					if row.p.Approximate {
						n = "~" + n
					}
				}
				ui.Text(c, []string{"P50", "P95", "P99"}[i]+"   "+n)
			}
		})
	}
	for _, row := range []struct {
		k      string
		counts map[string]uint64
	}{{"insights_end", x.EndKinds}, {"insights_failure", x.FailureKinds}} {
		if len(row.counts) == 0 {
			continue
		}
		ui.Text(c, t(row.k)).FontSize(16)
		ui.Row(c.Key(row.k)).Wrap().Gap(12).Children(func() {
			for _, k := range slices.Sorted(maps.Keys(row.counts)) {
				ui.Text(c, fmt.Sprintf("%s: %d", k, row.counts[k]))
			}
		})
	}
}

func tokenMix(c *ui.Context, tokens *protocol.ConversationUsage, x *protocol.UsageInsights) []kit.TreemapGroup {
	if tokens == nil {
		return nil
	}
	var groups []kit.TreemapGroup
	input := tokens.Input - min(tokens.Input, x.CacheRead)
	input -= min(input, x.CacheWrite)
	for _, row := range []struct {
		k string
		n uint64
	}{{"usage_uncached_input", input}, {"meter_cache_read", x.CacheRead}, {"trace_cache_write", x.CacheWrite}, {"meter_output", tokens.Output - min(tokens.Output, tokens.Reasoning)}, {"meter_reasoning", tokens.Reasoning}} {
		if row.n == 0 {
			continue
		}
		label := i18n.T(c, row.k)
		v := strconv.FormatUint(row.n, 10)
		groups = append(groups, kit.TreemapGroup{ID: row.k, Label: label, Detail: v, Items: []kit.TreemapItem{{ID: row.k, Label: label, Value: float64(row.n), Detail: v}}})
	}
	return groups
}
