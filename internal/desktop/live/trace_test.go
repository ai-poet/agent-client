package live

import (
	"encoding/json"
	"testing"
	"testing/synctest"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panel"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestTraceExportLimitToast(t *testing.T) {
	r := &Runtime{}
	tester := ui.NewTester(func(c *ui.Context) {
		queue := r.queue
		r.queue = nil
		for _, post := range queue {
			post(c)
		}
		kit.ToastHost(c)
	}, 640, 480)
	tester.SetLocale("zh-CN")
	r.ReportTraceExport(&link.FaultError{Fault: protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: protocol.TraceExportTooLargeMessage}})
	tester.Frame()
	if !tester.HasText(i18n.In("zh-CN", "trace_export_limit")) {
		t.Fatalf("missing narrower scope hint: %q", tester.Texts())
	}
}

func TestTracePanelFollowsWatchedSession(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r, transport, cv, id := cacheTestRuntime(t)
		state := cacheTestFrame(t, r, cv, id)
		var queries []protocol.TraceQuery
		transport.handle = func(request protocol.Request) (link.Response, bool) {
			if request.Command.Kind != "read_trace" {
				return link.Response{}, false
			}
			q, _ := detail[protocol.TraceQuery](request.Command.Data)
			queries = append(queries, q)
			page := protocol.TracePage{Session: id, Body: q.Body}
			switch {
			case q.StatusOnly:
				page.Status = &protocol.TraceStatus{Recording: true, Failed: 2}
			case q.Body != "":
				page.Content = []byte("fixture")
			case q.After == "":
				page.Records = []json.RawMessage{json.RawMessage(`{"request_id":"first","attempt":1}`)}
				page.Tail = "first"
			case q.After == "first":
				page.Records = []json.RawMessage{json.RawMessage(`{"request_id":"second","attempt":1}`)}
				page.Tail = "second"
			}
			return link.Response{Output: protocol.Output{Kind: "trace", Data: page}}, true
		}
		settle := func() { r.workers.Wait(); applyOverviewCallbacks(r) }
		r.traceStatus(cv)
		settle()
		known, recording, failed := r.traceStatus(cv)
		if !known || !recording || failed != 2 || len(queries) != 1 || !queries[0].StatusOnly {
			t.Fatal("closed panel fetched records or lost status", queries)
		}
		x := conversationActions{r: r, cv: cv}
		x.ViewTrace("first", 1)
		if !r.App.PanelOpen || r.App.Panel.Active.Kind != panel.Trace || r.App.Sidebar.Selected != id.String() {
			t.Fatal("retry did not mount its conversation")
		}
		r.traceFrame(nil, id.String())
		settle()
		if len(r.App.Panel.TracePage) != 1 {
			t.Fatal("trace index was not fetched")
		}
		r.beginConversationFrame()
		state.InventoryVersion++
		r.conversationCopies[id] = conversationCopy{state: state, ok: true}
		r.traceFrame(nil, id.String())
		settle()
		if len(r.App.Panel.TracePage) != 2 || queries[len(queries)-1].After != "first" {
			t.Fatal("watch did not append from the last cursor", queries)
		}
		r.App.Panel.TraceReadBody("bodies/second.req.gz")
		settle()
		if string(r.App.Panel.TraceContent.Content) != "fixture" || queries[len(queries)-1].PreviewBytes != protocol.TracePreviewLimit {
			t.Fatal("wrong lazy preview", queries)
		}
		r.finishTraceFrame()
		r.finishTraceFrame()
		if r.tracePanel != nil || r.App.Panel.TraceContent.Content != nil || r.App.Panel.TraceReadBody != nil {
			t.Fatal("closing trace retained private state")
		}
	})
}
