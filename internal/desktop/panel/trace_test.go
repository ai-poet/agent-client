package panel

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestTraceRowsGroupFilterAndSearch(t *testing.T) {
	page := []json.RawMessage{
		json.RawMessage(`{"turn":"a","request_id":"one","attempt":1,"outcome":"failed","status":429,"cache_reported":true,"retry":{"index":1},"failure":{"kind":"rate_limited","code":"slow_down"},"provider":{"model":"model-X"},"usage":{"input":10,"cache_read":0},"timing":{"elapsed_us":2000},"response":{"gateway_request_id":"gateway-one"}}`),
		json.RawMessage(`{"turn":"b","request_id":"two","attempt":1,"outcome":"succeeded"}`),
		json.RawMessage(`{"turn":"a","request_id":"one","attempt":2,"outcome":"succeeded","usage":{"cache_read":8}}`),
	}
	rows := (&State{}).traceRows(decodeTraceRows(page), "", "")
	if len(rows) != 3 || rows[0].Attempt != 1 || rows[1].Attempt != 2 || rows[2].Turn != "b" || rows[0].metric("input") != "10" || rows[0].metric("output") != "-" || rows[0].elapsed() != "2ms" {
		t.Fatalf("grouped rows: %+v", rows)
	}
	for _, c := range []struct {
		filter, query string
		count         int
	}{
		{"failed", "", 1}, {"retry", "", 2}, {"miss", "", 1}, {"", "429", 1}, {"", " SLOW_down ", 1}, {"", "MODEL-x", 1}, {"", "gateway-one", 1}, {"failed", "absent", 0},
	} {
		if rows := (&State{}).traceRows(decodeTraceRows(page), c.filter, c.query); len(rows) != c.count {
			t.Errorf("%s / %s returned %d rows", c.filter, c.query, len(rows))
		}
	}
}

func TestTraceBodyBoundariesAndEvents(t *testing.T) {
	raw := []byte(strings.Repeat("x", tracePreviewLimit-1) + "\u20ac")
	preview := prepareTraceBody("request", raw, false, false, false)
	if !preview.truncated || len(preview.raw) != tracePreviewLimit-1 {
		t.Fatal("invalid multibyte preview boundary")
	}
	preview = prepareTraceBody("request", []byte(`{"one":[2]}`), false, false, false)
	if !strings.Contains(preview.shown, "\n") || preview.raw != `{"one":[2]}` {
		t.Fatal("pretty view changed copy source")
	}
	preview = prepareTraceBody("response", []byte("data: {\"type\":\"delta\"}\r\n\r\nevent: done\r\ndata: [DONE]\r\n\r\n"), false, true, true)
	if len(preview.events) != 2 || eventLabel(preview.events[0]) != "delta" || eventLabel(preview.events[1]) != "done" || !strings.Contains(preview.raw, "\r\n") {
		t.Fatal("SSE framing or original text lost")
	}
}

func TestTraceSessionReset(t *testing.T) {
	s := State{TracePage: []json.RawMessage{json.RawMessage(`{}`)}, TraceContent: protocol.TracePage{Content: []byte("private")}, TraceError: "session error"}
	s.SelectTrace("request-id", 2)
	if !s.trace.detail || s.trace.request != "request-id" || s.trace.attempt != 2 {
		t.Fatal("retry did not select its exact attempt")
	}
	s.trace.preview = tracePreview{raw: "private"}
	s.trace.confirm = true
	s.ResetTrace()
	if s.trace.confirm || s.trace.detail || s.trace.preview.raw != "" || s.trace.request != "" || len(s.TracePage) != 0 || s.TraceError != "" || s.TraceContent.Content != nil {
		t.Fatal("session switch retained private trace data")
	}
	if !(Tab{Kind: Trace}).valid() || s.Offers(Trace) {
		t.Fatal("trace must be a non-launcher tab")
	}
}

func TestTraceObservationEnrichment(t *testing.T) {
	turn := protocol.TurnID{1}
	page := []json.RawMessage{
		json.RawMessage(`{"request_id":"r","turn":"` + turn.String() + `","attempt":1,"usage":{"cache_read":0},"categories":[{"kind":"system","tokens":12}],"request":{"dropped_headers":["authorization"]}}`),
		json.RawMessage(`{"request_id":"r","turn":"` + turn.String() + `","attempt":2,"usage":{"cache_read":0}}`),
		json.RawMessage(`{"request_id":"other","turn":"older","attempt":1,"usage":{"cache_read":0}}`),
	}
	epoch, tokens := uint64(7), uint64(55)
	s := State{TraceTurnNumbers: map[string]int{"older": 3}, TraceRecent: []protocol.ConversationObservationRequest{{
		ID: "r", Turn: turn, TurnIndex: &epoch, Attempts: []protocol.ConversationObservationAttempt{
			{Index: 2, CacheReported: true, Usage: &protocol.ConversationUsage{}, Categories: []protocol.ConversationObservationCategory{{Kind: "tools", Tokens: &tokens}}},
			{Index: 1, CacheReported: false},
		},
	}}}
	rows := s.traceRows(decodeTraceRows(page), "", "")
	if len(rows) != 3 || len(rows[0].Categories) != 1 || rows[0].Categories[0].Kind != "system" || rows[0].turnNumber != 7 || rows[0].metric("cache_read") != "-" || len(rows[0].Request.Dropped) != 1 {
		t.Fatalf("indexed evidence: %+v", rows)
	}
	if len(rows[1].Categories) != 1 || rows[1].Categories[0].Kind != "tools" || rows[1].metric("cache_read") != "0" || rows[2].turnNumber != 3 || rows[2].metric("cache_read") != "-" {
		t.Fatalf("joined evidence: %+v", rows)
	}
	if misses := s.traceRows(decodeTraceRows(page), "miss", ""); len(misses) != 1 || misses[0].Attempt != 2 {
		t.Fatalf("cache misses: %+v", misses)
	}
}

func TestTraceMarkersSelectNearestAttemptWithinTurn(t *testing.T) {
	rows := []traceRow{{Turn: "a", TS: 100}, {Turn: "b", TS: 180}, {Turn: "a", TS: 200}}
	markers := []TraceMarker{{Turn: "a", TimeMs: 60}, {Turn: "a", TimeMs: 181, Tool: true}, {Turn: "absent", TimeMs: 180}, {Turn: "a"}}
	points := tracePoints(rows, markers)
	if len(points) != 2 || points[0].row != 0 || points[1].row != 2 || points[1].TimeMs != 181 || !points[1].Tool {
		t.Fatalf("marker targets: %+v", points)
	}
}
