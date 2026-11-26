package panel

import (
	"encoding/json"
	"fmt"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestTracePanelInteraction(t *testing.T) {
	s := New(&preview.Workspace{})
	s.Show(Tab{Kind: Trace})
	for i := range 200 {
		outcome, status, extra := "succeeded", 200, ""
		if i == 0 {
			outcome, status = "failed", 429
			extra = `,"failure":{"kind":"rate_limited"},"retry":{"index":2},"verdict":{"code":"rate_limited","severity":"error","summary":"diagnosis_rate_limited","actions":["diagnosis_action_wait"],"retry_after_ms":2000},` +
				`"response":{"body":"bodies/response.res.gz","end":"http_error","content_type":"text/event-stream","gateway_request_id":"gw-123","headers":{"Retry-After":["2"],"Content-Type":["text/event-stream"]}}`
		}
		s.TracePage = append(s.TracePage, json.RawMessage(fmt.Sprintf(`{"turn":"turn-%02d","ts":%d,"request_id":"req-%d","attempt":1,"outcome":%q,"status":%d,"provider":{"model":"fixture-chat","id":"provider-1"},"usage":{"input":415,"output":32,"cache_read":16,"reasoning":8},"timing":{"elapsed_us":85000,"first_token_us":12000,"dispatch_us":900,"sent_us":1800,"headers_us":9000},"request":{"body":"bodies/request.req.gz","headers":{"Content-Type":["application/json"]}}%s}`,
			1+i/4, 1795500000000+int64(i)*1000, i, outcome, status, extra)))
	}
	reads := 0
	s.TraceReadBody = func(path string) {
		reads++
		content := `{"model":"fixture-chat","messages":[{"role":"user","content":"hello"}],"stream":true}`
		if path == "bodies/response.res.gz" {
			content = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit\"}}\n\n"
		}
		s.TraceContent = protocol.TracePage{Body: path, Content: []byte(content)}
	}
	var jumped string
	s.TraceJump = func(_, turn string) { jumped = turn }
	exported := ""
	s.TraceExport = func(scope, mode, _ string) { exported = scope + "/" + mode }
	tester := ui.NewTesterWith(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		ui.Column(c).Fill().Children(func() { View(c, s, nil, kit.KeyText) })
	}, 640, 720, ui.TesterOptions{Preferences: ui.Preferences{ReduceMotion: true, ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	text := func(key string) string { return i18n.In("zh-CN", key) }
	frame := func() { tester.Frame(); tester.Frame() }
	frame()
	click := func(role ui.Role, key string) {
		t.Helper()
		b, ok := tester.AccessByRole(role, text(key))
		if !ok || b.Bounds.H <= 0 {
			t.Fatalf("missing %s: %q", key, tester.Texts())
		}
		tester.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
		frame()
	}
	// The latest attempt opens by default.
	click(ui.RoleButton, "trace_locate")
	if jumped != "turn-50" {
		t.Fatalf("wrong turn: %q", jumped)
	}
	shot(t, tester, "trace-640")
	click(ui.RoleRadio, "trace_request")
	if s.TraceContent.Body != "bodies/request.req.gz" || reads != 1 {
		t.Fatalf("request tab did not load the selected body once: %d", reads)
	}
	if _, ok := tester.AccessByRole(ui.RoleButton, text("content_copy")); !ok {
		t.Fatal("loaded body has no copy action")
	}
	frame()
	if reads != 1 {
		t.Fatal("a frame repeated the body read")
	}
	shot(t, tester, "trace-request")
	// The failed attempt keeps the tab, and its diagnosis leads the overview.
	s.SelectTrace("req-0", 1)
	frame()
	if !tester.HasText(text("diagnosis_rate_limited")) {
		t.Fatalf("failed attempt has no diagnosis: %q", tester.Texts())
	}
	shot(t, tester, "trace-failed")
	click(ui.RoleRadio, "trace_response")
	if s.TraceContent.Body != "bodies/response.res.gz" {
		t.Fatal("response tab did not load its body")
	}
	shot(t, tester, "trace-response")
	click(ui.RoleRadio, "trace_metrics")
	shot(t, tester, "trace-metrics")
	click(ui.RoleButton, "trace_export")
	shot(t, tester, "trace-export")
	click(ui.RoleButton, "trace_export")
	if exported != "all/shareable" || s.trace.export {
		t.Fatalf("export: %q", exported)
	}
	tester.SetSize(360, 640)
	s.trace.detail = false
	frame()
	if tester.HasText(text("trace_locate")) {
		t.Fatal("narrow list showed overview")
	}
	shot(t, tester, "trace-360-list")
	// A retry opens the exact detail, even with the list collapsed.
	s.SelectTrace("req-11", 1)
	frame()
	click(ui.RoleButton, "trace_locate")
	if jumped != "turn-03" {
		t.Fatalf("narrow detail jumped to %q", jumped)
	}
	shot(t, tester, "trace-360-detail")
}

func TestTracePanelEmptyAndMetadataOnly(t *testing.T) {
	s := New(&preview.Workspace{})
	s.Show(Tab{Kind: Trace})
	s.TraceExport = func(_, _, _ string) { t.Fatal("exported without a capture") }
	tester := ui.NewTesterWith(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		ui.Column(c).Fill().Children(func() { View(c, s, nil, kit.KeyText) })
	}, 560, 480, ui.TesterOptions{Preferences: ui.Preferences{ReduceMotion: true, ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	text := func(key string) string { return i18n.In("zh-CN", key) }
	tester.Frame()
	tester.Frame()
	if !tester.HasText(text("trace_empty")) {
		t.Fatalf("no empty state: %q", tester.Texts())
	}
	shot(t, tester, "trace-empty")
	// Observations alone list the attempt, but there is nothing to export
	// and no body to read.
	v := &protocol.TraceVerdict{Code: "network", Severity: "error", Summary: "diagnosis_network"}
	s.TraceRecent = []protocol.ConversationObservationRequest{{ID: "request", Model: "model", TimestampMs: 123, Attempts: []protocol.ConversationObservationAttempt{{Index: 1, Verdict: v, Outcome: protocol.ConversationObservationOutcomeFailed}}}}
	tester.Frame()
	tester.Frame()
	if b, ok := tester.AccessByRole(ui.RoleButton, text("trace_export")); !ok || !b.Disabled {
		t.Fatal("export is offered without a captured record")
	}
	b, ok := tester.AccessByRole(ui.RoleRadio, text("trace_request"))
	if !ok {
		t.Fatalf("no request tab: %q", tester.Texts())
	}
	tester.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
	tester.Frame()
	tester.Frame()
	if !tester.HasText(text("trace_body_missing")) || !tester.HasText(text("trace_body_capture_hint")) {
		t.Fatalf("metadata-only body: %q", tester.Texts())
	}
	shot(t, tester, "trace-metadata-only")
}
