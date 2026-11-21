package shell

import (
	"reflect"
	"testing"

	cheapauthui "github.com/ai-poet/agent-client/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/egoist/mygo/ui"
)

// A CheapRouter account is optional: no stage of it hides the workspace, and
// the sign-in is a page of Settings.
func TestAccountNeverHidesWorkspace(t *testing.T) {
	const signIn, transcript = "在浏览器中登录", "Virtualized transcript stress test"
	for name, state := range map[string]*cheapauthui.State{
		"signed out":     {Stage: cheapauthui.SignedOut},
		"session ended":  {Stage: cheapauthui.SignedOut, Failure: "登录状态已失效，请重新登录"},
		"starting":       {Stage: cheapauthui.Starting},
		"waiting":        {Stage: cheapauthui.WaitingForCode},
		"gateway failed": {Stage: cheapauthui.GatewayFailed, Failure: "无法连接 CheapRouter"},
	} {
		t.Run(name, func(t *testing.T) {
			a, tt := newTester(t)
			a.Settings.CheapRouter = state
			tt.Frame()
			if !tt.HasText(transcript) || tt.HasText(signIn) || state.Failure != "" && tt.HasText(state.Failure) {
				t.Fatalf("the account took the workspace's place: %q", tt.Texts())
			}
			if _, ok := tt.Access(i18n.In("en", "composer")); !ok {
				t.Fatal("the composer is not reachable")
			}
			click(t, tt, i18n.In("en", "settings"))
			a.Settings.Open(settings.CheapAccount)
			tt.Frame()
			if a.Page != SettingsPage || !tt.HasText(i18n.In("en", "cheaprouter_account_description")) {
				t.Fatalf("Settings does not show the account page: %q", tt.Texts())
			}
			if state.Stage == cheapauthui.SignedOut && !(tt.HasText(i18n.In("en", "cheaprouter_signed_out")) && tt.HasText(signIn)) {
				t.Fatalf("the signed-out account page offers no sign-in: %q", tt.Texts())
			}
			if state.Failure != "" && !tt.HasText(state.Failure) {
				t.Fatalf("the account page does not report %q: %q", state.Failure, tt.Texts())
			}
		})
	}
}

// The shell offers its embedders no seam that takes the workspace's place:
// whatever a frame's yes-or-no hooks answer, the conversation is drawn. An
// account gate was once such a hook, installed by the composition root.
func TestHooksCannotHideWorkspace(t *testing.T) {
	a, tt := newTester(t)
	hooks := reflect.ValueOf(&a.Hooks).Elem()
	claimed := 0
	for i := range hooks.NumField() {
		if field := hooks.Field(i); field.Type() == reflect.TypeFor[func(*ui.Context) bool]() {
			field.Set(reflect.ValueOf(func(*ui.Context) bool { return true }))
			claimed++
		}
	}
	tt.Frame()
	if claimed == 0 || !tt.HasText("Virtualized transcript stress test") {
		t.Fatalf("%d hooks answering yes hid the workspace: %q", claimed, tt.Texts())
	}
	if _, ok := tt.Access(i18n.In("en", "composer")); !ok {
		t.Fatal("the composer is not reachable")
	}
}
