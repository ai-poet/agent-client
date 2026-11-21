package settings

import (
	"testing"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/preview"
	"github.com/egoist/mygo/ui"
)

func TestAboutUpstreamAttribution(t *testing.T) {
	const labelKey = "settings_about_based_on"
	const project = "sailry-harness"
	const url = "https://github.com/sailry/sailry-harness"
	for _, lang := range i18n.Languages {
		t.Run(lang, func(t *testing.T) {
			m := NewModel(preview.SettingsData())
			tester := ui.NewTesterWith(func(c *ui.Context) { about(c, m) }, 900, 700, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
			tester.SetLocale(lang)
			tester.Frame()

			label := i18n.In(lang, labelKey)
			if label == labelKey || (lang != "en" && label == i18n.In("en", labelKey)) || !tester.HasText(label) {
				t.Fatalf("localized attribution is missing: %q", tester.Texts())
			}
			if !tester.HasText(i18n.In(lang, "settings_license")) || !tester.HasText("Apache-2.0") {
				t.Fatalf("license attribution is missing: %q", tester.Texts())
			}
			link, ok := tester.AccessByRole(ui.RoleLink, project)
			if !ok || link.Disabled {
				t.Fatalf("no enabled link for %q: %q", project, tester.Texts())
			}
			if got := tester.OpenedURLs(); len(got) != 0 {
				t.Fatalf("rendering About opened URLs: %q", got)
			}
			tester.ClickAt(link.Bounds.X+link.Bounds.W/2, link.Bounds.Y+link.Bounds.H/2)
			if got := tester.OpenedURLs(); len(got) != 1 || got[0] != url {
				t.Fatalf("opened URLs = %q, want [%q]", got, url)
			}
		})
	}
}
