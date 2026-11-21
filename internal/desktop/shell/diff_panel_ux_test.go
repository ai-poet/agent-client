package shell

import (
	"fmt"
	"path"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/panel"
	"github.com/ai-poet/agent-client/internal/desktop/preview"
	"github.com/egoist/mygo/ui"
)

func TestDiffPanelDirectOpenDropdownAndKeyboard(t *testing.T) {
	data := preview.Load()
	extra := make([]preview.FileChange, 0, len(data.Workspace.Changes)+18)
	extra = append(extra, data.Workspace.Changes...)
	for i := range 18 {
		extra = append(extra, preview.FileChange{
			Path:     fmt.Sprintf("internal/feature/file_%02d.go", i+1),
			Status:   "M",
			Unstaged: "M",
			Added:    i + 2,
			Removed:  1,
			Hunks: []preview.Hunk{{
				Header: "@@ -1,2 +1,2 @@",
				Lines: []preview.DiffLine{
					{Kind: preview.Removed, Old: 1, Text: fmt.Sprintf("oldValue := %d", i)},
					{Kind: preview.Added, New: 1, Text: fmt.Sprintf("newValue := %d", i+1)},
				},
			}},
		})
	}
	data.Workspace.Changes = extra

	a := New(data)
	a.Sidebar.Selected = "sailry-go/theme"
	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()
	tt.Frame()

	added, removed := 0, 0
	for _, f := range a.Data.Workspace.Changes {
		added += f.Added
		removed += f.Removed
	}
	gitLabel := fmt.Sprintf("Git +%d -%d", added, removed)
	first := a.Data.Workspace.Changes[0].Path

	// 1. Clicking the composer diff entry directly opens the right Git diff
	// panel (no in-composer dropdown popover) and highlights the active file.
	badge, ok := tt.AccessByRole(ui.RoleButton, gitLabel)
	if !ok {
		t.Fatalf("composer diff entry %q not found: %q", gitLabel, tt.Texts())
	}
	tt.ClickAt(badge.Bounds.X+badge.Bounds.W/2, badge.Bounds.Y+badge.Bounds.H/2)
	settle(tt)

	if !a.PanelOpen || a.Panel.Active != (panel.Tab{Kind: panel.Changes, ID: first}) {
		t.Fatalf("clicking diff entry opened panel=%v active=%+v, want git %q", a.PanelOpen, a.Panel.Active, first)
	}
	if len(panels(tt, ui.RolePopup)) != 0 {
		t.Fatalf("clicking diff entry unexpectedly opened a popover: %v", panels(tt, ui.RolePopup))
	}
	if _, open := tt.AccessByRole(ui.RoleMenu, ""); open {
		t.Fatal("clicking diff entry unexpectedly opened a menu")
	}
	row, ok := tt.AccessByRole(ui.RoleTreeItem, path.Base(first))
	if !ok || !row.Selected {
		t.Fatalf("active diff file row %q not highlighted: found=%v selected=%v", first, ok, row.Selected)
	}
	shot(t, tt, "diff-panel-opened-zh")

	// 2. Inside the diff panel, opening the file switcher dropdown shows a
	// height-bounded, scrollable menu that is not clipped by the panel.
	switcher, ok := tt.AccessByRole(ui.RoleMenuButton, first)
	if !ok {
		t.Fatalf("in-panel diff file switcher %q not found: %q", first, tt.Texts())
	}
	tt.ClickAt(switcher.Bounds.X+switcher.Bounds.W/2, switcher.Bounds.Y+switcher.Bounds.H/2)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()

	menu, ok := tt.AccessByRole(ui.RoleMenu, "")
	if !ok {
		t.Fatalf("in-panel file switcher dropdown did not open: %q", tt.Texts())
	}
	if menu.Bounds.H <= 0 || menu.Bounds.H > panel.DiffFileMenuMaxHeight+2 {
		t.Fatalf("file switcher dropdown height = %.1f, want <= %d", menu.Bounds.H, panel.DiffFileMenuMaxHeight)
	}
	if menu.Bounds.Y < 0 || menu.Bounds.Y+menu.Bounds.H > 820 {
		t.Fatalf("file switcher dropdown %v clipped/overflowing window 1280x820", menu.Bounds)
	}
	shot(t, tt, "diff-switcher-dropdown-zh")

	// First Escape closes only the dropdown; the right Git diff panel stays open.
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if _, open := tt.AccessByRole(ui.RoleMenu, ""); open {
		t.Fatalf("Escape did not close the file switcher dropdown")
	}
	if !a.PanelOpen {
		t.Fatalf("Escape closing the dropdown also closed the Git diff panel")
	}

	// Reopen dropdown, scroll to a file past the fold, and switch to it.
	tt.ClickAt(switcher.Bounds.X+switcher.Bounds.W/2, switcher.Bounds.Y+switcher.Bounds.H/2)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()
	menu, ok = tt.AccessByRole(ui.RoleMenu, "")
	if !ok {
		t.Fatalf("reopening file switcher dropdown failed")
	}
	target := "internal/feature/file_16.go"
	tt.Scroll(menu.Bounds.X+menu.Bounds.W/2, menu.Bounds.Y+menu.Bounds.H/2, 0, 600)
	tt.Frame()
	item, ok := tt.AccessByRole(ui.RoleMenuItem, target)
	if !ok {
		t.Fatalf("scrolled file switcher missing %q: %q", target, tt.Texts())
	}
	tt.ClickAt(item.Bounds.X+item.Bounds.W/2, item.Bounds.Y+item.Bounds.H/2)
	settle(tt)
	if a.Panel.Active != (panel.Tab{Kind: panel.Changes, ID: target}) {
		t.Fatalf("selecting %q from switcher left active=%+v", target, a.Panel.Active)
	}

	// 3. Pressing Escape with no dropdown open closes the right Git diff panel.
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if a.PanelOpen || a.Page != ConversationPage || a.Sidebar.Selected != "sailry-go/theme" {
		t.Fatalf("Escape did not return to the same conversation: open=%v page=%v selected=%q",
			a.PanelOpen, a.Page, a.Sidebar.Selected)
	}
	shot(t, tt, "diff-panel-closed-zh")

	// 4. Keyboard accessibility on the diff entry: Enter and Space both open
	// the right Git diff panel, and Escape closes it back.
	for _, key := range []struct {
		name string
		code ui.Key
	}{{"Enter", ui.KeyEnter}, {"Space", ui.KeySpace}} {
		if _, ok := tt.AccessByRole(ui.RoleButton, gitLabel); !ok {
			t.Fatalf("%s: diff entry %q missing", key.name, gitLabel)
		}
		// Shift+Tab is the editor's mode shortcut. From an empty draft,
		// plain Tab traverses the controls and wraps back to the chip.
		editor := i18n.In("zh-CN", "composer")
		clickField(t, tt, editor)
		if field, ok := tt.AccessByRole(ui.RoleTextField, editor); !ok || !field.Focused {
			t.Fatalf("%s: composer did not receive focus", key.name)
		}
		reached := false
		for range len(tt.AccessAll()) {
			tt.Key(0, ui.KeyTab)
			tt.Frame()
			if button, ok := tt.AccessByRole(ui.RoleButton, gitLabel); ok && button.Focused {
				reached = true
				break
			}
		}
		if !reached {
			t.Fatalf("%s: could not reach diff entry %q via keyboard Tab", key.name, gitLabel)
		}
		tt.Key(0, key.code)
		settle(tt)
		if !a.PanelOpen || a.Panel.Active != (panel.Tab{Kind: panel.Changes, ID: first}) {
			t.Fatalf("%s on diff entry did not open Git diff panel: open=%v active=%+v", key.name, a.PanelOpen, a.Panel.Active)
		}
		tt.Key(0, ui.KeyEscape)
		settle(tt)
		if a.PanelOpen {
			t.Fatalf("%s: Escape did not close Git diff panel", key.name)
		}
	}
}
