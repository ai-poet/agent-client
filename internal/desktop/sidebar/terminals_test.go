package sidebar

import (
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func TestTerminalRowsSortByShownTitle(t *testing.T) {
	pr := &viewmodel.Project{ID: "project", Name: "Project"}
	s := New(&viewmodel.Data{Projects: []*viewmodel.Project{pr}}, "")
	*s.Sort = int(SortName)
	s.Actions = &Actions{Terminals: func(*viewmodel.Project) []Terminal {
		return []Terminal{
			{ID: "1", Title: "alice@host:~/zeta"},
			{ID: "2", Title: "~/beta"},
			{ID: "3", Title: "zed@host:~/alpha"},
		}
	}}
	tt := ui.NewTester(func(c *ui.Context) { View(c, s) }, 320, 600)
	tt.SetLocale("en")
	tt.Frame()
	want := []string{"~/alpha", "~/beta", "~/zeta"}
	var rows []string
	for _, text := range tt.Texts() {
		if slices.Contains(want, text) {
			rows = append(rows, text)
		}
	}
	if got := slices.Compact(rows); !slices.Equal(got, want) {
		t.Fatalf("terminal rows = %q, want %q", got, want)
	}
}

func TestTerminalTitleDropsShellPrompt(t *testing.T) {
	for title, want := range map[string]string{
		"lixinhua@appledeMBP-2:/Volumes/Web/sailry-go": "/Volumes/Web/sailry-go",
		"dev@host: ~/app": "~/app",
		"zsh":             "zsh",
		"vim notes@2.md":  "vim notes@2.md",
	} {
		if got := TerminalTitle(title); got != want {
			t.Errorf("TerminalTitle(%q) = %q, want %q", title, got, want)
		}
	}
}

func TestTerminalSelectionClearsConversationSelection(t *testing.T) {
	pr := &viewmodel.Project{
		ID:            "p1",
		Host:          "local",
		Name:          "Project",
		Conversations: []*viewmodel.Conversation{{ID: "c1", Title: "Session One"}},
	}
	s := New(&viewmodel.Data{Projects: []*viewmodel.Project{pr}}, "c1")
	s.Actions = &Actions{
		Terminals: func(*viewmodel.Project) []Terminal {
			return []Terminal{{ID: "t1", Title: "dev@host:/workspace/app"}}
		},
	}
	tt := ui.NewTester(func(c *ui.Context) { View(c, s) }, 320, 600)
	tt.SetLocale("en")
	tt.Frame()
	if row, ok := tt.Access("Session One"); !ok || (!row.Selected && !row.Checked) {
		t.Fatalf("conversation row should be selected when no terminal is shown: %+v (ok=%v)", row, ok)
	}
	if row, ok := tt.Access("/workspace/app"); !ok || row.Selected || row.Checked {
		t.Fatalf("terminal row should not be selected initially: %+v (ok=%v)", row, ok)
	}

	s.Terminal = "t1"
	tt.Frame()
	if row, ok := tt.Access("/workspace/app"); !ok || (!row.Selected && !row.Checked) {
		t.Fatalf("terminal row should be selected when shown: %+v (ok=%v)", row, ok)
	}
	if row, ok := tt.Access("Session One"); !ok || row.Selected || row.Checked {
		t.Fatalf("conversation row should not be selected while terminal is shown: %+v (ok=%v)", row, ok)
	}
}
