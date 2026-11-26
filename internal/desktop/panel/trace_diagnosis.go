package panel

import (
	"encoding/json"
	"slices"
	"strconv"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// The conversation watch includes metadata-only attempts even when no trace
// directory exists. Never synthesize a body reference or an attempt start time.
func withObservedDiagnoses(rows []traceRow, recent []protocol.ConversationObservationRequest) []traceRow {
	rows = slices.Clip(rows)
	type key struct {
		id    string
		index int
	}
	seen := make(map[key]bool, len(rows))
	for _, r := range rows {
		seen[key{r.RequestID, r.Attempt}] = true
	}
	for _, req := range recent {
		for _, a := range req.Attempts {
			k := key{req.ID, int(a.Index)}
			if a.Verdict == nil || seen[k] {
				continue
			}
			seen[k] = true
			r := traceRow{RequestID: req.ID, Attempt: k.index, Turn: req.Turn.String(), Outcome: string(a.Outcome), Verdict: a.Verdict}
			r.Provider.Model = req.Model
			if a.Index == 1 {
				r.TS = req.TimestampMs
			}
			if a.Status != nil {
				r.Status = int(*a.Status)
			}
			if a.EndKind != nil {
				r.Response.End = string(*a.EndKind)
			}
			if a.GatewayRequestID != nil {
				r.Response.Gateway = *a.GatewayRequestID
			}
			if a.RetryReason != nil {
				r.Failure.Kind = *a.RetryReason
				r.Retry = &struct{ Index int }{k.index + 1}
			}
			r.Timing = map[string]uint64{}
			for name, v := range map[string]*uint64{"elapsed_us": a.ElapsedUs, "first_token_us": a.FirstTokenUs, "dispatch_us": a.DispatchUs, "sent_us": a.RequestSentUs, "headers_us": a.HeadersUs} {
				if v != nil {
					r.Timing[name] = *v
				}
			}
			if a.Usage != nil {
				r.Usage = map[string]json.RawMessage{}
				for name, v := range map[string]*uint64{"input": &a.Usage.Input, "output": &a.Usage.Output, "cache_write": a.CacheWrite} {
					if v != nil {
						r.Usage[name] = json.RawMessage(strconv.FormatUint(*v, 10))
					}
				}
				if a.ReasoningReported {
					r.Usage["reasoning"] = json.RawMessage(strconv.FormatUint(a.Usage.Reasoning, 10))
				}
			}
			rows = append(rows, r)
		}
	}
	return rows
}

// diagnosis is the attempt's verdict as a card: the finding, its evidence
// and what to do about it.
func (r traceRow) diagnosis(c *ui.Context) {
	v := r.Verdict
	if v == nil {
		return
	}
	p := theme.Of(c)
	accent, icon := p.MutedForeground, icons.Info
	switch {
	case v.Severity == "error":
		accent, icon = p.Danger, icons.CircleX
	case v.Severity == "warning":
		accent, icon = p.Warning, icons.TriangleAlert
	case v.Code == "normal":
		accent, icon = p.Success, icons.CircleCheck
	}
	detail := func(text string) {
		ui.Text(c, text).FontSize(12).TextColor(p.MutedForeground).Selectable().FillWidth()
	}
	ui.Column(c.Key("verdict")).FillWidth().Gap(6).Padding(10, 12).Radius(p.Radius).
		Background(p.Background.Mix(accent, 0.05)).InsetShadow(0, 0, 0, 1, accent.Alpha(0.16)).Children(func() {
		ui.Row(c).FillWidth().Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Icon(c, icon).FontSize(16).TextColor(accent).Shrink(0)
			ui.Text(c, i18n.T(c, v.Summary)).FontSize(14).FontWeight(500).Grow(1).Shrink(1).MinWidth(0)
		})
		if v.CauseKey != "" {
			key := v.CauseKey
			if v.KeyOrderOnly {
				key += ":key-order-only"
			}
			cause := slices.DeleteFunc([]string{key, v.Path, v.Volatile}, func(s string) bool { return s == "" })
			ui.Text(c, strings.Join(cause, "  ")).FontSize(12).Font("monospace").TextColor(p.MutedForeground).Selectable().FillWidth()
		}
		if v.LostTokens > 0 {
			detail(i18n.T(c, "diagnosis_lost") + "  " + strconv.FormatUint(v.LostTokens, 10))
		}
		if v.GapStartUs != nil && v.MaxGapUs != nil {
			detail(i18n.T(c, "trace_gap") + ": " + traceDuration(*v.MaxGapUs) + " @ " + traceDuration(*v.GapStartUs))
		}
		if v.RetryAfterMs != nil {
			detail("Retry-After: " + traceDuration(*v.RetryAfterMs*1000))
		}
		if v.RetryDelayMs != nil {
			detail(i18n.T(c, "trace_retries") + ": " + traceDuration(*v.RetryDelayMs*1000))
		}
		if len(v.RetrySequence) > 1 {
			detail(strings.Join(v.RetrySequence, "  >  "))
		}
		for _, key := range v.Actions {
			if key == "connections_copy_id" {
				continue
			}
			ui.Row(c.Key(key)).FillWidth().Gap(6).Children(func() {
				ui.Text(c, "•").FontSize(13).TextColor(accent).Shrink(0)
				ui.Text(c, i18n.T(c, key)).FontSize(13).Grow(1).Shrink(1).MinWidth(0)
			})
		}
	})
}
