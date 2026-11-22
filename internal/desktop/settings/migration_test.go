package settings

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/migrate"
	"github.com/egoist/mygo/ui"
)

func TestMigrationFailureShownInGeneralSettingsAndRetryable(t *testing.T) {
	m := NewModel(preview.SettingsData())
	m.Migration = MigrationState{
		Known:         true,
		Found:         true,
		Status:        migrate.Failed,
		Failure:       "migration_failed",
		FailureDetail: "migration detect: count projects: file is not a database",
	}
	retried := false
	fillMissing := false
	m.MigrationActions = &MigrationActions{Run: func(fill bool) {
		retried = true
		fillMissing = fill
	}}
	tester := ui.NewTesterWith(func(c *ui.Context) { migration(c, m) }, 900, 700, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("en")
	tester.Frame()
	for _, want := range []string{
		i18n.In("en", "migration_failed"),
		i18n.In("en", "migration_failed_retry"),
		"file is not a database",
	} {
		if !tester.HasText(want) {
			t.Fatalf("missing %q in %q", want, tester.Texts())
		}
	}
	btn, ok := tester.AccessByRole(ui.RoleButton, i18n.In("en", "migration_retry"))
	if !ok || btn.Disabled {
		t.Fatalf("retry button = %+v, ok=%v", btn, ok)
	}
	tester.ClickAt(btn.Bounds.X+btn.Bounds.W/2, btn.Bounds.Y+btn.Bounds.H/2)
	tester.Frame()
	if !retried || !fillMissing {
		t.Fatalf("retry action not invoked with fillMissing=true: retried=%v fill=%v", retried, fillMissing)
	}
}
