package shell

import (
	"testing"
	"time"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func countText(texts []string, want string) int {
	n := 0
	for _, text := range texts {
		if text == want {
			n++
		}
	}
	return n
}

// TestShellButtonCaptionsAppearOnce guards against duplicate captions on key
// shell and sidebar controls. Each kit button exposes its accessible name once
// and draws its caption once, so a label must appear exactly twice in the
// frame's text list (a third occurrence means a redundant ui.Text child was
// added on top of kit.Primary/Secondary/Ghost/Danger).
func TestShellButtonCaptionsAppearOnce(t *testing.T) {
	project := &viewmodel.Project{
		ID: "proj-guard", Host: "local", Name: "guard-repo", Path: "/tmp/guard-repo", Branch: "main",
		Conversations: []*viewmodel.Conversation{{ID: "conv-guard", Node: "local", Title: "Guard session"}},
	}
	data := &viewmodel.Data{
		Workspace: &viewmodel.Workspace{
			Hosts: []viewmodel.Host{{ID: "local", Name: "local_host_name", Platform: "macos", Local: true}},
		},
		Projects: []*viewmodel.Project{project},
	}

	t.Run("sidebar new session", func(t *testing.T) {
		a := NewLive(data)
		a.Sidebar.Selected = "conv-guard"
		a.NavOpen = true
		tt := ui.NewTesterWith(a.View, 1280, 820, opaque)
		tt.SetLocale("zh-CN")
		tt.Frame()
		label := i18n.In("zh-CN", "conversation")
		if got := countText(tt.Texts(), label); got != 2 {
			t.Fatalf("sidebar new session %q appeared %d times (want 2): %q", label, got, tt.Texts())
		}
	})

	t.Run("project create session and terminal", func(t *testing.T) {
		a := NewLive(data)
		a.NavOpen = false
		a.OpenProject(project, ProjectActions{
			NewSession:  func(*viewmodel.Project) {},
			NewTerminal: func(*viewmodel.Project) {},
		})
		tt := ui.NewTesterWith(a.View, 1280, 820, opaque)
		tt.SetLocale("zh-CN")
		tt.Frame()
		for _, key := range []string{"workspace_new_session", "workspace_new_terminal"} {
			label := i18n.In("zh-CN", key)
			if got := countText(tt.Texts(), label); got != 2 {
				t.Fatalf("project button %q appeared %d times (want 2): %q", label, got, tt.Texts())
			}
		}
	})

	t.Run("avatar popover buttons", func(t *testing.T) {
		a := NewLive(data)
		a.Sidebar.Selected = "conv-guard"
		state := cheapauthui.NewState()
		a.Settings.CheapRouter = state
		tt := ui.NewTesterWith(a.View, 1280, 820, opaque)
		tt.SetLocale("zh-CN")
		tt.Frame()

		// Signed-out popover buttons.
		trigger, ok := tt.AccessByRole(ui.RoleButton, "登录 CheapRouter")
		if !ok {
			t.Fatalf("missing signed-out footer trigger: %q", tt.Texts())
		}
		tt.ClickAt(trigger.Bounds.X+trigger.Bounds.W/2, trigger.Bounds.Y+trigger.Bounds.H/2)
		tt.Frame()
		tt.Advance(250 * time.Millisecond)
		tt.Frame()
		for _, label := range []string{"在浏览器中登录", "账号设置"} {
			if got := countText(tt.Texts(), label); got != 2 {
				t.Fatalf("signed-out popover button %q appeared %d times (want 2): %q", label, got, tt.Texts())
			}
		}

		// Signed-in popover buttons (including retry-groups in level-2 submenu on error).
		state.Stage = cheapauthui.Ready
		state.Account = core.User{Email: "guard@example.test", Username: "Guard", PlanName: "Pro", Balance: 5}
		state.GroupsError = "加载失败"
		state.RefreshGroups = func() {}
		a.accountGroupsOpen = true
		tt.Frame()
		for _, label := range []string{"账号详情", "退出登录", "重试加载分组"} {
			if got := countText(tt.Texts(), label); got != 2 {
				t.Fatalf("signed-in popover button %q appeared %d times (want 2): %q", label, got, tt.Texts())
			}
		}
	})
}
