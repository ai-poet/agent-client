package panel

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestColdIndexedSelectionWithoutWatchOrSidecar(t *testing.T) {
	s := State{}
	id := "outside-watch"
	attempt := uint64(3)
	elapsed := uint64(50000)
	write := uint64(0)
	row := protocol.UsageRequest{Session: protocol.SessionID{1}, Turn: protocol.TurnID{2}, Model: "cold-model",
		RequestID: &id, Attempt: &attempt, Position: protocol.UsagePosition{TimestampMs: 123456}, ElapsedUs: &elapsed, CacheWrite: &write,
		Verdict: &protocol.TraceVerdict{Summary: "diagnosis_unavailable"}}
	s.SelectIndexedTrace(row)
	rows := s.traceRows(nil, "", "")
	if len(rows) != 1 || !s.trace.detail || s.trace.request != id || s.trace.attempt != 3 {
		t.Fatal("indexed attempt was not selected", rows)
	}
	r := rows[0]
	if r.Turn != row.Turn.String() || r.Verdict != row.Verdict || r.Provider.Model != row.Model || r.Timing["elapsed_us"] != elapsed {
		t.Fatal("lost authorized evidence", r)
	}
	if r.TS != 0 || len(r.Timing) != 1 || r.metric("input") != "-" || r.metric("cache_read") != "-" || r.metric("cache_write") != "0" || r.Request.Body != "" || r.Response.Body != "" {
		t.Fatal("manufactured observations", r)
	}
	// A later sidecar wins; the fallback does not add a second attempt.
	captured := traceRow{RequestID: id, Attempt: 3, Turn: r.Turn, TS: 98}
	if got := s.traceRows([]traceRow{captured}, "", ""); len(got) != 1 || got[0].TS != 98 {
		t.Fatal("fallback shadowed captured evidence", got)
	}
	s.ResetTrace()
	if got := s.traceRows(nil, "", ""); len(got) != 0 {
		t.Fatal("selection escaped session lifetime")
	}
}
