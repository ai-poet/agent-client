package insights

import (
	"fmt"
	"strconv"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func usageGroup(c *ui.Context, s *State, g protocol.UsageGroup, apply func()) {
	t := func(k string) string { return i18n.T(c, k) }
	name := groupName(g)
	if g.Key.Kind == "unassigned" {
		name = t("insights_unassigned")
	}
	key := fmt.Sprintf("%s:%v", g.Key.Kind, keyData(g.Key))
	expanded := s.Expanded == key
	ui.Column(c.Key(key)).MinWidth(0).Gap(4).Children(func() {
		kit.Ghost(c, name, expanded).Expanded(expanded).OnClick(func() {
			if expanded {
				s.Expanded = ""
			} else {
				s.Expanded = key
			}
		})
		if g.Insights != nil {
			ui.Text(c, fmt.Sprintf("%s: %d   %s: %d   %s: %d", t("insights_attempts"), g.Insights.Attempts, t("meter_failed"), g.Insights.Failed, t("trace_retries"), g.Insights.Retries)).FontSize(12)
		}
		if !expanded {
			return
		}
		ui.Row(c).Wrap().Gap(8).Children(func() {
			var input, output *uint64
			if u := g.Metrics.Tokens; u != nil {
				input, output = &u.Input, &u.Output
			}
			metric := func(label, value string) { ui.Text(c, label+": "+value).FontSize(13) }
			metric(t("meter_input"), number(input))
			metric(t("meter_output"), number(output))
			cost := "-"
			if g.Metrics.Cost != nil {
				cost = fmt.Sprintf("$%.6f", float64(g.Metrics.Cost.UsdMicros)/1e6)
			}
			metric(t("usage_cost"), cost)
			if x := g.Insights; x != nil {
				var read, write *uint64
				if x.CacheReadSamples > 0 {
					read = &x.CacheRead
				}
				if x.CacheWriteSamples > 0 {
					write = &x.CacheWrite
				}
				metric(t("meter_cache_read"), number(read))
				metric(t("trace_cache_write"), number(write))
				for _, p := range []struct {
					k string
					v protocol.UsagePercentiles
				}{{"meter_first_response", x.FirstTokenUs}, {"usage_duration", x.ElapsedUs}} {
					value := duration(c, p.v.P95)
					if p.v.P95 != nil && p.v.Approximate {
						value = "~" + value
					}
					metric(t(p.k)+" P95", value)
				}
			}
		})
		if f, ok := narrow(s.Draft, g.Key); ok {
			kit.Secondary(c, t("insights_index")).OnClick(func() { s.Draft = f; s.Tab = 1; s.Draft.ObservedOnly = true; apply() })
		}
	})
}

func number(v *uint64) string {
	if v == nil {
		return "-"
	}
	return strconv.FormatUint(*v, 10)
}
func duration(c *ui.Context, v *uint64) string {
	if v == nil {
		return "-"
	}
	return i18n.T(c, "usage_duration_ms", "value", strconv.FormatFloat(float64(*v)/1000, 'f', 1, 64))
}
