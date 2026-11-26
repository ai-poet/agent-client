package shell

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/conversation"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panel"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func TestTraceButtonOpensOriginatingPane(t *testing.T) {
	first := &viewmodel.Conversation{ID: "first", Turns: []conversation.Turn{{ID: "one"}}}
	second := &viewmodel.Conversation{ID: "second", Turns: []conversation.Turn{{ID: "two"}}}
	a := NewLive(&viewmodel.Data{Workspace: &viewmodel.Workspace{}, Unassigned: []*viewmodel.Conversation{first, second}})
	a.Hooks.TraceStatus = func(cv *viewmodel.Conversation) (bool, bool, int) { return true, true, 3 }
	a.focusPane(session(first.ID))
	a.MountPanel()
	tester := ui.NewTester(func(c *ui.Context) { a.paneHeader(c, session(second.ID)) }, 640, 90)
	tester.SetLocale("en")
	tester.Frame()
	b, ok := tester.AccessByRole(ui.RoleButton, i18n.In("en", "trace_title"))
	if !ok || !tester.HasText("3") {
		t.Fatal("split header lost trace status")
	}
	tester.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
	tester.Frame()
	if a.Sidebar.Selected != second.ID || !a.PanelOpen || a.Panel.Active.Kind != panel.Trace || len(a.ConversationTabs(first.ID)) != 0 {
		t.Fatal("trace mounted on wrong pane")
	}
}
