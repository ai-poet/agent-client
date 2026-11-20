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

	// Close popover and transition to signed-in state with per-lane groups.
	a.accountOpen = false
	a.accountGroupLane = ""
	refreshed := 0
	type lanePick struct {
		lane string
		id   *int64
	}
	var selectedCalls []lanePick
	state.Stage = cheapauthui.Ready
	state.Account = core.User{Email: "dev@example.test", Username: "Dev", PlanName: "Pro", Balance: 12.5}
	state.ModelCount = 3
	state.LaneBindings = map[string]int64{core.LaneClaude: 101}
	state.Groups = []core.Group{
		{ID: 101, Name: "Claude 高速组", Platform: "anthropic", RateMultiplier: 0.5, Status: "active"},
		{ID: 202, Name: "OpenAI 订阅组", Platform: "openai", SubscriptionType: "subscription", Status: "active"},
	}
	state.RefreshGroups = func() { refreshed++ }
	state.SelectLaneGroup = func(lane string, id *int64) { selectedCalls = append(selectedCalls, lanePick{lane: lane, id: id}) }
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

	// 3. Clicking trigger opens level-1 popover with per-lane entries.
	tt.ClickAt(signedTrigger.Bounds.X+signedTrigger.Bounds.W/2, signedTrigger.Bounds.Y+signedTrigger.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if refreshed != 1 {
		t.Fatalf("opening signed-in popover called RefreshGroups %d times, want 1", refreshed)
	}
	for _, want := range []string{"Dev", "Pro", "余额 $12.50", "3 个模型", "Claude", "Claude 高速组", "Codex", "未绑定", "退出登录"} {
		if !tt.HasText(want) {
			t.Errorf("level-1 signed-in popover missing %q: %q", want, tt.Texts())
		}
	}
	if tt.HasText("OpenAI 订阅组") {
		t.Fatalf("groups should not be flattened in level-1 popover before expanding a lane: %q", tt.Texts())
	}
	shot(t, tt, "account-popover-level1-zh")

	// 4. Clicking Codex lane opens level-2 submenu for Codex groups.
	codexEntry, ok := tt.AccessByRole(ui.RoleButton, "Codex")
	if !ok {
		t.Fatalf("level-1 popover missing Codex lane trigger: %q", tt.Texts())
	}
	if codexEntry.Expanded {
		t.Fatal("Codex lane submenu trigger should start collapsed")
	}
	tt.ClickAt(codexEntry.Bounds.X+codexEntry.Bounds.W/2, codexEntry.Bounds.Y+codexEntry.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if a.accountGroupLane != core.LaneCodex {
		t.Fatal("clicking Codex did not open Codex level-2 submenu")
	}
	for _, want := range []string{"Codex 分组", "未绑定（留空）", "OpenAI 订阅组"} {
		if !tt.HasText(want) {
			t.Errorf("level-2 Codex submenu missing %q: %q", want, tt.Texts())
		}
	}
	shot(t, tt, "account-popover-groups-submenu-zh")

	// 5. Clicking a group in Codex submenu invokes SelectLaneGroup for openai and closes submenu.
	g2Btn, ok := tt.AccessByRole(ui.RoleButton, "OpenAI 订阅组")
	if !ok {
		t.Fatalf("Codex group row missing in submenu: %q", tt.Texts())
	}
	tt.ClickAt(g2Btn.Bounds.X+g2Btn.Bounds.W/2, g2Btn.Bounds.Y+g2Btn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if len(selectedCalls) != 1 || selectedCalls[0].lane != core.LaneCodex || selectedCalls[0].id == nil || *selectedCalls[0].id != 202 {
		t.Fatalf("SelectLaneGroup calls = %+v, want [(openai, 202)]", selectedCalls)
	}
	if a.accountGroupLane != "" {
		t.Fatal("selecting a group did not close level-2 submenu")
	}

	// 6. Group switch error shows in level-1 panel while preserving Claude's bound group.
	state.GroupError = "切换分组失败：无法连接 CheapRouter，请检查网络或切换主站/CDN 后重试"
	tt.Frame()
	if !tt.HasText(state.GroupError) {
		t.Fatalf("popover did not show group error: %q", tt.Texts())
	}
	claudeEntry, ok := tt.AccessByRole(ui.RoleButton, "Claude")
	if !ok {
		t.Fatalf("Claude trigger missing after error: %q", tt.Texts())
	}
	tt.ClickAt(claudeEntry.Bounds.X+claudeEntry.Bounds.W/2, claudeEntry.Bounds.Y+claudeEntry.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	g1Btn, ok := tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	if !ok || !g1Btn.Checked {
		t.Fatalf("previous active group lost checked state after error: found=%v checked=%v", ok, g1Btn.Checked)
	}
	shot(t, tt, "account-popover-group-error-zh")

	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if !a.accountOpen || a.accountGroupLane != "" {
		t.Fatal("first Escape should dismiss only the lane submenu")
	}
	tt.Key(0, ui.KeyRight)
	tt.Frame()
	if a.accountGroupLane != core.LaneClaude {
		t.Fatal("Right on the lane trigger should reopen the submenu")
	}

	const width, height = 540, 550
	tt.SetSize(width, height)
	tt.Frame()
	g1Btn, ok = tt.AccessByRole(ui.RoleButton, "Claude 高速组")
	bounds := g1Btn.Bounds
	if !ok || bounds.X < 0 || bounds.Y < 0 || bounds.W <= 0 || bounds.H <= 0 || bounds.X+bounds.W > width || bounds.Y+bounds.H > height {
		t.Fatalf("compact submenu row is outside the window: found=%v bounds=%+v", ok, bounds)
	}
	shot(t, tt, "account-popover-groups-compact-zh")
	tt.Key(0, ui.KeyEscape)
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if a.accountOpen || a.accountGroupLane != "" {
		t.Fatal("second Escape should dismiss the account popover")
	}
}
