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
		outcome, status := "succeeded", 200
		if i == 0 {
			outcome, status = "failed", 429
		}
		s.TracePage = append(s.TracePage, json.RawMessage(fmt.Sprintf(`{"turn":"turn-%02d","ts":1795500000000,"request_id":"req-%d","attempt":1,"outcome":%q,"status":%d,"provider":{"model":"fixture-chat"},"usage":{"input":415,"output":32,"cache_read":16},"timing":{"elapsed_us":85000,"first_token_us":12000},"request":{"body":"bodies/request.req.gz"}}`, 1+i/4, i, outcome, status)))
	}
	s.TraceReadBody = func(path string) {
		s.TraceContent = protocol.TracePage{Body: path, Content: []byte(`{"model":"fixture-chat","messages":[]}`)}
	}
	var jumped string
	s.TraceJump = func(_, turn string) { jumped = turn }
	s.TraceExport = func(_, _, _ string) {}
	tester := ui.NewTesterWith(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		ui.Column(c).Fill().Children(func() { View(c, s, nil, kit.KeyText) })
	}, 560, 640, ui.TesterOptions{Preferences: ui.Preferences{ReduceMotion: true, ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	text := func(key string) string { return i18n.In("zh-CN", key) }
	frame := func() { tester.Frame(); tester.Frame() }
	frame()
	click := func(key string) {
		t.Helper()
		b, ok := tester.AccessByRole(ui.RoleButton, text(key))
		if !ok || b.Bounds.H <= 0 {
			t.Fatalf("missing %s: %q", key, tester.Texts())
		}
		tester.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
		frame()
	}
	click("trace_locate")
	if jumped != "turn-01" {
		t.Fatalf("wrong turn: %q", jumped)
	}
	shot(t, tester, "trace-560")
	click("trace_request")
	if s.TraceContent.Body != "bodies/request.req.gz" || !tester.HasText(text("content_copy")) {
		t.Fatal("request tab did not load the selected body")
	}
	click("trace_export")
	shot(t, tester, "trace-export")
	s.trace.export = false
	tester.SetSize(360, 640)
	frame()
	if tester.HasText(text("trace_locate")) {
		t.Fatal("narrow list showed overview")
	}
	shot(t, tester, "trace-360-list")
	// A retry opens the exact detail, even with the list collapsed.
	s.SelectTrace("req-11", 1)
	frame()
	click("trace_locate")
	if jumped != "turn-03" {
		t.Fatalf("narrow detail jumped to %q", jumped)
	}
	shot(t, tester, "trace-360-detail")
}
