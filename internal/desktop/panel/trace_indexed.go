package panel

import (
	"encoding/json"
	"strconv"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// SelectIndexedTrace retains only the selected authorized attempt for this
// mount. The index timestamp is completion time, not a trace start time.
func (s *State) SelectIndexedTrace(q protocol.UsageRequest) {
	if q.RequestID == nil || *q.RequestID == "" || q.Attempt == nil {
		return
	}
	s.SelectTrace(*q.RequestID, int(*q.Attempt))
	r := &traceRow{RequestID: *q.RequestID, Attempt: int(*q.Attempt), Turn: q.Turn.String(), Verdict: q.Verdict}
	r.Provider.ID, r.Provider.Model = q.Provider.String(), q.Model
	if q.Status != nil {
		r.Status = int(*q.Status)
	}
	if q.Outcome != nil {
		r.Outcome = string(*q.Outcome)
	}
	if q.EndKind != nil {
		r.Response.End = string(*q.EndKind)
	}
	if q.FailureKind != nil {
		r.Failure.Kind = *q.FailureKind
	}
	if q.GatewayRequestID != nil {
		r.Response.Gateway = *q.GatewayRequestID
	}
	r.Timing = map[string]uint64{}
	if q.ElapsedUs != nil {
		r.Timing["elapsed_us"] = *q.ElapsedUs
	}
	if q.FirstTokenUs != nil {
		r.Timing["first_token_us"] = *q.FirstTokenUs
	}
	r.Usage = map[string]json.RawMessage{}
	set := func(key string, value *uint64) {
		if value != nil {
			r.Usage[key] = json.RawMessage(strconv.FormatUint(*value, 10))
		}
	}
	if q.Tokens != nil {
		set("input", &q.Tokens.Input)
		set("output", &q.Tokens.Output)
	}
	set("cache_read", q.CacheRead)
	set("cache_write", q.CacheWrite)
	s.trace.indexed = r
}
