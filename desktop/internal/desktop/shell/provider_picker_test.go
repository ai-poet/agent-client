package shell

import (
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preview"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/egoist/mygo/ui"
)

func TestCheapRouterFeaturedFirstInWelcomeAndAddChannel(t *testing.T) {
	assertLoginOrder := func(t *testing.T, label string, tt *ui.Tester) ui.AccessInfo {
		t.Helper()
		loginHeading, ok := tt.Find(i18n.In("zh-CN", "provider_access_login"))
		if !ok {
			t.Fatalf("%s: missing login heading: %q", label, tt.Texts())
		}
		keyHeading, ok := tt.Find(i18n.In("zh-CN", "provider_access_key"))
		if !ok {
			t.Fatalf("%s: missing API key heading: %q", label, tt.Texts())
		}

		cheap, ok := tt.AccessByRole(ui.RoleButton, "CheapRouter")
		if !ok {
			t.Fatalf("%s: CheapRouter button missing: %q", label, tt.Texts())
		}
		chatgpt, ok := tt.AccessByRole(ui.RoleButton, "ChatGPT")
		if !ok {
			t.Fatalf("%s: ChatGPT button missing: %q", label, tt.Texts())
		}
		claude, ok := tt.AccessByRole(ui.RoleButton, "Claude")
		if !ok {
			t.Fatalf("%s: Claude button missing: %q", label, tt.Texts())
		}

		for name, info := range map[string]ui.AccessInfo{"CheapRouter": cheap, "ChatGPT": chatgpt, "Claude": claude} {
			if info.Bounds.Y <= loginHeading.Y || info.Bounds.Y >= keyHeading.Y {
				t.Errorf("%s: %s at %v is outside login group (%v .. %v)", label, name, info.Bounds, loginHeading.Y, keyHeading.Y)
			}
		}
		if cheap.Bounds.Y > chatgpt.Bounds.Y || (cheap.Bounds.Y == chatgpt.Bounds.Y && cheap.Bounds.X >= chatgpt.Bounds.X) {
			t.Errorf("%s: CheapRouter (%v) must come before ChatGPT (%v)", label, cheap.Bounds, chatgpt.Bounds)
		}
		if cheap.Bounds.Y > claude.Bounds.Y || (cheap.Bounds.Y == claude.Bounds.Y && cheap.Bounds.X >= claude.Bounds.X) {
			t.Errorf("%s: CheapRouter (%v) must come before Claude (%v)", label, cheap.Bounds, claude.Bounds)
		}
		if !cheap.Checked {
			t.Errorf("%s: CheapRouter must be default selected/highlighted (Checked=true)", label)
		}
		if chatgpt.Checked || claude.Checked {
			t.Errorf("%s: non-featured providers must not be Checked: chatgpt=%v claude=%v", label, chatgpt.Checked, claude.Checked)
		}
		return cheap
	}

	// 1. Welcome guide picker.
	a := New(preview.Load())
	a.Welcome.Open = true
	a.Welcome.Seen = false
	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()
	tt.Frame()

	cheap := assertLoginOrder(t, "welcome guide", tt)
	shot(t, tt, "picker-welcome-zh")

	tt.ClickAt(cheap.Bounds.X+cheap.Bounds.W/2, cheap.Bounds.Y+cheap.Bounds.H/2)
	tt.Frame()
	if a.Welcome.Open || !a.Welcome.Seen || a.Page != SettingsPage || a.Settings.Section != settings.CheapAccount {
		t.Fatalf("clicking CheapRouter in welcome guide: welcome=%+v page=%v section=%v", a.Welcome, a.Page, a.Settings.Section)
	}

	// 2. Settings -> Providers -> Add Channel dialog.
	a.Settings.Open(settings.Providers)
	tt.Frame()
	addBtn, ok := tt.AccessByRole(ui.RoleButton, i18n.In("zh-CN", "settings_add"))
	if !ok {
		t.Fatalf("add channel button missing: %q", tt.Texts())
	}
	tt.ClickAt(addBtn.Bounds.X+addBtn.Bounds.W/2, addBtn.Bounds.Y+addBtn.Bounds.H/2)
	tt.Frame()
	tt.Frame()

	cheap = assertLoginOrder(t, "add channel dialog", tt)
	shot(t, tt, "picker-add-channel-zh")

	tt.ClickAt(cheap.Bounds.X+cheap.Bounds.W/2, cheap.Bounds.Y+cheap.Bounds.H/2)
	tt.Frame()
	if a.Settings.Section != settings.CheapAccount {
		t.Fatalf("clicking CheapRouter in add channel dialog did not open CheapAccount: section=%v", a.Settings.Section)
	}
}
