package shell

import (
	"testing"
	"time"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func TestSidebarAccountFooterSignedOutAndSignedInPopover(t *testing.T) {
	data := &viewmodel.Data{
		Workspace: &viewmodel.Workspace{
			Hosts: []viewmodel.Host{{ID: "local", Name: "local_host_name", Platform: "macos", Local: true}},
		},
		Projects: []*viewmodel.Project{{
			ID: "proj-1", Host: "local", Name: "my-workspace", Path: "/tmp/my-workspace", Branch: "main",
			Conversations: []*viewmodel.Conversation{{ID: "conv-1", Title: "Demo session"}},
		}},
	}
	a := NewLive(data)
	a.Sidebar.Selected = "conv-1"
	state := cheapauthui.NewState()
	a.Settings.CheapRouter = state

	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()

	// 1. Signed-out footer button renders "登录 CheapRouter".
	trigger, ok := tt.AccessByRole(ui.RoleButton, "登录 CheapRouter")
	if !ok {
		t.Fatalf("signed-out sidebar footer missing 登录 CheapRouter button: %q", tt.Texts())
	}
	shot(t, tt, "account-footer-signed-out-zh")

	// Clicking opens the signed-out popover panel.
	tt.ClickAt(trigger.Bounds.X+trigger.Bounds.W/2, trigger.Bounds.Y+trigger.Bounds.H/2)
	tt.Frame()
	tt.Advance(250 * time.Millisecond)
	tt.Frame()
	if !tt.HasText("CheapRouter 账号") || !tt.HasText("在浏览器中登录") {
		t.Fatalf("signed-out popover missing expected content: %q", tt.Texts())
	}
	shot(t, tt, "account-popover-signed-out-zh")

	// Close popover and transition to signed-in state with groups.
	a.accountOpen = false
	refreshed := 0
	var selectedCalls []*int64
	group1 := int64(101)
	state.Stage = cheapauthui.Ready
	state.Account = core.User{Email: "dev@example.test", Username: "Dev", PlanName: "Pro", Balance: 12.5}
	state.ModelCount = 3
	state.SelectedGroupID = &group1
	state.Groups = []core.Group{
		{ID: 101, Name: "Claude 高速组", Platform: "anthropic", RateMultiplier: 0.5, Status: "active"},
		{ID: 202, Name: "OpenAI 订阅组", Platform: "openai", SubscriptionType: "subscription", Status: "active"},
	}
	state.RefreshGroups = func() { refreshed++ }
	state.SelectGroup = func(id *int64) { selectedCalls = append(selectedCalls, id) }
	tt.Frame()

	// 2. Signed-in footer shows avatar initial, email, and plan/group summary.
	signedTrigger, ok := tt.AccessByRole(ui.RoleButton, "dev@example.test")
	if !ok {
		t.Fatalf("signed-in sidebar footer missing email button: %q", tt.Texts())
	}
	if !tt.HasText("Pro · Claude 高速组") {
		t.Fatalf("signed-in sidebar footer missing plan/group label: %q", tt.Texts())
	}
	shot(t, tt, "account-footer-signed-in-zh")

	// 3. Clicking trigger opens popover, refreshes groups, and displays account info + group list.
	tt.ClickAt(signedTrigger.Bounds.X+signedTrigger.Bounds.W/2, signedTrigger.Bounds.Y+signedTrigger.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if refreshed != 1 {
		t.Fatalf("opening signed-in popover called RefreshGroups %d times, want 1", refreshed)
	}
	for _, want := range []string{"Dev", "Pro", "余额 $12.50", "3 个模型", "模型分组", "默认分组（自动）", "Claude 高速组", "OpenAI 订阅组", "退出登录"} {
		if !tt.HasText(want) {
			t.Errorf("signed-in popover missing %q: %q", want, tt.Texts())
		}
	}
	g1Btn, ok := tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	if !ok || !g1Btn.Checked {
		t.Fatalf("active group row not checked: found=%v checked=%v", ok, g1Btn.Checked)
	}
	shot(t, tt, "account-popover-groups-zh")

	// 4. Clicking another group invokes SelectGroup with its ID.
	g2Btn, ok := tt.AccessByRole(ui.RoleButton, "OpenAI 订阅组")
	if !ok {
		t.Fatalf("second group row missing: %q", tt.Texts())
	}
	tt.ClickAt(g2Btn.Bounds.X+g2Btn.Bounds.W/2, g2Btn.Bounds.Y+g2Btn.Bounds.H/2)
	tt.Frame()
	if len(selectedCalls) != 1 || selectedCalls[0] == nil || *selectedCalls[0] != 202 {
		t.Fatalf("SelectGroup calls = %+v, want [202]", selectedCalls)
	}

	// 5. Group switch error shows clearly while preserving current active group selection.
	state.GroupError = "切换分组失败：无法连接 CheapRouter，请检查网络或切换主站/CDN 后重试"
	tt.Frame()
	if !tt.HasText(state.GroupError) {
		t.Fatalf("popover did not show group error: %q", tt.Texts())
	}
	g1Btn, ok = tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	if !ok || !g1Btn.Checked {
		t.Fatalf("previous active group lost checked state after error: found=%v checked=%v", ok, g1Btn.Checked)
	}
	shot(t, tt, "account-popover-group-error-zh")
}
