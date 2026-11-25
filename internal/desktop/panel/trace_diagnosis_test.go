package panel

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestTraceObservationWithoutBody(t *testing.T) {
	v := &protocol.TraceVerdict{Code: "network", Severity: "error", Summary: "diagnosis_network"}
	req := protocol.ConversationObservationRequest{ID: "request", Model: "model", TimestampMs: 123, Attempts: []protocol.ConversationObservationAttempt{{Index: 1, Verdict: v, Outcome: protocol.ConversationObservationOutcomeFailed}}}
	state := State{TraceRecent: []protocol.ConversationObservationRequest{req}}
	rows := state.traceRows(nil, "failed", "")
	if len(rows) != 1 || rows[0].Verdict != v || rows[0].TS != 123 || rows[0].Request.Body != "" {
		t.Fatalf("metadata-only rows: %+v", rows)
	}
	captured := traceRow{RequestID: req.ID, Turn: req.Turn.String(), Attempt: 1, Outcome: "failed", Request: traceSide{Body: "bodies/captured.req.gz"}}
	rows = state.traceRows([]traceRow{captured}, "", "")
	if len(rows) != 1 || rows[0].Verdict != v || rows[0].Request.Body != captured.Request.Body {
		t.Fatalf("overlapping capture: %+v", rows)
	}
}
