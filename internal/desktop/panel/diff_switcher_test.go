package panel

import (
	"fmt"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

func manyChanges(n int) []preview.FileChange {
	changes := make([]preview.FileChange, 0, n)
	for i := range n {
		path := fmt.Sprintf("pkg/mod%02d/file_%02d.go", i/5, i)
		changes = append(changes, preview.FileChange{
			Path:     path,
			Status:   "M",
			Unstaged: "M",
			Added:    i + 1,
			Removed:  1,
			Hunks: []preview.Hunk{{
				Header: "@@ -1,2 +1,2 @@",
				Lines: []preview.DiffLine{
					{Kind: preview.Removed, Old: 1, Text: fmt.Sprintf("old line %d", i)},
					{Kind: preview.Added, New: 1, Text: fmt.Sprintf("new line %d", i)},
				},
			}},
		})
	}
	return changes
}

func TestDiffFileSwitcherBoundedAndScrollable(t *testing.T) {
	ws := preview.Load().Workspace
	wsCopy := *ws
	wsCopy.Changes = manyChanges(20)
	pr := NewProjects(&wsCopy).Of("many-changes")
	state := New(&wsCopy)
	state.HasProject = true
	first := wsCopy.Changes[0].Path
	pr.SetGitDiffState(first, "all", GitDiffState{Binary: true})
	state.Show(Tab{Kind: Changes, ID: first})

	const width, height = 640, 620
	tt := ui.NewTesterWith(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		ui.Column(c).Fill().Clip().Children(func() {
			View(c, state, pr, func(ui.Modifiers, ui.Key) string { return "" })
		})
	}, width, height, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("en")
	tt.Frame()
	tt.Frame()

	trigger, ok := tt.AccessByRole(ui.RoleMenuButton, first)
	if !ok {
		t.Fatalf("diff file switcher button %q not found: %q", first, tt.Texts())
	}
	tt.ClickAt(trigger.Bounds.X+trigger.Bounds.W/2, trigger.Bounds.Y+trigger.Bounds.H/2)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()

	menu, ok := tt.AccessByRole(ui.RoleMenu, "")
	if !ok {
		t.Fatalf("diff file switcher menu did not open: %q", tt.Texts())
	}
	if menu.Bounds.H <= 0 || menu.Bounds.H > DiffFileMenuMaxHeight+2 {
		t.Fatalf("diff file switcher menu height = %.1f, want <= %d", menu.Bounds.H, DiffFileMenuMaxHeight)
	}
	if menu.Bounds.Y < 0 || menu.Bounds.Y+menu.Bounds.H > height {
		t.Fatalf("diff file switcher menu %v overflows viewport height %d", menu.Bounds, height)
	}
	if checked, ok := tt.AccessByRole(ui.RoleMenuItemCheckBox, first); !ok || !checked.Checked {
		t.Fatalf("current file %q is not checked in switcher menu: ok=%v checked=%v", first, ok, checked.Checked)
	}

	// Scroll inside the bounded dropdown to reach a file past the initial fold.
	target := wsCopy.Changes[18].Path
	if item, found := tt.AccessByRole(ui.RoleMenuItem, target); found && menu.Bounds.Contains(item.Bounds.X+item.Bounds.W/2, item.Bounds.Y+item.Bounds.H/2) {
		t.Fatalf("%q should start outside the menu viewport, got %v in %v", target, item.Bounds, menu.Bounds)
	}
	tt.Scroll(menu.Bounds.X+menu.Bounds.W/2, menu.Bounds.Y+menu.Bounds.H/2, 0, 500)
	tt.Frame()
	item, ok := tt.AccessByRole(ui.RoleMenuItem, target)
	if !ok || !menu.Bounds.Contains(item.Bounds.X+item.Bounds.W/2, item.Bounds.Y+item.Bounds.H/2) {
		t.Fatalf("scrolled switcher menu missing %q: %q", target, tt.Texts())
	}
	tt.ClickAt(item.Bounds.X+item.Bounds.W/2, item.Bounds.Y+item.Bounds.H/2)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()

	if pr.Git.Change() != target || state.Active != (Tab{Kind: Changes, ID: target}) {
		t.Fatalf("switching file gave git=%q active=%+v, want %q", pr.Git.Change(), state.Active, target)
	}
	if _, stillOpen := tt.AccessByRole(ui.RoleMenu, ""); stillOpen {
		t.Fatalf("switcher menu stayed open after picking %q", target)
	}

	// Keyboard: open switcher with Enter and dismiss with Escape.
	if !tt.Focused(target) {
		t.Fatalf("focus did not follow the switcher to %q", target)
	}
	tt.Key(0, ui.KeyEnter)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()
	if _, open := tt.AccessByRole(ui.RoleMenu, ""); !open {
		t.Fatalf("Enter on focused switcher trigger did not open menu")
	}
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()
	if _, open := tt.AccessByRole(ui.RoleMenu, ""); open {
		t.Fatalf("Escape did not close switcher menu")
	}
}

func TestRevealChangeExpandsCollapsedFoldersAndSwitchesFromHistory(t *testing.T) {
	pr, tt := pageTester(t, Changes)
	// Switch to History and collapse both the tracked group and docs folder.
	pr.Git.view = 1
	groupOpen, folderOpen := false, false
	pr.Git.expanded["tracked"] = &groupOpen
	pr.Git.expanded["dir/tracked/docs"] = &folderOpen
	tt.Frame()

	pr.RevealChange("docs/example.md")
	tt.Frame()

	if pr.Git.view != 0 {
		t.Fatalf("RevealChange kept view=%d, want 0 (Changes)", pr.Git.view)
	}
	if pr.Git.Change() != "docs/example.md" {
		t.Fatalf("RevealChange selected %q, want docs/example.md", pr.Git.Change())
	}
	row, ok := tt.AccessByRole(ui.RoleTreeItem, "example.md")
	if !ok || !row.Selected {
		t.Fatalf("revealed file row example.md: ok=%v selected=%v", ok, row.Selected)
	}
}
