package settings

import (
	"path/filepath"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preview"
	clientupdate "github.com/ai-poet/agent-client/desktop/internal/update"
	"github.com/egoist/mygo/ui"
)

func TestNativeUpdateSettings(t *testing.T) {
	m := NewModel(preview.SettingsData())
	m.Updater = Updater{Native: true, Configured: true, NativeEnabled: true}
	var saved [][2]bool
	path := filepath.Join(t.TempDir(), clientupdate.ConfigName)
	store, err := clientupdate.OpenStore(path)
	if err != nil {
		t.Fatal(err)
	}
	m.UpdateActions = &UpdateActions{SaveOptions: func(automatic, prereleases bool) {
		saved = append(saved, [2]bool{automatic, prereleases})
		config := store.Config()
		config.Automatic, config.Prereleases = automatic, prereleases
		if err := store.Save(config); err != nil {
			t.Errorf("save update config: %v", err)
		}
	}}
	tester := ui.NewTesterWith(func(c *ui.Context) { updates(c, m) }, 900, 700, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("en")
	tester.Frame()
	toggle := func(key string) {
		t.Helper()
		name := i18n.In("en", key)
		control, ok := tester.AccessByRole(ui.RoleSwitch, name)
		if !ok {
			t.Fatalf("no %q switch: %q", name, tester.Texts())
		}
		tester.ClickAt(control.Bounds.X+control.Bounds.W/2, control.Bounds.Y+control.Bounds.H/2)
		tester.Frame()
		tester.Advance(time.Second)
	}
	toggle("updates_automatic")
	toggle("updates_prereleases")
	if len(saved) != 2 || saved[0] != [2]bool{true, false} || saved[1] != [2]bool{true, true} {
		t.Fatalf("saved switches = %v", saved)
	}
	if !m.Updater.Automatic || !m.Updater.Prereleases {
		t.Fatalf("switch state = %+v", m.Updater)
	}
	reopened, err := clientupdate.OpenStore(path)
	if err != nil || !reopened.Config().Automatic || !reopened.Config().Prereleases {
		t.Fatalf("saved config = %+v, %v", reopened.Config(), err)
	}

	m.Updater.Failure = &UpdateFailure{Key: "updates_not_found", Detail: "The update server returned HTTP 404"}
	tester.Frame()
	if !tester.HasText(i18n.In("en", "updates_not_found")) || !tester.HasText("The update server returned HTTP 404") {
		t.Fatalf("failure is not readable: %q", tester.Texts())
	}
	m.Updater.Failure, m.Updater.Current = nil, true
	tester.Frame()
	if !tester.HasText(i18n.In("en", "updates_current")) {
		t.Fatalf("up-to-date result is not shown: %q", tester.Texts())
	}
}
