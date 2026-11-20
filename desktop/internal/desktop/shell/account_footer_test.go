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
	a.accountGroupsOpen = false
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

	// 3. Clicking trigger opens level-1 popover: groups are NOT flattened; a level-1 "分组" entry is shown.
	tt.ClickAt(signedTrigger.Bounds.X+signedTrigger.Bounds.W/2, signedTrigger.Bounds.Y+signedTrigger.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if refreshed != 1 {
		t.Fatalf("opening signed-in popover called RefreshGroups %d times, want 1", refreshed)
	}
	for _, want := range []string{"Dev", "Pro", "余额 $12.50", "3 个模型", "分组", "Claude 高速组", "退出登录"} {
		if !tt.HasText(want) {
			t.Errorf("level-1 signed-in popover missing %q: %q", want, tt.Texts())
		}
	}
	if tt.HasText("OpenAI 订阅组") || tt.HasText("默认分组（自动）") {
		t.Fatalf("groups should not be flattened in level-1 popover before expanding 分组: %q", tt.Texts())
	}
	shot(t, tt, "account-popover-level1-zh")

	// 4. Clicking level-1 "分组" expands level-2 group submenu with available groups and active check mark.
	groupEntry, ok := tt.AccessByRole(ui.RoleButton, "分组")
	if !ok {
		t.Fatalf("level-1 popover missing 分组 submenu trigger: %q", tt.Texts())
	}
	if groupEntry.Expanded {
		t.Fatal("分组 submenu trigger should start collapsed")
	}
	tt.ClickAt(groupEntry.Bounds.X+groupEntry.Bounds.W/2, groupEntry.Bounds.Y+groupEntry.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if !a.accountGroupsOpen {
		t.Fatal("clicking 分组 did not open level-2 group submenu")
	}
	for _, want := range []string{"模型分组", "默认分组（自动）", "Claude 高速组", "OpenAI 订阅组"} {
		if !tt.HasText(want) {
			t.Errorf("level-2 group submenu missing %q: %q", want, tt.Texts())
		}
	}
	g1Btn, ok := tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	if !ok || !g1Btn.Checked {
		t.Fatalf("active group row not checked in level-2 submenu: found=%v checked=%v", ok, g1Btn.Checked)
	}
	shot(t, tt, "account-popover-groups-submenu-zh")

	// 5. Clicking another group in level-2 submenu invokes SelectGroup with its ID and closes the submenu.
	g2Btn, ok := tt.AccessByRole(ui.RoleButton, "OpenAI 订阅组")
	if !ok {
		t.Fatalf("second group row missing in submenu: %q", tt.Texts())
	}
	tt.ClickAt(g2Btn.Bounds.X+g2Btn.Bounds.W/2, g2Btn.Bounds.Y+g2Btn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if len(selectedCalls) != 1 || selectedCalls[0] == nil || *selectedCalls[0] != 202 {
		t.Fatalf("SelectGroup calls = %+v, want [202]", selectedCalls)
	}
	if a.accountGroupsOpen {
		t.Fatal("selecting a group did not close level-2 submenu")
	}

	// 6. Group switch error shows in level-1 panel while preserving current active group selection.
	state.GroupError = "切换分组失败：无法连接 CheapRouter，请检查网络或切换主站/CDN 后重试"
	tt.Frame()
	if !tt.HasText(state.GroupError) {
		t.Fatalf("popover did not show group error: %q", tt.Texts())
	}
	// Re-open level-2 submenu to confirm previous active group stays checked after failure rollback.
	groupEntry, ok = tt.AccessByRole(ui.RoleButton, "分组")
	if !ok {
		t.Fatalf("分组 trigger missing after error: %q", tt.Texts())
	}
	tt.ClickAt(groupEntry.Bounds.X+groupEntry.Bounds.W/2, groupEntry.Bounds.Y+groupEntry.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	g1Btn, ok = tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	if !ok || !g1Btn.Checked {
		t.Fatalf("previous active group lost checked state after error: found=%v checked=%v", ok, g1Btn.Checked)
	}
	shot(t, tt, "account-popover-group-error-zh")

	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if !a.accountOpen || a.accountGroupsOpen {
		t.Fatal("first Escape should dismiss only the group submenu")
	}
	tt.Key(0, ui.KeyRight)
	tt.Frame()
	if !a.accountGroupsOpen {
		t.Fatal("Right on the group trigger should reopen the submenu")
	}

	const width, height = 540, 550
	tt.SetSize(width, height)
	tt.Frame()
	g2Btn, ok = tt.AccessByRole(ui.RoleButton, "OpenAI 订阅组")
	bounds := g2Btn.Bounds
	if !ok || bounds.X < 0 || bounds.Y < 0 || bounds.W <= 0 || bounds.H <= 0 || bounds.X+bounds.W > width || bounds.Y+bounds.H > height {
		t.Fatalf("compact submenu row is outside the window: found=%v bounds=%+v", ok, bounds)
	}
	shot(t, tt, "account-popover-groups-compact-zh")
	tt.Key(0, ui.KeyEscape)
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if a.accountOpen || a.accountGroupsOpen {
		t.Fatal("second Escape should dismiss the account popover")
	}
}
