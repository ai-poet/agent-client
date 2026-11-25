package panel

import (
	"encoding/json"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
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

func (r traceRow) diagnosis(c *ui.Context) {
	v := r.Verdict
	if v == nil {
		return
	}
	tone := kit.Neutral
	if v.Severity == "error" {
		tone = kit.Negative
	} else if v.Severity == "warning" {
		tone = kit.Caution
	}
	kit.Badge(c.Key("verdict"), i18n.T(c, v.Summary), tone)
	if v.CauseKey != "" {
		key := v.CauseKey
		if v.KeyOrderOnly {
			key += ":key-order-only"
		}
		ui.Text(c, key+"  "+v.Path+"  "+v.Volatile).FontSize(12)
	}
	if v.LostTokens > 0 {
		ui.Text(c, i18n.T(c, "diagnosis_lost")+"  "+strconv.FormatUint(v.LostTokens, 10))
	}
	if v.GapStartUs != nil && v.MaxGapUs != nil {
		ui.Text(c, i18n.T(c, "trace_gap")+": "+(time.Duration(*v.MaxGapUs)*time.Microsecond).String()+" @ "+(time.Duration(*v.GapStartUs)*time.Microsecond).String())
	}
	for _, key := range v.Actions {
		if key != "connections_copy_id" {
			ui.Text(c, i18n.T(c, key)).FontSize(12)
		}
	}
	if v.RetryAfterMs != nil {
		ui.Text(c, "Retry-After: "+(time.Duration(*v.RetryAfterMs)*time.Millisecond).String())
	}
	if v.RetryDelayMs != nil {
		ui.Text(c, i18n.T(c, "trace_retries")+": "+(time.Duration(*v.RetryDelayMs)*time.Millisecond).String())
	}
	if len(v.RetrySequence) > 1 {
		ui.Text(c, strings.Join(v.RetrySequence, "  >  ")).FontSize(12)
	}
}
