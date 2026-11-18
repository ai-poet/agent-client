package cheapauth

import (
	"image/png"
	"os"
	"path/filepath"
	"slices"
	"testing"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/egoist/mygo/ui"
)

func loginTester(state *State) *ui.Tester {
	tt := ui.NewTester(func(c *ui.Context) { Gate(c, state) }, 960, 720)
	tt.SetLocale("zh-CN")
	return tt
}

func loginControl(t *testing.T, tt *ui.Tester, role ui.Role, name string) ui.AccessInfo {
	t.Helper()
	info, ok := tt.AccessByRole(role, name)
	if !ok {
		t.Fatalf("missing control %q; texts: %q", name, tt.Texts())
	}
	return info
}

func loginClick(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	if err := tt.Click(name); err != nil {
		t.Fatal(err)
	}
}

func TestLoginEndpointsAndStartActions(t *testing.T) {
	for _, dark := range []bool{false, true} {
		t.Run(map[bool]string{false: "light", true: "dark"}[dark], func(t *testing.T) {
			state := NewState()
			var started []bool
			state.Start = func(manual bool) { started = append(started, manual) }
			tt := loginTester(state)
			tt.SetDark(dark)
			primary, cdn := "主站 · "+core.PrimaryEndpoint, "CDN · "+core.CDNEndpoint
			for _, name := range []string{primary, cdn, "在浏览器中登录", "使用粘贴登录码方式"} {
				if !tt.HasText(name) {
					t.Errorf("missing visible label %q", name)
				}
			}
			if !loginControl(t, tt, ui.RoleToggleButton, primary).Checked || loginControl(t, tt, ui.RoleToggleButton, cdn).Checked {
				t.Fatal("initial endpoint selection is incorrect")
			}
			tt.Key(0, ui.KeyTab)
			tt.Key(0, ui.KeyTab)
			if !tt.Focused(cdn) {
				t.Fatal("CDN endpoint is not keyboard reachable")
			}
			tt.Key(0, ui.KeySpace)
			if state.Endpoint != core.CDNEndpoint || !loginControl(t, tt, ui.RoleToggleButton, cdn).Checked {
				t.Fatal("keyboard selection did not select the CDN endpoint")
			}
			loginClick(t, tt, "在浏览器中登录")
			loginClick(t, tt, "使用粘贴登录码方式")
			if !slices.Equal(started, []bool{false, true}) {
				t.Fatalf("start callbacks = %v", started)
			}
			state.Stage = Starting
			tt.Frame()
			for _, name := range []string{primary, cdn, "在浏览器中登录", "使用粘贴登录码方式"} {
				info, ok := tt.Access(name)
				if !ok || !info.Disabled {
					t.Errorf("starting control %q is not disabled", name)
				}
			}
			if !tt.HasText("正在准备安全登录") {
				t.Fatal("starting status is missing")
			}
		})
	}
}

func TestPasteLoginSubmissionAndBrowserActions(t *testing.T) {
	state := NewState()
	state.Stage, state.LoginURL = WaitingForCode, "https://example.com/login"
	submissions, cancellations := 0, 0
	state.SubmitPaste = func() {
		submissions++
		state.Exchanging = true
	}
	state.Cancel = func() { cancellations++ }
	tt := loginTester(state)
	for _, pasted := range []string{"", " \t "} {
		state.Pasted = pasted
		tt.Frame()
		if !loginControl(t, tt, ui.RoleButton, "完成登录").Disabled {
			t.Fatalf("blank code %q enabled submission", pasted)
		}
		input := loginControl(t, tt, ui.RoleTextField, "一次性登录码").Bounds
		tt.ClickAt(input.X+input.W/2, input.Y+input.H/2)
		tt.Key(0, ui.KeyEnter)
		if submissions != 0 {
			t.Fatal("Enter submitted a blank code")
		}
	}
	state.Pasted = ""
	tt.Frame()
	tt.Type("K7QM-3XPD")
	if state.Pasted != "K7QM-3XPD" || loginControl(t, tt, ui.RoleButton, "完成登录").Disabled {
		t.Fatalf("typing a code did not enable submission: pasted=%q", state.Pasted)
	}
	tt.Key(0, ui.KeyEnter)
	if !loginControl(t, tt, ui.RoleButton, "完成登录").Disabled {
		t.Fatal("Enter left submission enabled while exchanging")
	}
	state.Exchanging = false
	tt.Frame()
	loginClick(t, tt, "完成登录")
	if submissions != 2 {
		t.Fatalf("Enter and click submitted %d times, want 2", submissions)
	}
	if !loginControl(t, tt, ui.RoleTextField, "一次性登录码").Disabled || !loginControl(t, tt, ui.RoleButton, "完成登录").Disabled {
		t.Fatal("exchange did not disable the input and submit button")
	}
	if !tt.HasText("正在兑换一次性登录码") {
		t.Fatal("exchange status is missing")
	}
	loginClick(t, tt, "重新打开登录页")
	loginClick(t, tt, "复制链接")
	if !slices.Equal(tt.OpenedURLs(), []string{state.LoginURL}) || tt.Clipboard() != state.LoginURL {
		t.Fatal("browser actions did not preserve the login URL")
	}
	loginClick(t, tt, "取消本次登录")
	if cancellations != 1 {
		t.Fatal("cancel did not invoke its callback")
	}
	state.Exchanging, state.Loopback = false, true
	tt.Frame()
	if !tt.HasText("完成登录后会自动返回应用") {
		t.Fatal("loopback guidance is missing")
	}
}

func TestLoginFailureRecoveryAndReadyGate(t *testing.T) {
	state := NewState()
	state.Stage, state.Failure = SyncingGateway, "连接模型网关失败"
	retries, cancellations := 0, 0
	state.Retry = func() { retries++ }
	state.Cancel = func() { cancellations++ }
	tt := loginTester(state)
	if !tt.HasText(state.Failure) || !tt.HasText("正在连接模型网关") {
		t.Fatal("gateway failure or synchronization status is missing")
	}
	loginClick(t, tt, "重试网关配置")
	loginClick(t, tt, "返回重新登录")
	if retries != 1 || cancellations != 1 {
		t.Fatal("gateway recovery callbacks changed")
	}
	state.Stage = Ready
	tt.Frame()
	if tt.HasText("登录 CheapRouter") {
		t.Fatal("ready account is still gated")
	}
}

func TestLoginSnapshots(t *testing.T) {
	dir := os.Getenv("SAILRY_LOGIN_SNAPSHOTS")
	if dir == "" {
		t.Skip("SAILRY_LOGIN_SNAPSHOTS is not set")
	}
	for _, shot := range []struct {
		name  string
		stage Stage
	}{{"mygo-login-lobe.png", SignedOut}, {"mygo-login-paste.png", WaitingForCode}} {
		state := NewState()
		state.Stage, state.LoginURL = shot.stage, "https://example.com/login"
		tt := loginTester(state)
		file, err := os.Create(filepath.Join(dir, shot.name))
		if err != nil {
			t.Fatal(err)
		}
		err = png.Encode(file, tt.Image())
		closeErr := file.Close()
		if err != nil {
			t.Fatal(err)
		}
		if closeErr != nil {
			t.Fatal(closeErr)
		}
	}
}
