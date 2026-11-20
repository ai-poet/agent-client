package shell

import (
	"fmt"
	"slices"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preview"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/sidebar"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func projectRowPlus(tt *ui.Tester, locale string, row ui.AccessInfo) (ui.AccessInfo, bool) {
	label := i18n.In(locale, "workspace_new_session")
	for _, info := range tt.AccessAll() {
		if info.Role != ui.RoleButton || info.Name != label {
			continue
		}
		if info.Bounds.X < RailWidth || info.Bounds.X >= RailWidth+NavWidth {
			continue
		}
		cy := info.Bounds.Y + info.Bounds.H/2
		if cy >= row.Bounds.Y && cy <= row.Bounds.Y+row.Bounds.H {
			return info, true
		}
	}
	return ui.AccessInfo{}, false
}

func TestProjectQuickNewSession(t *testing.T) {
	t.Run("HoverPlusCreatesInTargetProjectAndExpands", func(t *testing.T) {
		a, tt := newTester(t)
		target := a.Data.Projects[1]
		a.Sidebar.SetCollapsed([]string{target.ID})
		tt.Move(900, 600)
		settle(tt)

		row := visibleSidebarRow(t, tt, target.Name)
		if _, shown := projectRowPlus(tt, "en", row); shown {
			t.Fatalf("project row %q shows '+' button without hover", target.Name)
		}

		tt.Move(row.Bounds.X+24, row.Bounds.Y+row.Bounds.H/2)
		tt.Frame()
		plus, shown := projectRowPlus(tt, "en", row)
		if !shown {
			t.Fatalf("hovering project row %q did not reveal '+' quick button", target.Name)
		}

		beforeTarget := len(target.Conversations)
		beforeUnassigned := len(a.Data.Unassigned)
		tt.ClickAt(plus.Bounds.X+plus.Bounds.W/2, plus.Bounds.Y+plus.Bounds.H/2)
		settle(tt)

		if len(target.Conversations) != beforeTarget+1 {
			t.Fatalf("target project %q conversations = %d, want %d", target.ID, len(target.Conversations), beforeTarget+1)
		}
		if len(a.Data.Unassigned) != beforeUnassigned {
			t.Fatalf("unassigned conversations changed from %d to %d", beforeUnassigned, len(a.Data.Unassigned))
		}
		if a.Data.ProjectOf(a.Sidebar.Selected) != target {
			t.Fatalf("selected %q belongs to %v, want project %q", a.Sidebar.Selected, a.Data.ProjectOf(a.Sidebar.Selected), target.ID)
		}
		if slices.Contains(a.Sidebar.Collapsed(), target.ID) {
			t.Fatalf("project %q stayed collapsed after quick new session", target.ID)
		}
		if a.Page != ConversationPage {
			t.Fatalf("page = %d, want ConversationPage", a.Page)
		}
		if editor := i18n.In("en", "composer"); !tt.Focused(editor) {
			t.Fatalf("composer %q is not focused after quick new session", editor)
		}
	})

	t.Run("ContextMenuMoreMenuAndProjectPageEquivalent", func(t *testing.T) {
		en := func(key string) string { return i18n.In("en", key) }
		editor := en("composer")

		// 1. Right-click context menu on a collapsed project row.
		a := New(preview.Load())
		a.Settings.Model.DemoMode = true
		tt := ui.NewTesterWith(a.View, 1280, 820, opaque)
		tt.SetLocale("en")
		tt.Frame()
		settle(tt)

		target := a.Data.Projects[1]
		a.Sidebar.SetCollapsed([]string{target.ID})
		tt.Move(900, 600)
		settle(tt)

		row := visibleSidebarRow(t, tt, target.Name)
		before := len(target.Conversations)
		tt.RightClickAt(row.Bounds.X+row.Bounds.W/2, row.Bounds.Y+row.Bounds.H/2)
		if err := tt.ChooseMenuItem(en("workspace_new_session")); err != nil {
			t.Fatalf("context menu New session failed: %v (%q)", err, tt.Menu())
		}
		settle(tt)
		firstDraft := a.Sidebar.Selected
		if len(target.Conversations) != before+1 || a.Data.ProjectOf(firstDraft) != target || slices.Contains(a.Sidebar.Collapsed(), target.ID) || !tt.Focused(editor) {
			t.Fatalf("context menu: count=%d selected=%q collapsed=%v focused=%v", len(target.Conversations), firstDraft, a.Sidebar.Collapsed(), tt.Focused(editor))
		}

		// Mark first draft sent so subsequent entry points create fresh sessions.
		a.Data.Find(firstDraft).Title = "Sent from context menu"
		a.Sidebar.SetCollapsed([]string{target.ID})
		a.demoProject(target)
		settle(tt)
		if a.Page != ProjectPage {
			t.Fatalf("expected ProjectPage, got %d", a.Page)
		}

		// 2. Main area "New session" primary button on ProjectPage.
		var mainNew ui.AccessInfo
		foundMain := false
		for _, info := range tt.AccessAll() {
			if info.Role == ui.RoleButton && info.Name == en("workspace_new_session") && info.Bounds.X > RailWidth+NavWidth {
				mainNew, foundMain = info, true
				break
			}
		}
		if !foundMain {
			t.Fatalf("ProjectPage missing main area New session button: %q", tt.Texts())
		}
		tt.ClickAt(mainNew.Bounds.X+mainNew.Bounds.W/2, mainNew.Bounds.Y+mainNew.Bounds.H/2)
		settle(tt)
		secondDraft := a.Sidebar.Selected
		if len(target.Conversations) != before+2 || a.Data.ProjectOf(secondDraft) != target || a.Page != ConversationPage || slices.Contains(a.Sidebar.Collapsed(), target.ID) || !tt.Focused(editor) {
			t.Fatalf("main area button: count=%d selected=%q page=%d collapsed=%v focused=%v", len(target.Conversations), secondDraft, a.Page, a.Sidebar.Collapsed(), tt.Focused(editor))
		}

		// 3. ProjectPage header More menu "New session" item.
		a.Data.Find(secondDraft).Title = "Sent from project page"
		a.Sidebar.SetCollapsed([]string{target.ID})
		a.demoProject(target)
		settle(tt)
		more, ok := tt.AccessByRole(ui.RoleMenuButton, en("more"))
		if !ok {
			t.Fatalf("ProjectPage header missing More menu button: %q", tt.Texts())
		}
		tt.ClickAt(more.Bounds.X+more.Bounds.W/2, more.Bounds.Y+more.Bounds.H/2)
		tt.Advance(250 * time.Millisecond)
		tt.Frame()
		clickItem(t, tt, en("workspace_new_session"))
		settle(tt)
		thirdDraft := a.Sidebar.Selected
		if len(target.Conversations) != before+3 || a.Data.ProjectOf(thirdDraft) != target || a.Page != ConversationPage || slices.Contains(a.Sidebar.Collapsed(), target.ID) || !tt.Focused(editor) {
			t.Fatalf("more menu: count=%d selected=%q page=%d collapsed=%v focused=%v", len(target.Conversations), thirdDraft, a.Page, a.Sidebar.Collapsed(), tt.Focused(editor))
		}
	})

	t.Run("LiveHoverPlusCallsNewDraftAndFocusesComposer", func(t *testing.T) {
		projA := &viewmodel.Project{
			ID: "proj-a", Host: "local", Name: "alpha-service", Path: "/work/alpha", Branch: "main",
			Conversations: []*viewmodel.Conversation{{ID: "conv-a", Title: "Alpha chat"}},
		}
		projB := &viewmodel.Project{
			ID: "proj-b", Host: "local", Name: "beta-web", Path: "/work/beta", Branch: "main",
			Conversations: []*viewmodel.Conversation{{ID: "conv-b", Title: "Beta chat"}},
		}
		data := &viewmodel.Data{
			Projects:  []*viewmodel.Project{projA, projB},
			Workspace: &viewmodel.Workspace{Hosts: []viewmodel.Host{{ID: "local", Name: "local_host_name", Platform: "macos", Local: true}}},
		}
		a := NewLive(data)
		a.Sidebar.Selected = "conv-a"
		a.Sidebar.SetCollapsed([]string{projB.ID})

		var draftedFor *viewmodel.Project
		draftCount := 0
		a.Sidebar.Actions = &sidebar.Actions{
			NewDraft: func(pr *viewmodel.Project) {
				draftedFor = pr
				draftCount++
				cv := &viewmodel.Conversation{ID: fmt.Sprintf("draft/%d", draftCount)}
				pr.Conversations = append([]*viewmodel.Conversation{cv}, pr.Conversations...)
				a.Sidebar.Reveal(pr.ID)
				a.Page = ConversationPage
				a.Sidebar.Selected = cv.ID
			},
		}

		tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
		tt.SetLocale("zh-CN")
		tt.Frame()

		row := visibleSidebarRow(t, tt, projB.Name)
		tt.Move(row.Bounds.X+24, row.Bounds.Y+row.Bounds.H/2)
		tt.Frame()
		plus, shown := projectRowPlus(tt, "zh-CN", row)
		if !shown {
			t.Fatalf("live project row %q did not reveal '+' on hover", projB.Name)
		}
		shot(t, tt, "project-hover-plus-zh")

		tt.ClickAt(plus.Bounds.X+plus.Bounds.W/2, plus.Bounds.Y+plus.Bounds.H/2)
		settle(tt)
		if draftedFor != projB || a.Sidebar.Selected != "draft/1" || slices.Contains(a.Sidebar.Collapsed(), projB.ID) {
			t.Fatalf("live quick new session: draftedFor=%v selected=%q collapsed=%v", draftedFor, a.Sidebar.Selected, a.Sidebar.Collapsed())
		}
		editor := i18n.In("zh-CN", "composer")
		if !tt.Focused(editor) {
			t.Fatalf("live composer %q is not focused after quick new session", editor)
		}
		shot(t, tt, "project-quick-new-session-zh")

		// Capture project overview page with main area "新建会话" and More menu open.
		a.OpenProject(projB, ProjectActions{
			NewSession: func(pr *viewmodel.Project) { a.Sidebar.NewSession(pr) },
		})
		settle(tt)
		more, ok := tt.AccessByRole(ui.RoleMenuButton, i18n.In("zh-CN", "more"))
		if !ok {
			t.Fatalf("zh-CN ProjectPage missing More menu button")
		}
		tt.ClickAt(more.Bounds.X+more.Bounds.W/2, more.Bounds.Y+more.Bounds.H/2)
		tt.Advance(250 * time.Millisecond)
		tt.Frame()
		shot(t, tt, "project-overview-more-menu-zh")
	})
}
