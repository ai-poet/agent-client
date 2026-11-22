package live

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	clientupdate "github.com/ai-poet/cheaprouter-harness/internal/update"
	"github.com/egoist/mygo/ui"
)

func TestNativeUpdatePromptAndManualReset(t *testing.T) {
	path := filepath.Join(t.TempDir(), clientupdate.ConfigName)
	start := func() *NativeUpdates {
		t.Helper()
		store, err := clientupdate.OpenStore(path)
		if err != nil {
			t.Fatal(err)
		}
		return &NativeUpdates{store: store, changed: func() {}}
	}
	n := start()
	// This fixture has no network or installer. Results below are injected
	// at the worker completion boundary.
	if err := n.store.Save(clientupdate.Config{}); err != nil {
		t.Fatal(err)
	}
	found := &clientupdate.Update{Version: "1.2.0", Notes: "Improved session recovery"}
	n.checkedLocked(true, found, nil)
	if n.prompt(nil, false) != nil {
		t.Fatal("unfocused window claimed prompt")
	}
	tester := ui.NewTesterWith(n.dialog(nil), 800, 600, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("en")
	tester.Frame()
	tester.Advance(time.Second)
	later, ok := tester.AccessByRole(ui.RoleButton, i18n.In("en", "updates_later"))
	_, install := tester.AccessByRole(ui.RoleButton, i18n.In("en", "updates_download_install"))
	if !ok || !install || !tester.HasText(i18n.In("en", "updates_prompt_title")) || !tester.HasText(found.Notes) || !tester.HasText(i18n.In("en", "updates_available", "version", found.Version)) {
		t.Fatalf("incomplete update prompt: %q", tester.Texts())
	}
	if n.prompt(nil, true) != nil {
		t.Fatal("second window claimed the same release")
	}
	tester.ClickAt(later.Bounds.X+later.Bounds.W/2, later.Bounds.Y+later.Bounds.H/2)
	tester.Frame()
	tester.Advance(time.Second)
	if tester.HasText(found.Notes) || n.state != settings.UpdateAvailable {
		t.Fatal("Later did not dismiss while leaving the update available")
	}
	n = start()
	n.checkedLocked(true, found, nil)
	if n.prompt(nil, true) != nil {
		t.Fatal("deferred version prompted again after reopening the profile")
	}
	newer := &clientupdate.Update{Version: "1.3.0"}
	n.checkedLocked(true, newer, nil)
	if n.prompt(nil, true) != newer {
		t.Fatal("deferral suppressed a different version")
	}
	n.install(found)
	if n.state != settings.UpdateAvailable {
		t.Fatal("stale prompt installed a different release")
	}
	n.Check() // disabled fixture; reset still precedes the manual check
	n.checkedLocked(false, found, nil)
	if n.prompt(nil, true) != nil || len(n.takeNotices()) != 0 || n.state != settings.UpdateAvailable {
		t.Fatal("manual result was not confined to About")
	}
	n = start()
	n.checkedLocked(true, found, nil)
	tester = ui.NewTesterWith(n.dialog(nil), 800, 600, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	tester.Frame()
	tester.Advance(time.Second)
	button, ok := tester.AccessByRole(ui.RoleButton, i18n.In("zh-CN", "updates_download_install"))
	if !ok {
		t.Fatal("manual check did not restore the automatic install prompt")
	}
	// Observe the existing install action before its worker starts. The
	// fixture has no native archive, so no real download or install follows.
	downloads := 0
	n.changed = func() {
		state, version, _, _, _, _ := n.snapshot()
		if state == settings.UpdateDownloading && version == found.Version {
			downloads++
		}
	}
	tester.ClickAt(button.Bounds.X+button.Bounds.W/2, button.Bounds.Y+button.Bounds.H/2)
	tester.Frame()
	n.workers.Wait()
	if downloads != 1 || tester.HasText(found.Notes) {
		t.Fatalf("install button started %d downloads; texts: %q", downloads, tester.Texts())
	}
}
