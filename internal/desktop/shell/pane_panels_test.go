package shell

import (
	"slices"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panel"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/panes"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

// The right panel belongs to the header tab's pane, conversation or main
// terminal alike: switching tabs shows each pane's own column, and coming
// back restores it with its terminal still attached.
func TestPanelBelongsToPane(t *testing.T) {
	firstID, thirdID := "panel/first", "panel/third"
	data := &viewmodel.Data{
		Workspace: &viewmodel.Workspace{},
		Projects: []*viewmodel.Project{{
			ID:   "project/1",
			Name: "Project",
			Conversations: []*viewmodel.Conversation{
				{ID: firstID, Title: "First"},
				{ID: thirdID, Title: "Third"},
			},
		}},
	}
	a := NewLive(data)
	stopped := 0
	a.Panel.TerminalActions = &panel.TerminalActions{
		View:  func(c *ui.Context, id string) { ui.Text(c, "details screen "+id) },
		Pane:  func(c *ui.Context, id string) { ui.Text(c, "main screen "+id) },
		Close: func(string) { stopped++ },
	}
	tt := ui.NewTesterWith(a.View, 1280, 820, opaque)
	tt.SetLocale("en")
	frames := func() {
		tt.Frame()
		tt.Advance(time.Second)
		tt.Frame()
	}
	// tab clicks a workspace tab: the leftmost header button of its title, as
	// the details column's own tabs follow the workspace tabs.
	tab := func(title string) {
		t.Helper()
		var b ui.AccessInfo
		for _, info := range tt.AccessAll() {
			r := info.Bounds
			if info.Role == ui.RoleButton && info.Name == title && r.W > 0 && r.Y+r.H <= HeaderHeight && (b.Bounds.W == 0 || r.X < b.Bounds.X) {
				b = info
			}
		}
		if b.Bounds.W == 0 {
			t.Fatalf("no header tab %q: %q", title, tt.Texts())
		}
		tt.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
		frames()
	}
	for _, id := range []string{thirdID, firstID} {
		a.Sidebar.Selected = id
		a.panes.Keep(session(id))
		frames()
	}
	details := panel.Tab{Kind: panel.Terminal, ID: "details-terminal"}
	a.MountPanel()
	a.PanelOpen = true
	a.Panel.Show(details)
	frames()
	if !a.PanelOpen || a.Panel.Active != details || !tt.HasText("details screen "+details.ID) {
		t.Fatalf("the conversation's details terminal: open %v, active %v, texts %q", a.PanelOpen, a.Panel.Active, tt.Texts())
	}

	a.OpenWorkspaceTerminal("main-terminal", "Project shell")
	main := panes.Target{Kind: panes.Terminal, ID: "main-terminal"}
	a.panes.Keep(main)
	frames()
	if a.panes.Active != main || a.PanelOpen || tt.HasText("details screen "+details.ID) || !slices.Equal(a.ConversationTabs(firstID), []panel.Tab{details}) {
		t.Fatalf("the terminal tab borrowed the conversation's panel: open %v, shown %v, first's %v", a.PanelOpen, a.Panel.Tabs, a.ConversationTabs(firstID))
	}
	side := panel.Tab{Kind: panel.Browser, ID: "1"}
	a.MountPanel()
	a.PanelOpen = true
	a.Panel.Show(side)
	frames()
	if !a.PanelOpen || a.Panel.Active != side || !slices.Equal(a.ConversationTabs(firstID), []panel.Tab{details}) {
		t.Fatalf("the terminal tab's own panel: open %v, shown %v, first's %v", a.PanelOpen, a.Panel.Tabs, a.ConversationTabs(firstID))
	}
	if convs := a.PanelConversations(); slices.Contains(convs, terminalPanel(main.ID)) {
		t.Fatalf("PanelConversations included terminal pane: %v", convs)
	}

	tab("Third")
	if a.Sidebar.Selected != thirdID || a.PanelOpen || len(a.Panel.Tabs) != 0 {
		t.Fatalf("a conversation that never opened its panel inherited one: selected %q, open %v, tabs %v", a.Sidebar.Selected, a.PanelOpen, a.Panel.Tabs)
	}

	tab("First")
	if a.Sidebar.Selected != firstID || !a.PanelOpen || a.Panel.Active != details || !slices.Equal(a.Panel.Tabs, []panel.Tab{details}) || !tt.HasText("details screen "+details.ID) {
		t.Fatalf("the conversation's panel did not come back: open %v, tabs %v, active %v, texts %q", a.PanelOpen, a.Panel.Tabs, a.Panel.Active, tt.Texts())
	}

	tab("Project shell")
	if a.panes.Active != main || !a.PanelOpen || !slices.Equal(a.Panel.Tabs, []panel.Tab{side}) || !tt.HasText("main screen main-terminal") {
		t.Fatalf("the terminal tab's panel did not come back: active %v, open %v, tabs %v", a.panes.Active, a.PanelOpen, a.Panel.Tabs)
	}
	if !a.HasPanelTab(details) || stopped != 0 {
		t.Fatalf("switching tabs closed a terminal: tabs %v, stopped %d", a.AllPanelTabs(), stopped)
	}

	// Toggling the right panel while on a terminal pane affects only that pane.
	a.togglePanel()
	frames()
	if a.PanelOpen {
		t.Fatal("togglePanel did not close the terminal pane's panel")
	}
	a.togglePanel()
	frames()
	if !a.PanelOpen {
		t.Fatal("togglePanel did not reopen the terminal pane's panel")
	}

	// Closing the main terminal's surface takes its panel, not its process,
	// and does not persist a ghost panel for the closed terminal.
	a.DropWorkspaceTerminal(main.ID)
	frames()
	if a.HasPanelTab(side) || !a.HasPanelTab(details) || stopped != 0 || !a.PanelOpen || a.Panel.Active != details {
		t.Fatalf("closing the terminal tab: tabs %v, stopped %d, open %v, active %v", a.AllPanelTabs(), stopped, a.PanelOpen, a.Panel.Active)
	}
	if saved := a.Prefs(); saved.PanelOf == terminalPanel(main.ID) || saved.Panels[terminalPanel(main.ID)].Open || len(saved.Panels[terminalPanel(main.ID)].Tabs) != 0 {
		t.Fatalf("closed terminal left a ghost saved panel: %+v", saved)
	}
}
