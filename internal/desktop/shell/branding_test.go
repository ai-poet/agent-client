package shell

import (
	"slices"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/egoist/mygo/ui"
)

func TestBrandUnifiedAcrossLocalesAndUI(t *testing.T) {
	const attributionKey = "settings_about_based_on"
	const upstreamProject = "sailry-harness"
	for _, lang := range i18n.Languages {
		if got := i18n.In(lang, "app"); got != "CheapRouter" {
			t.Errorf("locale %s: app = %q, want CheapRouter", lang, got)
		}
		for _, key := range i18n.Keys(lang) {
			val := i18n.In(lang, key)
			if key != attributionKey && strings.Contains(strings.ToLower(val), "sailry") {
				t.Errorf("locale %s key %s still contains Sailry: %q", lang, key, val)
			}
		}
	}

	assertNoSailry := func(t *testing.T, label string, texts []string, allowed ...string) {
		t.Helper()
		for _, text := range texts {
			if strings.Contains(strings.ToLower(text), "sailry") && !slices.Contains(allowed, text) {
				t.Errorf("%s rendered unexpected Sailry text: %q", label, text)
			}
		}
	}

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
	a.Settings.Version = "0.4.0"
	a.Settings.Build = "release"
	a.Sidebar.Selected = "conv-1"
	tt := ui.NewTesterWith(a.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true, ReduceMotion: true}})
	tt.SetLocale("zh-CN")
	tt.Frame()
	if !tt.HasText("CheapRouter") {
		t.Fatalf("main workspace header missing CheapRouter brand: %q", tt.Texts())
	}
	assertNoSailry(t, "main workspace zh-CN", tt.Texts())
	shot(t, tt, "brand-main-zh")

	a.Do("app.about")
	tt.Frame()
	if a.Page != SettingsPage || a.Settings.Section != settings.About {
		t.Fatalf("app.about did not open Settings About: page=%v section=%v", a.Page, a.Settings.Section)
	}
	if !tt.HasText("CheapRouter") {
		t.Fatalf("About page missing CheapRouter brand: %q", tt.Texts())
	}
	assertNoSailry(t, "about page zh-CN", tt.Texts(), i18n.In("zh-CN", attributionKey), upstreamProject)
	shot(t, tt, "brand-about-zh")

	// The guide overlays Settings; the About attribution remains in the frame.
	a.Welcome.Open = true
	for range 10 {
		tt.Frame()
	}
	if !tt.HasText("欢迎使用 CheapRouter") {
		t.Fatalf("welcome guide missing CheapRouter title: %q", tt.Texts())
	}
	assertNoSailry(t, "welcome guide zh-CN", tt.Texts(), i18n.In("zh-CN", attributionKey), upstreamProject)
	shot(t, tt, "brand-welcome-zh")

	tt.SetLocale("en")
	tt.Frame()
	if !tt.HasText("Welcome to CheapRouter") {
		t.Fatalf("english welcome guide missing CheapRouter title: %q", tt.Texts())
	}
	assertNoSailry(t, "welcome guide en", tt.Texts(), i18n.In("en", attributionKey), upstreamProject)

	a.Welcome.Open = false
	a.Do("app.about")
	tt.Frame()
	if !tt.HasText("CheapRouter") {
		t.Fatalf("english About page missing CheapRouter brand: %q", tt.Texts())
	}
	assertNoSailry(t, "about page en", tt.Texts(), i18n.In("en", attributionKey), upstreamProject)
	shot(t, tt, "brand-about-en")

	// Also verify preview dataset has no user-visible Sailry branding.
	for _, theme := range preview.ExtensionsData().Themes {
		if strings.Contains(strings.ToLower(theme.Description), "sailry") {
			t.Errorf("preview theme %s still mentions Sailry: %q", theme.ID, theme.Description)
		}
	}
	for _, relay := range preview.SettingsData().Relays {
		if strings.Contains(strings.ToLower(relay), "sailry") {
			t.Errorf("preview relay still mentions Sailry: %q", relay)
		}
	}
}
