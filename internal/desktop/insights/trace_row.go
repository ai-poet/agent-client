package insights

import (
	"fmt"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func requestRow(c *ui.Context, r protocol.UsageRequest, open func(protocol.UsageRequest)) {
	t := func(k string) string { return i18n.T(c, k) }
	title := r.Session.String()
	if r.SessionTitle != nil && *r.SessionTitle != "" {
		title = *r.SessionTitle
	}
	at := time.UnixMilli(r.Position.TimestampMs).Local().Format("2006-01-02 15:04:05")
	caption := t("trace_view") + ": " + title + " / " + at + " #" + number(r.Attempt)
	row := kit.GhostLook(c, ui.ButtonBase(c.Key(r.Position)).Label(caption), false).FillWidth().MinWidth(0)
	row.Children(func() {
		ui.Column(c).FillWidth().MinWidth(0).Padding(8).Gap(4).Children(func() {
			line := func(v string) { ui.Text(c, v).FillWidth().SingleLine().FontSize(12) }
			ui.Text(c, title).FillWidth().SingleLine().FontSize(15)
			line(at + "   #" + number(r.Attempt))
			line(t("trace_turn_label") + ": " + r.Turn.String())
			line(t("usage_column_provider") + ": " + r.ProviderName + " / " + r.Model)
			bits := []string{}
			if r.Status != nil {
				bits = append(bits, fmt.Sprintf("HTTP %d", *r.Status))
			}
			if r.Outcome != nil {
				bits = append(bits, string(*r.Outcome))
			}
			if r.EndKind != nil {
				bits = append(bits, string(*r.EndKind))
			}
			if r.FailureKind != nil {
				bits = append(bits, *r.FailureKind)
			}
			if len(bits) > 0 {
				line(strings.Join(bits, "   "))
			}
			if r.Credential != nil {
				line(t("insights_credential") + ": " + r.Credential.String())
			}
			ui.Row(c).Wrap().Gap(8).Children(func() {
				metric := func(k, v string) { ui.Text(c, t(k)+": "+v).FontSize(12) }
				metric("usage_column_duration", duration(c, r.ElapsedUs))
				metric("usage_column_first_token", duration(c, r.FirstTokenUs))
				var input, output *uint64
				if r.Tokens != nil {
					input, output = &r.Tokens.Input, &r.Tokens.Output
				}
				metric("meter_input", number(input))
				metric("meter_output", number(output))
				metric("meter_cache_read", number(r.CacheRead))
				metric("trace_cache_write", number(r.CacheWrite))
			})
			if r.RequestID != nil {
				line(t("trace_request_id") + ": " + *r.RequestID)
			}
			if r.GatewayRequestID != nil {
				line(t("insights_gateway_id") + ": " + *r.GatewayRequestID)
			}
			if v := r.Verdict; v != nil {
				line(t(v.Summary))
			}
		})
	}).OnClick(func() { open(r) })
}
