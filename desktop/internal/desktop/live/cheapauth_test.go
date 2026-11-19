package live

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/client"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preferences"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/shell"
	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/node"
	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

const (
	cheapTestCode    = "K7QM3XPD"
	cheapTestAccess  = "access-fixture"
	cheapTestRefresh = "refresh-fixture"
	cheapTestKey     = "sk-gateway-fixture"
	cheapSignIn      = "在浏览器中登录"
)

// cheapGateway stands in for the CheapRouter service: the one-time-code
// exchange, the account, the catalog, the keys, renewal and revocation.
type cheapGateway struct {
	*httptest.Server
	mu sync.Mutex
	// down drops every connection, as an unreachable network does.
	down bool
	// rejected refuses the refresh token.
	rejected bool
	// challenge is the PKCE challenge the browser was sent with.
	challenge string
	// exchange, when set, holds the code exchange until it is closed.
	exchange chan struct{}
	calls    []string
	revoked  []string
}

func newCheapGateway(t *testing.T) *cheapGateway {
	gateway := &cheapGateway{}
	gateway.Server = httptest.NewServer(http.HandlerFunc(gateway.serve))
	t.Cleanup(gateway.Close)
	return gateway
}

func (g *cheapGateway) serve(writer http.ResponseWriter, request *http.Request) {
	route := request.Method + " " + strings.TrimPrefix(request.URL.Path, "/api/v1")
	g.mu.Lock()
	g.calls = append(g.calls, route)
	down, rejected, challenge, exchange := g.down, g.rejected, g.challenge, g.exchange
	g.mu.Unlock()
	if down {
		panic(http.ErrAbortHandler)
	}
	var body map[string]string
	if request.Method == http.MethodPost {
		_ = json.NewDecoder(request.Body).Decode(&body)
	}
	reply := func(status int, payload string) {
		writer.Header().Set("Content-Type", "application/json")
		writer.WriteHeader(status)
		_, _ = writer.Write([]byte(payload))
	}
	if strings.HasPrefix(route, "GET ") && request.Header.Get("Authorization") != "Bearer "+cheapTestAccess {
		reply(http.StatusUnauthorized, `{"code":401,"message":"unauthorized"}`)
		return
	}
	session := `{"code":0,"data":{"access_token":"` + cheapTestAccess + `","refresh_token":"` + cheapTestRefresh + `","expires_in":900,"api_key":"` + cheapTestKey + `"}}`
	switch route {
	case "POST /auth/desktop-session/exchange":
		if exchange != nil {
			<-exchange
		}
		if body["code"] != cheapTestCode || challenge == "" || core.PKCEChallenge(body["code_verifier"]) != challenge {
			reply(http.StatusBadRequest, `{"code":400,"reason":"DESKTOP_LOGIN_CODE_INVALID","message":"invalid code"}`)
			return
		}
		reply(http.StatusOK, session)
	case "POST /auth/refresh":
		if rejected || body["refresh_token"] != cheapTestRefresh {
			reply(http.StatusUnauthorized, `{"code":401,"reason":"REFRESH_TOKEN_INVALID","message":"invalid refresh token"}`)
			return
		}
		reply(http.StatusOK, session)
	case "POST /auth/logout":
		g.mu.Lock()
		g.revoked = append(g.revoked, body["refresh_token"])
		g.mu.Unlock()
		reply(http.StatusOK, `{"code":0,"data":{}}`)
	case "GET /auth/me":
		reply(http.StatusOK, `{"code":0,"data":{"id":7,"email":"dev@example.test","username":"Dev","plan":"Pro","balance":12.5}}`)
	case "GET /models/catalog":
		reply(http.StatusOK, `{"code":0,"data":{"items":[{"model":"cheap-chat","display_name":"Cheap Chat","context_window":64000},{"model":"cheap-image","billing_mode":"image"}]}}`)
	case "GET /keys":
		reply(http.StatusOK, `{"code":0,"data":{"items":[{"id":1,"key":"`+cheapTestKey+`","status":"active"}],"total":1}}`)
	default:
		http.NotFound(writer, request)
	}
}

func (g *cheapGateway) set(change func()) {
	g.mu.Lock()
	defer g.mu.Unlock()
	change()
}

func (g *cheapGateway) requests() []string {
	g.mu.Lock()
	defer g.mu.Unlock()
	return slices.Clone(g.calls)
}

// cheapDesktop is a whole window over a real Node on a new profile, as the
// desktop composes it, with one provider and one session of the user's own.
type cheapDesktop struct {
	t          *testing.T
	profile    string
	controller *client.Client
	// another is a second window's Client on the same Node.
	another *client.Client
	// before runs as the window dispatches a command, ahead of the Node.
	before  atomic.Pointer[func(protocol.Request)]
	runtime *Runtime
	tester  *ui.Tester
	own     protocol.ConversationProvider
	session protocol.Session
}

type cheapDesktopTransport struct {
	link.Transport
	desktop *cheapDesktop
}

func (t cheapDesktopTransport) Dispatch(ctx context.Context, request protocol.Request) (link.Admission, error) {
	if before := t.desktop.before.Load(); before != nil {
		(*before)(request)
	}
	return t.Transport.Dispatch(ctx, request)
}

// openCheapDesktop starts the Node, lets prepare seed the profile and the
// Node, then opens the window.
func openCheapDesktop(t *testing.T, prepare func(*cheapDesktop)) *cheapDesktop {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	t.Cleanup(cancel)
	d := &cheapDesktop{t: t, profile: filepath.Join(t.TempDir(), "profile")}
	owner, err := node.StartWithOptions(ctx, d.profile, node.Options{})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { owner.Stop(context.Background()) })
	d.controller = client.New(cheapDesktopTransport{link.NewLocal(owner.ID(), owner.ID(), owner), d})
	d.another = client.New(link.NewLocal(owner.ID(), owner.ID(), owner))
	for _, controller := range []*client.Client{d.controller, d.another} {
		if err := controller.Start(ctx); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { controller.Close() })
	}
	d.own = protocol.DefaultConversationProvider()
	d.own.ID, d.own.Name, d.own.Api = protocol.NewProviderID(), "Own", protocol.ConversationModelApiChatCompletions
	d.own.Authentication, d.own.Endpoint = protocol.AuthenticationApiKey, "http://127.0.0.1:9/v1"
	d.own.Enabled, d.own.DefaultModel = true, "own-model"
	model := protocol.DefaultConversationModel()
	model.ID, model.Context, model.Output = "own-model", 200000, 8192
	d.own.Models = []protocol.ConversationModel{model}
	secret := protocol.NewSecret("own-key")
	d.own, _ = detail[protocol.ConversationProvider](d.do(protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: d.own, Secret: &secret}}).Data)
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model = d.own.ID, model.ID
	config.Mode, config.Permission = protocol.WorkModeCode, protocol.PermissionFull
	d.session, _ = detail[protocol.Session](d.do(protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Config: &config}}).Data)
	// The window is composed over the Client's snapshot, which follows the
	// commands' events.
	for end := time.Now().Add(30 * time.Second); d.provider(d.own.ID) == nil || len(d.controller.Snapshot().Sessions) != 1; time.Sleep(2 * time.Millisecond) {
		if time.Now().After(end) {
			t.Fatal("the Node did not publish the fixture's provider and session")
		}
	}
	d.own, d.session = *d.provider(d.own.ID), d.controller.Snapshot().Sessions[0]
	if prepare != nil {
		prepare(d)
	}
	d.runtime, err = NewWithPreferences(ctx, d.controller, preferences.ForStartup(d.profile))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { d.runtime.Close() })
	d.tester = ui.NewTesterWith(d.runtime.App.View, 1280, 820, liveTesterOptions(""))
	d.tester.SetLocale("zh-CN")
	liveTesterScale(d.tester, "")
	d.tester.Frame()
	return d
}

func (d *cheapDesktop) do(command protocol.Command) protocol.Output {
	d.t.Helper()
	output, err := d.controller.Do(d.t.Context(), command)
	if err != nil {
		d.t.Fatalf("%s: %v", command.Kind, err)
	}
	return output
}

func (d *cheapDesktop) account() *cheapauthui.State { return d.runtime.App.Settings.CheapRouter }

// await frames the window until done: the account's workers answer through
// the runtime's queue, which only a frame drains.
func (d *cheapDesktop) await(what string, done func() bool) {
	d.t.Helper()
	for end := time.Now().Add(30 * time.Second); time.Now().Before(end); time.Sleep(2 * time.Millisecond) {
		d.tester.Frame()
		if done() {
			return
		}
	}
	d.t.Fatalf("timed out waiting for %s (stage %d, failure %q); texts: %q", what, d.account().Stage, d.account().Failure, d.tester.Texts())
}

func (d *cheapDesktop) click(name string) {
	d.t.Helper()
	if err := d.tester.Click(name); err != nil {
		d.t.Fatalf("click %q: %v; texts: %q", name, err, d.tester.Texts())
	}
	d.tester.Frame()
}

// workspace fails unless the window shows the user's session with its
// composer and the rail, and no sign-in in their place.
func (d *cheapDesktop) workspace(when string) {
	d.t.Helper()
	d.tester.Frame()
	app := d.runtime.App
	_, composer := d.tester.Access(i18n.In("zh-CN", "composer"))
	_, rail := d.tester.AccessByRole(ui.RoleButton, i18n.In("zh-CN", "settings"))
	if app.Page != shell.ConversationPage || app.Sidebar.Selected != d.session.ID.String() || !composer || !rail ||
		!d.tester.HasText(d.own.DefaultModel) || d.tester.HasText(cheapSignIn) {
		d.t.Fatalf("%s: the workspace is not shown (page %v, selected %q, composer %t, rail %t); texts: %q",
			when, app.Page, app.Sidebar.Selected, composer, rail, d.tester.Texts())
	}
}

// accountPage opens Settings › CheapRouter account from the rail.
func (d *cheapDesktop) accountPage() {
	d.t.Helper()
	if d.runtime.App.Page != shell.SettingsPage {
		d.click(i18n.In("zh-CN", "settings"))
	}
	d.click(i18n.In("zh-CN", "cheaprouter_account"))
	if d.runtime.App.Page != shell.SettingsPage || d.runtime.App.Settings.Section != settings.CheapAccount {
		d.t.Fatalf("the account page did not open: %q", d.tester.Texts())
	}
}

func (d *cheapDesktop) provider(id protocol.ProviderID) *protocol.ConversationProvider {
	for _, provider := range d.controller.Snapshot().Providers {
		if provider.ID == id {
			return &provider
		}
	}
	return nil
}

// gateway awaits the CheapRouter provider in the Client's snapshot, which
// follows a finished write's event, with the given enabled state.
func (d *cheapDesktop) gateway(enabled bool) protocol.ConversationProvider {
	d.t.Helper()
	var provider *protocol.ConversationProvider
	d.await("the CheapRouter provider", func() bool {
		provider = d.provider(providers.CheapRouterProviderID)
		return provider != nil && provider.Enabled == enabled
	})
	return *provider
}

// untouched fails when the user's own provider or session changed.
func (d *cheapDesktop) untouched(when string) {
	d.t.Helper()
	snapshot := d.controller.Snapshot()
	if own := d.provider(d.own.ID); own == nil || !reflect.DeepEqual(*own, d.own) {
		d.t.Fatalf("%s: the user's provider changed: %+v", when, own)
	}
	if index := slices.IndexFunc(snapshot.Sessions, func(session protocol.Session) bool { return session.ID == d.session.ID }); index < 0 ||
		!reflect.DeepEqual(snapshot.Sessions[index], d.session) || len(snapshot.Sessions) != 1 {
		d.t.Fatalf("%s: the user's session changed: %+v", when, snapshot.Sessions)
	}
}

func (d *cheapDesktop) stored() *core.Credentials {
	d.t.Helper()
	store, err := core.NewStore(d.profile)
	if err != nil {
		d.t.Fatal(err)
	}
	credentials, err := store.Load()
	if err != nil {
		d.t.Fatal(err)
	}
	return credentials
}

// signedIn saves a session of endpoint, as a previous run left it.
func (d *cheapDesktop) signedIn(endpoint, access string, expires time.Duration) {
	d.t.Helper()
	store, err := core.NewStore(d.profile)
	if err == nil {
		err = store.Save(core.Credentials{AccessToken: access, RefreshToken: cheapTestRefresh,
			ExpiresAt: time.Now().Add(expires).Unix(), Endpoint: endpoint, APIKey: cheapTestKey})
	}
	if err != nil {
		d.t.Fatal(err)
	}
}

// saveGateway saves the enabled CheapRouter provider an earlier sign-in at
// endpoint left on the Node.
func (d *cheapDesktop) saveGateway(endpoint string) protocol.ConversationProvider {
	d.t.Helper()
	provider, secret, err := providers.CheapRouterProvider(core.Gateway{BaseURL: endpoint + "/v1", APIKey: cheapTestKey,
		Models: []core.Model{{ID: "cheap-chat", ContextWindow: 64000}}})
	if err != nil {
		d.t.Fatal(err)
	}
	saved, _ := detail[protocol.ConversationProvider](d.do(protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: provider, Secret: secret}}).Data)
	return saved
}

// A profile without credentials opens on the workspace; the sign-in is
// offered on its settings page, and starting and cancelling it there changes
// nothing else.
func TestCheapAccountIsOptionalAtStartup(t *testing.T) {
	gateway := newCheapGateway(t)
	d := openCheapDesktop(t, nil)
	d.workspace("startup without credentials")
	if state := d.account(); state.Stage != cheapauthui.SignedOut || state.Failure != "" {
		t.Fatalf("startup account state: stage %d, failure %q", state.Stage, state.Failure)
	}
	d.accountPage()
	if !d.tester.HasText(i18n.In("zh-CN", "cheaprouter_signed_out")) || !d.tester.HasText(cheapSignIn) {
		t.Fatalf("the account page does not offer the sign-in: %q", d.tester.Texts())
	}

	d.account().Endpoint = gateway.URL
	d.click("使用粘贴登录码方式")
	d.await("the sign-in to wait for its code", func() bool { return d.account().Stage == cheapauthui.WaitingForCode })
	if opened := d.tester.OpenedURLs(); len(opened) != 1 || !strings.HasPrefix(opened[0], gateway.URL+"/auth/paseo?") {
		t.Fatalf("the sign-in opened %q", opened)
	}
	d.click(i18n.In("zh-CN", "settings"))
	d.workspace("with a sign-in under way")
	d.accountPage()
	d.click("取消本次登录")
	if state := d.account(); state.Stage != cheapauthui.SignedOut || state.Failure != "" || !d.tester.HasText(cheapSignIn) {
		t.Fatalf("cancelling left stage %d, failure %q: %q", state.Stage, state.Failure, d.tester.Texts())
	}
	d.click(i18n.In("zh-CN", "settings"))
	d.workspace("after cancelling the sign-in")
	d.untouched("after cancelling the sign-in")
	if d.stored() != nil || d.provider(providers.CheapRouterProviderID) != nil || len(gateway.requests()) != 0 {
		t.Fatalf("a cancelled sign-in left credentials, a provider or requests %q behind", gateway.requests())
	}
}

// Saved credentials whose gateway cannot be reached open on the workspace
// all the same: the failure is the account page's, the session is kept, and
// a retry there connects once the gateway answers.
func TestCheapGatewayOutageKeepsWorkspace(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.down = true
	d := openCheapDesktop(t, func(d *cheapDesktop) { d.signedIn(gateway.URL, cheapTestAccess, time.Hour) })
	d.workspace("startup with an unreachable gateway")
	d.await("the gateway sync to fail", func() bool { return d.account().Stage == cheapauthui.GatewayFailed })
	failure := (&core.AuthError{Kind: core.ErrorNetwork}).Error()
	if d.account().Failure != failure {
		t.Fatalf("the unreachable gateway is reported as %q", d.account().Failure)
	}
	d.workspace("after the gateway sync failed")
	d.untouched("after the gateway sync failed")
	if d.stored() == nil || d.provider(providers.CheapRouterProviderID) != nil {
		t.Fatal("an unreachable gateway ended the session or left a provider behind")
	}
	d.accountPage()
	if !d.tester.HasText(failure) || d.tester.HasText(cheapSignIn) {
		t.Fatalf("the account page does not show the failed gateway: %q", d.tester.Texts())
	}

	gateway.set(func() { gateway.down = false })
	d.click("重试网关配置")
	d.await("the retry to connect", func() bool { return d.account().Stage == cheapauthui.Ready })
	d.gateway(true)
	d.untouched("after the retry connected")
}

// Signing in on the account page adds the CheapRouter provider with the
// gateway's key while the window stays where the user is; signing out
// disables that provider alone and removes the credentials.
func TestCheapSignInAddsProviderAndSignOutDisablesOnlyIt(t *testing.T) {
	gateway := newCheapGateway(t)
	exchange := make(chan struct{})
	gateway.exchange = exchange
	d := openCheapDesktop(t, nil)
	d.accountPage()
	d.account().Endpoint = gateway.URL
	d.click(cheapSignIn)
	d.await("the sign-in to wait for the browser", func() bool { return d.account().Stage == cheapauthui.WaitingForCode })
	opened := d.tester.OpenedURLs()
	if len(opened) != 1 {
		t.Fatalf("the sign-in opened %q", opened)
	}
	login, err := url.Parse(opened[0])
	if err != nil {
		t.Fatal(err)
	}
	callback, err := url.Parse(login.Query().Get("redirect_to"))
	if err != nil || callback.Hostname() != "127.0.0.1" {
		t.Fatalf("the sign-in has no loopback redirect: %q", opened[0])
	}
	gateway.set(func() { gateway.challenge = login.Query().Get("code_challenge") })
	// The browser's relay page posts the callback's fragment to the listener.
	delivered := make(chan int, 1)
	go func() {
		request, _ := http.NewRequest(http.MethodPost, "http://"+callback.Host+"/deliver", strings.NewReader("#code=k7qm-3xpd&endpoint="+url.QueryEscape(gateway.URL)))
		request.Header.Set("Origin", "http://"+callback.Host)
		response, err := http.DefaultClient.Do(request)
		if err != nil {
			delivered <- 0
			return
		}
		response.Body.Close()
		delivered <- response.StatusCode
	}()
	d.await("the code to reach the gateway", func() bool { return slices.Contains(gateway.requests(), "POST /auth/desktop-session/exchange") })
	d.click(i18n.In("zh-CN", "settings"))
	d.workspace("while the code is exchanged")
	close(exchange)
	d.await("the gateway to connect", func() bool { return d.account().Stage == cheapauthui.Ready })
	if status := <-delivered; status != http.StatusOK {
		t.Fatalf("the loopback delivery answered %d", status)
	}
	d.workspace("after signing in")
	if !d.tester.HasText(i18n.In("zh-CN", "cheaprouter_signed_in")) {
		t.Fatalf("the sign-in was not reported: %q", d.tester.Texts())
	}
	provider := d.gateway(true)
	if provider.Endpoint != gateway.URL+"/v1" || provider.DefaultModel != "cheap-chat" ||
		len(provider.Models) != 1 || provider.Models[0].ID != "cheap-chat" || provider.Models[0].Context != 64000 {
		t.Fatalf("the signed-in gateway's provider: %+v", provider)
	}
	key, _ := detail[protocol.Secret](d.do(protocol.Command{Kind: "read_provider_key", Data: protocol.CommandReadProviderKeyPayload{
		Provider: provider.ID, ExpectedRevision: provider.Revision}}).Data)
	if key.Expose() != cheapTestKey {
		t.Fatal("the provider does not hold the gateway key")
	}
	d.untouched("after signing in")
	if stored := d.stored(); stored == nil || stored.RefreshToken != cheapTestRefresh || stored.Endpoint != gateway.URL {
		t.Fatal("the sign-in was not saved")
	}
	if info, err := os.Stat(filepath.Join(d.profile, "cheaprouter-account.json")); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("the credential file: %v, %v", info, err)
	}
	d.accountPage()
	for _, text := range []string{"已连接 CheapRouter", "Dev", "Pro", "$12.50", "1 个", gateway.URL} {
		if !d.tester.HasText(text) {
			t.Errorf("the account page lacks %q: %q", text, d.tester.Texts())
		}
	}

	d.click("退出登录")
	d.await("the sign-out", func() bool { return d.account().Stage == cheapauthui.SignedOut && !d.account().LoggingOut })
	if d.account().Failure != "" || !d.tester.HasText(cheapSignIn) {
		t.Fatalf("signing out left %q: %q", d.account().Failure, d.tester.Texts())
	}
	disabled := provider
	disabled.Enabled, disabled.Revision = false, provider.Revision+1
	if after := d.gateway(false); !reflect.DeepEqual(after, disabled) {
		t.Fatalf("signing out did more than disable the provider: %+v", after)
	}
	d.untouched("after signing out")
	if _, err := os.Stat(filepath.Join(d.profile, "cheaprouter-account.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("signing out kept the credential file: %v", err)
	}
	gateway.set(func() {
		if !slices.Equal(gateway.revoked, []string{cheapTestRefresh}) {
			t.Errorf("signing out revoked %q", gateway.revoked)
		}
	})
	d.click(i18n.In("zh-CN", "settings"))
	d.workspace("after signing out")
}

// A session the service rejects ends as a sign-out does: the credentials are
// removed and the CheapRouter provider alone is disabled, in a window that
// shows the workspace throughout.
func TestCheapSessionRejectionDisablesOnlyGateway(t *testing.T) {
	for name, fixture := range map[string]struct {
		access  string
		expires time.Duration
		failure core.ErrorKind
	}{
		"refresh token refused": {cheapTestAccess, -time.Hour, core.ErrorRefresh},
		"access token refused":  {"stale-access", time.Hour, core.ErrorUnauthorized},
	} {
		t.Run(name, func(t *testing.T) {
			gateway := newCheapGateway(t)
			gateway.rejected = true
			var enabled protocol.ConversationProvider
			d := openCheapDesktop(t, func(d *cheapDesktop) {
				d.signedIn(gateway.URL, fixture.access, fixture.expires)
				enabled = d.saveGateway(gateway.URL)
			})
			d.workspace("startup with a rejected session")
			d.await("the session to end", func() bool { return d.account().Stage == cheapauthui.SignedOut && d.account().Failure != "" })
			failure := (&core.AuthError{Kind: fixture.failure, Status: http.StatusUnauthorized}).Error()
			if d.account().Failure != failure {
				t.Fatalf("the rejected session is reported as %q, want %q", d.account().Failure, failure)
			}
			disabled := enabled
			disabled.Enabled, disabled.Revision = false, enabled.Revision+1
			if after := d.gateway(false); !reflect.DeepEqual(after, disabled) {
				t.Fatalf("the rejected session did more than disable the provider: %+v", after)
			}
			d.untouched("after the session ended")
			if d.stored() != nil {
				t.Fatal("the rejected session's credentials were kept")
			}
			d.workspace("after the session ended")
			d.accountPage()
			for _, text := range []string{failure, i18n.In("zh-CN", "cheaprouter_signed_out"), cheapSignIn} {
				if !d.tester.HasText(text) {
					t.Errorf("the account page lacks %q: %q", text, d.tester.Texts())
				}
			}
		})
	}
}

// A CheapRouter provider that another window wrote between this window's
// read and its write fails neither the gateway sync nor the sign-out: the
// change is made to what is stored now.
func TestCheapGatewayWriteSurvivesRevisionRace(t *testing.T) {
	gateway := newCheapGateway(t)
	var saved protocol.ConversationProvider
	var lost [2]atomic.Bool // by the enabled state this window writes
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		d.signedIn(gateway.URL, cheapTestAccess, time.Hour)
		saved = d.saveGateway(gateway.URL)
		race := func(request protocol.Request) {
			input, ok := detail[protocol.CommandSaveProviderPayload](request.Command.Data)
			if !ok || input.Provider.ID != providers.CheapRouterProviderID {
				return
			}
			index := 0
			if input.Provider.Enabled {
				index = 1
			}
			if lost[index].Swap(true) {
				return
			}
			// The other window saves the provider as it is stored.
			output, err := d.another.Do(d.t.Context(), protocol.Command{Kind: "list_providers"})
			if err != nil {
				d.t.Error(err)
				return
			}
			list, _ := detail[[]protocol.ConversationProvider](output.Data)
			for _, stored := range list {
				if stored.ID == providers.CheapRouterProviderID {
					if _, err := d.another.Do(d.t.Context(), protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: stored, ExpectedRevision: stored.Revision}}); err != nil {
						d.t.Error(err)
					}
				}
			}
		}
		d.before.Store(&race)
	})
	d.await("the gateway sync", func() bool { return d.account().Stage != cheapauthui.SyncingGateway })
	if state := d.account(); state.Stage != cheapauthui.Ready || state.Failure != "" || !lost[1].Load() {
		t.Fatalf("the raced sync ended in stage %d with %q", state.Stage, state.Failure)
	}
	// Each of this window's writes lands after the other window's.
	stored := func(enabled bool, revision uint64) func() bool {
		return func() bool {
			provider := d.provider(providers.CheapRouterProviderID)
			return provider != nil && provider.Enabled == enabled && provider.Revision == revision
		}
	}
	d.await("the synced provider", stored(true, saved.Revision+2))
	d.accountPage()
	d.click("退出登录")
	d.await("the sign-out", func() bool { return d.account().Stage == cheapauthui.SignedOut && !d.account().LoggingOut })
	if d.account().Failure != "" || !lost[0].Load() {
		t.Fatalf("the raced sign-out reports %q", d.account().Failure)
	}
	d.await("the disabled provider", stored(false, saved.Revision+4))
	d.untouched("after the raced writes")
}
