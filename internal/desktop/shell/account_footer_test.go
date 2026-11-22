package shell

import (
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
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
	for _, want := range []string{"Dev", "Pro", "余额 $12.50", "充值", "套餐", "3 个模型", "Claude", "Claude 高速组", "Codex", "未绑定", "退出登录"} {
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

func TestSidebarAccountPopoverOpensRechargeAndPlansDirectly(t *testing.T) {
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
	state.Stage = cheapauthui.Ready
	state.Account = core.User{Email: "dev@example.test", Username: "Dev", PlanName: "Pro", Balance: 18.5}
	state.ModelCount = 2
	a.Settings.CheapRouter = state

	rechargeOpens, planOpens := 0, 0
	state.OpenRecharge = func() {
		rechargeOpens++
		d := &cheapauthui.RechargeDialog{
			Amount: "10",
			Config: &core.PayConfig{
				MinAmount: 5, MaxAmount: 200, MaxPendingOrders: 3,
				EnabledPaymentTypes: []string{"alipay", "wxpay"},
			},
			Submit: func(float64, string) {},
		}
		d.Close = func() { state.Recharge = nil }
		state.Recharge = d
	}
	state.OpenPlans = func() {
		planOpens++
		d := &cheapauthui.RechargeDialog{
			Listing: true,
			Plans:   []core.SubscriptionPlan{{ID: "pro-m", Name: "Pro Monthly", Price: 29, ValidityDays: 1, ValidityUnit: "month"}},
		}
		d.Close = func() { state.Recharge = nil }
		state.Recharge = d
	}

	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()

	openPopover := func() {
		trigger, ok := tt.AccessByRole(ui.RoleButton, "dev@example.test")
		if !ok {
			t.Fatalf("missing signed-in footer trigger: %q", tt.Texts())
		}
		tt.ClickAt(trigger.Bounds.X+trigger.Bounds.W/2, trigger.Bounds.Y+trigger.Bounds.H/2)
		tt.Frame()
		tt.Frame()
	}

	// 1. Click "充值" inside avatar popover -> opens recharge dialog directly without leaving ConversationPage.
	openPopover()
	rechargeBtn, ok := tt.AccessByRole(ui.RoleButton, "充值")
	if !ok || rechargeBtn.Disabled {
		t.Fatalf("popover recharge button unavailable: found=%v disabled=%v texts=%q", ok, rechargeBtn.Disabled, tt.Texts())
	}
	tt.ClickAt(rechargeBtn.Bounds.X+rechargeBtn.Bounds.W/2, rechargeBtn.Bounds.Y+rechargeBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if rechargeOpens != 1 || a.accountOpen || a.Page != ConversationPage {
		t.Fatalf("expected direct recharge dialog on ConversationPage: opens=%d accountOpen=%v page=%v", rechargeOpens, a.accountOpen, a.Page)
	}
	dlg, ok := tt.AccessByRole(ui.RoleDialog, "余额充值")
	if !ok || dlg.Bounds.W < 360 || dlg.Bounds.W > 420 {
		t.Fatalf("recharge dialog not shown or width out of 360..420: found=%v bounds=%+v", ok, dlg.Bounds)
	}
	for _, want := range []string{"预计到账 $10.00", "支付 $10.00", "取消"} {
		if !tt.HasText(want) {
			t.Errorf("recharge dialog missing %q: %q", want, tt.Texts())
		}
	}

	cancelBtn, ok := tt.AccessByRole(ui.RoleButton, "取消")
	if !ok {
		t.Fatalf("missing Cancel button in recharge dialog: %q", tt.Texts())
	}
	tt.ClickAt(cancelBtn.Bounds.X+cancelBtn.Bounds.W/2, cancelBtn.Bounds.Y+cancelBtn.Bounds.H/2)
	tt.Frame()
	if state.Recharge != nil {
		t.Fatal("Cancel did not close recharge dialog")
	}

	// 2. Click "套餐" inside avatar popover -> opens plans dialog directly without leaving ConversationPage.
	openPopover()
	plansBtn, ok := tt.AccessByRole(ui.RoleButton, "套餐")
	if !ok || plansBtn.Disabled {
		t.Fatalf("popover plans button unavailable: found=%v disabled=%v", ok, plansBtn.Disabled)
	}
	tt.ClickAt(plansBtn.Bounds.X+plansBtn.Bounds.W/2, plansBtn.Bounds.Y+plansBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if planOpens != 1 || a.accountOpen || a.Page != ConversationPage {
		t.Fatalf("expected direct plans dialog on ConversationPage: opens=%d accountOpen=%v page=%v", planOpens, a.accountOpen, a.Page)
	}
	if !tt.HasText("Pro Monthly") {
		t.Fatalf("plans dialog missing plan item: %q", tt.Texts())
	}
}

func TestSidebarAccountPopoverGatewayOriginSubmenu(t *testing.T) {
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
	state.Stage = cheapauthui.Ready
	state.Account = core.User{Email: "dev@example.test", Username: "Dev", PlanName: "Pro", Balance: 15}
	state.ModelCount = 2
	state.OriginResults = []core.CandidateResult{
		{URL: core.PrimaryEndpoint, Result: &core.ProbeResult{LatencyMS: 210, Status: 200, Verdict: core.ProbeOK}},
		{URL: core.CDNEndpoint, Result: &core.ProbeResult{LatencyMS: 65, Status: 200, Verdict: core.ProbeOK}},
	}
	var selected []string
	tested := 0
	state.SelectOrigin = func(origin string) { selected = append(selected, origin) }
	state.TestOrigins = func() { tested++ }
	state.SetOriginAuto = func(on bool) { state.OriginConfig.AutoSelect = on }
	a.Settings.CheapRouter = state

	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()

	trigger, ok := tt.AccessByRole(ui.RoleButton, "dev@example.test")
	if !ok {
		t.Fatalf("missing signed-in trigger: %q", tt.Texts())
	}
	tt.ClickAt(trigger.Bounds.X+trigger.Bounds.W/2, trigger.Bounds.Y+trigger.Bounds.H/2)
	tt.Frame()
	tt.Frame()

	originBtn, ok := tt.AccessByRole(ui.RoleButton, "API 节点")
	if !ok {
		t.Fatalf("signed-in popover missing API 节点 row: %q", tt.Texts())
	}
	tt.ClickAt(originBtn.Bounds.X+originBtn.Bounds.W/2, originBtn.Bounds.Y+originBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if !a.accountNodesOpen {
		t.Fatal("clicking API 节点 did not open node submenu")
	}
	for _, want := range []string{"云端 API 节点", "CDN 加速节点", "65 ms", "主站（默认）", "210 ms", "全部测速", "管理节点…"} {
		if !tt.HasText(want) {
			t.Errorf("origin submenu missing %q: %q", want, tt.Texts())
		}
	}

	// Click "全部测速" button in submenu.
	testBtn, ok := tt.AccessByRole(ui.RoleButton, "全部测速")
	if !ok {
		t.Fatalf("origin submenu missing 全部测速 button: %q", tt.Texts())
	}
	tt.ClickAt(testBtn.Bounds.X+testBtn.Bounds.W/2, testBtn.Bounds.Y+testBtn.Bounds.H/2)
	tt.Frame()
	if tested != 1 {
		t.Fatalf("TestOrigins calls = %d, want 1", tested)
	}

	// Select CDN node from submenu -> invokes SelectOrigin and closes submenu.
	cdnBtn, ok := tt.AccessByRole(ui.RoleButton, "CDN 加速节点")
	if !ok {
		t.Fatalf("origin submenu missing CDN node row: %q", tt.Texts())
	}
	tt.ClickAt(cdnBtn.Bounds.X+cdnBtn.Bounds.W/2, cdnBtn.Bounds.Y+cdnBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	if len(selected) != 1 || selected[0] != core.CDNEndpoint || a.accountNodesOpen {
		t.Fatalf("selected = %v, accountNodesOpen = %v", selected, a.accountNodesOpen)
	}

	// Reopen submenu and click "管理节点…" -> opens Settings → CheapRouter 账号.
	tt.ClickAt(originBtn.Bounds.X+originBtn.Bounds.W/2, originBtn.Bounds.Y+originBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()
	manageBtn, ok := tt.AccessByRole(ui.RoleButton, "管理节点…")
	if !ok {
		t.Fatalf("origin submenu missing 管理节点… button: %q", tt.Texts())
	}
	tt.ClickAt(manageBtn.Bounds.X+manageBtn.Bounds.W/2, manageBtn.Bounds.Y+manageBtn.Bounds.H/2)
	tt.Frame()
	if a.Page != SettingsPage || a.accountOpen || a.accountNodesOpen {
		t.Fatalf("管理节点… did not open SettingsPage cleanly: page=%v open=%v nodes=%v", a.Page, a.accountOpen, a.accountNodesOpen)
	}
}
