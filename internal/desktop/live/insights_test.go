package live

import (
	"context"
	"testing"
	"testing/synctest"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panel"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestInsightDrilldownAndUnmount(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r, _, _, id := cacheTestRuntime(t)
		ctx, cancel := context.WithCancel(r.ctx)
		p := &insightsPage{owner: r, node: r.controller.Target().String(), seen: true, cancel: cancel}
		r.insights = p
		turn := protocol.TurnID{5}
		name := "request"
		attempt := uint64(2)
		gateway := "indexed-only-gateway"
		row := protocol.UsageRequest{Position: protocol.UsagePosition{Node: r.controller.Target()}, Session: id, Turn: turn, RequestID: &name, Attempt: &attempt, GatewayRequestID: &gateway}
		r.openInsight(nil, p, row)
		if r.App.Sidebar.Selected != id.String() || r.App.Panel.Active.Kind != panel.Trace || r.tracePanel == nil || r.tracePanel.jump != turn {
			t.Fatal("did not open originating conversation and turn")
		}
		// The first frame can have neither a sidecar row nor a watched
		// transcript. The detail remains available while the jump waits.
		r.conversationCopies = map[protocol.SessionID]conversationCopy{id: {}}
		r.conversationsStale.Store(false)
		r.tracePanel.requested = true
		tt := ui.NewTester(func(c *ui.Context) { r.traceFrame(c, id.String()); panel.View(c, r.App.Panel, nil, nil) }, 360, 680)
		tt.SetLocale("en")
		tt.Frame()
		if !tt.HasText(gateway) || r.tracePanel.jump != turn || len(r.App.Panel.TraceRecent) != 0 || len(r.App.Panel.TracePage) != 0 {
			t.Fatal("cold detail or pending jump lost", tt.Texts())
		}
		r.finishInsightsFrame()
		r.finishInsightsFrame()
		if r.insights != nil || ctx.Err() == nil {
			t.Fatal("closing insights retained the read")
		}
	})
}
