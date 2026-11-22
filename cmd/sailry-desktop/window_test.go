package main

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preferences"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node"
	"github.com/egoist/mygo/ui"
)

// The window the desktop composes over a new profile opens on the workspace:
// without credentials there is no sign-in in its place, only the account
// page of Settings.
func TestLiveWindowOpensWithoutAccount(t *testing.T) {
	home := t.TempDir()
	// The legacy client's data is looked up below the home directory.
	for _, name := range []string{"HOME", "USERPROFILE", "LOCALAPPDATA", "XDG_DATA_HOME"} {
		t.Setenv(name, home)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	profile := filepath.Join(t.TempDir(), "profile")
	backend := &desktopBackend{Path: profile, Preferences: preferences.ForStartup(profile)}
	var err error
	if backend.Node, err = node.StartWithOptions(ctx, profile, node.Options{}); err != nil {
		t.Fatal(err)
	}
	defer backend.Node.Stop(context.Background())
	backend.Migration = newDesktopMigration(backend.Node, profile)
	backend.Client = client.New(link.NewLocal(backend.Node.ID(), backend.Node.ID(), backend.Node))
	if err := backend.Client.Start(ctx); err != nil {
		t.Fatal(err)
	}
	defer backend.Client.Close()
	runtime, err := liveWindow(ctx, backend, true)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	tester := ui.NewTesterWith(runtime.App.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	text := func(key string) string { return i18n.In("zh-CN", key) }
	const signIn = "在浏览器中登录"
	tester.Frame()
	// A first launch shows the welcome guide over the workspace; closing it
	// leaves the workspace to work in.
	if !runtime.App.Welcome.Open || tester.HasText(signIn) {
		t.Fatalf("the first launch does not open on the welcome guide: %q", tester.Texts())
	}
	if err := tester.Click(text("close")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	_, composer := tester.Access(text("composer"))
	_, rail := tester.AccessByRole(ui.RoleButton, text("settings"))
	if runtime.App.Page != shell.ConversationPage || !composer || !rail || tester.HasText(signIn) {
		t.Fatalf("the first window is not the workspace (page %v, composer %t, rail %t): %q", runtime.App.Page, composer, rail, tester.Texts())
	}
	if err := tester.Click(text("settings")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if err := tester.Click(text("cheaprouter_account")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText(text("cheaprouter_signed_out")) || !tester.HasText(signIn) {
		t.Fatalf("the account page does not offer the sign-in: %q", tester.Texts())
	}
}
