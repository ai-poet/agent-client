package live

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/client"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preferences"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node"
	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
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
	// catalogBody, when non-empty, overrides the /models/catalog JSON response.
	catalogBody string
	// groupsBody, when non-empty, overrides the /groups/available JSON response.
	groupsBody string
	keysBody   string
	// failCreateKeyGroup, when non-zero, causes POST /keys for that group_id to return HTTP 500.
	failCreateKeyGroup int64
	createdGroups      []int64
	calls              []string
	revoked            []string
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
	catalogBody, groupsBody, keysBody, failCreateKeyGroup := g.catalogBody, g.groupsBody, g.keysBody, g.failCreateKeyGroup
	g.mu.Unlock()
	if down {
		panic(http.ErrAbortHandler)
	}
	var body map[string]any
	if request.Method == http.MethodPost {
		_ = json.NewDecoder(request.Body).Decode(&body)
	}
	bodyStr := func(key string) string {
		s, _ := body[key].(string)
		return s
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
		if bodyStr("code") != cheapTestCode || challenge == "" || core.PKCEChallenge(bodyStr("code_verifier")) != challenge {
			reply(http.StatusBadRequest, `{"code":400,"reason":"DESKTOP_LOGIN_CODE_INVALID","message":"invalid code"}`)
			return
		}
		reply(http.StatusOK, session)
	case "POST /auth/refresh":
		if rejected || bodyStr("refresh_token") != cheapTestRefresh {
			reply(http.StatusUnauthorized, `{"code":401,"reason":"REFRESH_TOKEN_INVALID","message":"invalid refresh token"}`)
			return
		}
		reply(http.StatusOK, session)
	case "POST /auth/logout":
		g.mu.Lock()
		g.revoked = append(g.revoked, bodyStr("refresh_token"))
		g.mu.Unlock()
		reply(http.StatusOK, `{"code":0,"data":{}}`)
	case "GET /auth/me":
		reply(http.StatusOK, `{"code":0,"data":{"id":7,"email":"dev@example.test","username":"Dev","plan":"Pro","balance":12.5}}`)
	case "GET /groups/available":
		if groupsBody != "" {
			reply(http.StatusOK, groupsBody)
			return
		}
		reply(http.StatusOK, `{"code":0,"data":[{"id":1,"name":"Claude 高速组","platform":"anthropic","rate_multiplier":0.5,"status":"active"},{"id":2,"name":"OpenAI 订阅组","platform":"openai","subscription_type":"subscription","status":"active"},{"id":3,"name":"故障测试组","platform":"gemini","status":"active"}]}`)
	case "GET /models/catalog":
		if catalogBody != "" {
			reply(http.StatusOK, catalogBody)
			return
		}
		reply(http.StatusOK, `{"code":0,"data":{"items":[{"model":"cheap-chat","display_name":"Cheap Chat","context_window":64000},{"model":"cheap-image","billing_mode":"image"}]}}`)
	case "GET /keys":
		if keysBody != "" {
			reply(http.StatusOK, keysBody)
			return
		}
		reply(http.StatusOK, `{"code":0,"data":{"items":[{"id":1,"key":"`+cheapTestKey+`","status":"active"}],"total":1}}`)
	case "POST /keys":
		var gid int64
		if raw, ok := body["group_id"].(float64); ok {
			gid = int64(raw)
		}
		g.mu.Lock()
		g.createdGroups = append(g.createdGroups, gid)
		g.mu.Unlock()
		if failCreateKeyGroup != 0 && gid == failCreateKeyGroup {
			reply(http.StatusInternalServerError, `{"code":500,"message":"group key mint failed"}`)
			return
		}
		keyID := int64(99)
		keyVal := "sk-group-2-minted"
		respGroup := int64(2)
		if gid > 0 {
			keyID = 100 + gid
			keyVal = fmt.Sprintf("sk-group-%d-minted", gid)
			respGroup = gid
		}
		reply(http.StatusOK, fmt.Sprintf(`{"code":0,"data":{"id":%d,"key":%q,"group_id":%d,"status":"active"}}`, keyID, keyVal, respGroup))
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
	reject  atomic.Bool // simulate a definite refusal to admit save_provider
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
	if request.Command.Kind == "save_provider" && t.desktop.reject.Load() {
		return link.Admission{}, errors.New("fixture refused provider write")
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

func (d *cheapDesktop) accountLogout() {
	d.t.Helper()
	if app := d.runtime.App; app.Page != shell.SettingsPage || app.Settings.Section != settings.CheapAccount {
		d.accountPage()
	}
	// The lane cards can push the sign-out button below the account viewport.
	d.tester.Scroll(500, 700, 0, 1000)
	d.tester.Frame()
	d.click("退出登录")
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
	resolvedKey, _ := providers.ResolveCheapRouterKey(key.Expose(), "cheap-chat", "")
	if resolvedKey != cheapTestKey {
		t.Fatalf("the provider does not resolve the gateway key: got %q", resolvedKey)
	}
	d.untouched("after signing in")
	if stored := d.stored(); stored == nil || stored.RefreshToken != cheapTestRefresh || stored.Endpoint != gateway.URL {
		t.Fatal("the sign-in was not saved")
	}
	if info, err := os.Stat(filepath.Join(d.profile, "cheaprouter-account.json")); err != nil ||
		!info.Mode().IsRegular() || runtime.GOOS != "windows" && info.Mode().Perm() != 0o600 {
		t.Fatalf("the credential file: %v, %v", info, err)
	}
	d.accountPage()
	for _, text := range []string{"已连接 CheapRouter", "Dev", "Pro", "$12.50", "1 个", gateway.URL} {
		if !d.tester.HasText(text) {
			t.Errorf("the account page lacks %q: %q", text, d.tester.Texts())
		}
	}

	d.accountLogout()
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

// Capture the default structured sink before startup (a restored session may
// already have finished invalidating by the time openCheapDesktop returns).
func captureCheapInvalidation(t *testing.T) func(string, string, int, bool) {
	t.Helper()
	profile := filepath.Join(t.TempDir(), "profile")
	stop, err := logging.Install(profile, "desktop")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(stop)
	return func(flow, op string, status int, sameOrigin bool) {
		t.Helper()
		path := filepath.Join(profile, "logs", "desktop", "current.log")
		data, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		want := fmt.Sprintf("WARN desktop area=cheaprouter phase=session_invalidation upstream_status=%d auth_flow=%s auth_interface=%s gateway_matches_login=%t disable_succeeded=true", status, flow, op, sameOrigin)
		count := 0
		for _, line := range strings.Split(string(data), "\n") {
			if strings.Contains(line, " phase=session_invalidation ") {
				count++
				_, record, _ := strings.Cut(line, " ")
				if record != want {
					t.Errorf("invalidation record = %q, want %q", record, want)
				}
			}
		}
		if count != 1 {
			t.Errorf("got %d invalidation warnings, want one", count)
		}
		tail, err := logging.Tail(path)
		if err != nil {
			t.Fatal(err)
		}
		for _, field := range []string{"auth_flow", "auth_interface", "upstream_status", "gateway_matches_login", "disable_succeeded"} {
			if strings.Contains(string(tail), `"`+field+`"`) {
				t.Errorf("local field %s appeared in diagnostics", field)
			}
		}
	}
}

// A session the service rejects ends as a sign-out does: the credentials are
// removed and the CheapRouter provider alone is disabled, in a window that
// shows the workspace throughout.
func TestCheapSessionRejectionDisablesOnlyGateway(t *testing.T) {
	for name, fixture := range map[string]struct {
		access  string
		expires time.Duration
		failure core.ErrorKind
		op      string
	}{
		"refresh token refused": {cheapTestAccess, -time.Hour, core.ErrorRefresh, "/auth/refresh"},
		"access token refused":  {"stale-access", time.Hour, core.ErrorUnauthorized, "/auth/me"},
	} {
		t.Run(name, func(t *testing.T) {
			gateway := newCheapGateway(t)
			gateway.rejected = true
			checkLog := captureCheapInvalidation(t)
			var enabled protocol.ConversationProvider
			d := openCheapDesktop(t, func(d *cheapDesktop) {
				d.signedIn(gateway.URL, fixture.access, fixture.expires)
				enabled = d.saveGateway(gateway.URL)
			})
			d.workspace("startup with a rejected session")
			d.await("the session to end", func() bool { return d.account().Stage == cheapauthui.SignedOut && d.account().Failure != "" })
			checkLog("sync", fixture.op, http.StatusUnauthorized, true)
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
	d.accountLogout()
	d.await("the sign-out", func() bool { return d.account().Stage == cheapauthui.SignedOut && !d.account().LoggingOut })
	if d.account().Failure != "" || !lost[0].Load() {
		t.Fatalf("the raced sign-out reports %q", d.account().Failure)
	}
	d.await("the disabled provider", stored(false, saved.Revision+4))
	d.untouched("after the raced writes")
}

func TestCheapGatewaySavesMultiOfferAndMissingContextCatalog(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.groupsBody = `{"code":0,"data":[]}` // Account-default catalog.
	gateway.catalogBody = `{"code":0,"data":{"items":[` +
		`{"model":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","billing_mode":"token","context_window":0},` +
		`{"model":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","billing_mode":"token","context_window":200000},` +
		`{"model":"gpt-5","billing_mode":"token"},` +
		`{"model":"gpt-image-1","billing_mode":"image","context_window":0}` +
		`]}}`
	d := openCheapDesktop(t, func(d *cheapDesktop) { d.signedIn(gateway.URL, cheapTestAccess, time.Hour) })
	d.await("the multi-offer catalog sync to succeed", func() bool { return d.account().Stage == cheapauthui.Ready })
	if d.account().Failure != "" || d.account().ModelCount != 2 {
		t.Fatalf("account = stage:%d count:%d failure:%q", d.account().Stage, d.account().ModelCount, d.account().Failure)
	}
	provider := d.gateway(true)
	if len(provider.Models) != 2 || provider.Models[0].ID != "claude-sonnet-4-6" || provider.Models[0].Context != 200000 ||
		provider.Models[1].ID != "gpt-5" || provider.Models[1].Context != 0 {
		t.Fatalf("saved provider models = %+v", provider.Models)
	}
}

func TestCheapAuthErrorDistinguishesCauses(t *testing.T) {
	cases := []struct {
		name string
		err  error
		want string
	}{
		{
			name: "missing key",
			err:  &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "CheapRouter gateway key is empty"},
			want: "账号没有可用的模型网关密钥",
		},
		{
			name: "no chat models",
			err:  &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "CheapRouter model catalog has no chat models"},
			want: "账号下没有可用的聊天模型",
		},
		{
			name: "invalid parameters",
			err:  &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "model configuration is invalid or unsupported"},
			want: "CheapRouter 模型网关配置参数无效（model configuration is invalid or unsupported）",
		},
		{
			name: "save failure",
			err:  &cheapProviderSaveError{err: errors.New("disk full")},
			want: "保存 CheapRouter 模型网关配置失败，请重试（disk full）",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := cheapAuthError(tc.err)
			if !strings.Contains(got, tc.want) {
				t.Fatalf("cheapAuthError(%v) = %q, want substring %q", tc.err, got, tc.want)
			}
		})
	}
}

func TestCheapSidebarAccountAndGroupSwitchEndToEnd(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.failCreateKeyGroup = 3
	gateway.groupsBody = `{"code":0,"data":[` +
		`{"id":1,"name":"Claude 高速组","platform":"anthropic","status":"active"},` +
		`{"id":2,"name":"OpenAI 订阅组","platform":"openai","status":"active"},` +
		`{"id":3,"name":"故障测试组","platform":"gemini","status":"active"},` +
		`{"id":4,"name":"OpenAI 备用组","platform":"openai","status":"active"},` +
		`{"id":5,"name":"Claude 备用组","platform":"anthropic","status":"active"}]}`
	keyItems := `{"id":1,"key":"` + cheapTestKey + `","status":"active"},` +
		`{"id":2,"key":"sk-group-1","group_id":1,"status":"active"},` +
		`{"id":3,"key":"sk-group-2","group_id":2,"status":"active"},` +
		`{"id":10,"key":"sk-group-5-disabled","group_id":5,"status":"disabled"},` +
		`{"id":11,"key":"sk-group-5-active","group_id":5,"status":"active"}`
	gateway.keysBody = `{"code":0,"data":{"items":[` + keyItems + `],"total":5}}`
	gateway.catalogBody = `{"code":0,"data":{"items":[` +
		`{"model":"claude-sonnet-4-6","billing_mode":"token","context_window":200000,"best_group":{"id":1}},` +
		`{"model":"gpt-5","billing_mode":"token","context_window":128000,"best_group":{"id":2}},` +
		`{"model":"gemini-2.5-pro","billing_mode":"token","context_window":128000,"best_group":{"id":3}},` +
		`{"model":"gpt-5-mini","billing_mode":"token","context_window":128000,"best_group":{"id":4}},` +
		`{"model":"claude-haiku-4-5","billing_mode":"token","context_window":200000,"best_group":{"id":5}}]}}`
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		store, err := core.NewStore(d.profile)
		if err == nil {
			err = store.Save(core.Credentials{AccessToken: cheapTestAccess, RefreshToken: cheapTestRefresh,
				ExpiresAt: time.Now().Add(time.Hour).Unix(), Endpoint: gateway.URL, APIKey: cheapTestKey,
				LaneBindings: map[string]core.LaneBinding{
					core.LaneClaude: {GroupID: 1, APIKey: "sk-group-1"},
					core.LaneCodex:  {GroupID: 2, APIKey: "sk-group-2"},
				},
				ClearedLanes: []string{core.LaneGemini},
			})
		}
		if err != nil {
			t.Fatal(err)
		}
	})
	d.await("initial sync and groups load", func() bool {
		return d.account().Stage == cheapauthui.Ready && len(d.account().Groups) == 5
	})
	if !d.tester.HasText("dev@example.test") || !d.tester.HasText("Pro") {
		t.Fatalf("sidebar footer missing signed-in info: %q", d.tester.Texts())
	}

	clickButton := func(name string) {
		t.Helper()
		button, ok := d.tester.AccessByRole(ui.RoleButton, name)
		if !ok {
			t.Fatalf("no button %q: %q", name, d.tester.Texts())
		}
		d.tester.ClickAt(button.Bounds.X+button.Bounds.W/2, button.Bounds.Y+button.Bounds.H/2)
		d.tester.Frame()
	}
	openLane := func(lane, option string) {
		t.Helper()
		clickButton(lane)
		d.await(lane+" submenu", func() bool {
			_, ok := d.tester.AccessByRole(ui.RoleButton, option)
			return d.tester.HasText(lane+" 分组") && ok
		})
	}
	choose := func(lane, option string) {
		t.Helper()
		openLane(lane, option)
		clickButton(option)
	}
	bindings := func(claude, codex int64) {
		t.Helper()
		want := map[string]int64{core.LaneClaude: claude, core.LaneCodex: codex}
		state := d.account()
		if state.SelectedGroupID != nil || !maps.Equal(state.LaneBindings, want) || state.Stage != cheapauthui.Ready || state.ModelCount != 2 {
			t.Fatalf("confirmed account stage=%d models=%d bindings=%v, want %v", state.Stage, state.ModelCount, state.LaneBindings, want)
		}
		stored := d.stored()
		if stored == nil || stored.GroupID != nil || len(stored.LaneBindings) != len(want) {
			t.Fatalf("wrong persisted lane selection: %v", stored)
		}
		for lane, group := range want {
			if stored.LaneBindings[lane].GroupID != group {
				t.Fatalf("stored group for %s = %d, want %d", lane, stored.LaneBindings[lane].GroupID, group)
			}
		}
		if !slices.Contains(stored.ClearedLanes, core.LaneGemini) {
			t.Fatal("changing another lane lost Gemini's cleared state")
		}
	}
	routes := func(claude, codex string) map[string]protocol.ConversationProvider {
		t.Helper()
		records := map[string]protocol.ConversationProvider{}
		d.await("provider and composer use both selected lane models", func() bool {
			for lane, model := range map[string]string{core.LaneClaude: claude, core.LaneCodex: codex} {
				p := d.provider(providers.CheapRouterLaneID(lane))
				if p == nil || !p.Enabled || len(p.Models) != 1 || p.Models[0].ID != model {
					return false
				}
				shown := false
				for _, m := range d.runtime.App.Data.Models {
					if m.ID == modelKey(p.ID, model) {
						shown = true
					}
				}
				if !shown {
					return false
				}
				records[model] = *p
			}
			return true
		})
		return records
	}
	checkKey := func(records map[string]protocol.ConversationProvider, model, want string) {
		t.Helper()
		p := records[model]
		key, ok := detail[protocol.Secret](d.do(protocol.Command{Kind: "read_provider_key", Data: protocol.CommandReadProviderKeyPayload{Provider: p.ID, ExpectedRevision: p.Revision}}).Data)
		if !ok || key.Expose() != want {
			t.Fatalf("wrong scoped key for %s", model)
		}
	}

	clickButton("dev@example.test")
	d.await("account popover and bound lanes", func() bool {
		return d.tester.HasText("模型分组") && d.tester.HasText("OpenAI 订阅组") && d.tester.HasText("Claude 高速组")
	})
	if d.tester.HasText("OpenAI 备用组") {
		t.Fatalf("the popover showed unselected groups before opening the lane: %q", d.tester.Texts())
	}
	bindings(1, 2)
	routes("claude-sonnet-4-6", "gpt-5")

	beforeSwitch := len(gateway.requests())
	choose("Codex", "OpenAI 备用组")
	d.await("Codex group switch completes", func() bool {
		return !d.account().SwitchingGroup && d.account().LaneBindings[core.LaneCodex] == 4
	})
	if d.account().GroupError != "" {
		t.Fatalf("switching Codex failed: %q", d.account().GroupError)
	}
	bindings(1, 4)
	provider := routes("claude-sonnet-4-6", "gpt-5-mini")
	checkKey(provider, "gpt-5-mini", "sk-group-4-minted")
	checkKey(provider, "claude-sonnet-4-6", "sk-group-1")
	for _, route := range []string{"GET /models/catalog", "GET /keys", "POST /keys"} {
		if !slices.Contains(gateway.requests()[beforeSwitch:], route) {
			t.Errorf("switching Codex did not call %s", route)
		}
	}
	// The minted key is now visible to later switches in other lanes.
	gateway.set(func() {
		gateway.keysBody = `{"code":0,"data":{"items":[` + keyItems +
			`,{"id":104,"key":"sk-group-4-minted","group_id":4,"status":"active"}],"total":6}}`
	})

	// A failed key mint must report the error without changing either working lane.
	choose("Gemini", "故障测试组")
	d.await("failed key mint settles", func() bool {
		return !d.account().SwitchingGroup && d.account().GroupError != ""
	})
	if err := d.account().GroupError; !strings.Contains(err, "切换分组失败") || !d.tester.HasText(err) {
		t.Fatalf("failed group switch did not report its error: %q; texts: %q", err, d.tester.Texts())
	}
	bindings(1, 4)
	if after := routes("claude-sonnet-4-6", "gpt-5-mini"); !reflect.DeepEqual(after, provider) {
		t.Fatal("failed key mint changed the active provider")
	}

	// A definite provider refusal must roll back the staged Claude binding.
	d.reject.Store(true)
	reuseRequests := len(gateway.requests())
	choose("Claude", "Claude 备用组")
	d.await("provider refusal rolls back Claude", func() bool {
		return !d.account().SwitchingGroup && strings.Contains(d.account().GroupError, "fixture refused provider write")
	})
	bindings(1, 4)
	if after := routes("claude-sonnet-4-6", "gpt-5-mini"); !reflect.DeepEqual(after, provider) {
		t.Fatal("provider refusal changed the active provider")
	}

	d.reject.Store(false)
	// An unreadable account file does not end the in-memory session. Force the
	// replacement itself to fail and verify it never admits a provider write.
	store, err := core.NewStore(d.profile)
	if err != nil {
		t.Fatal(err)
	}
	path := store.Path()
	backup := path + ".fixture-backup"
	if err := os.Rename(path, backup); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(path, 0o700); err != nil {
		t.Fatal(err)
	}
	blocker := filepath.Join(path, "fixture")
	if err := os.WriteFile(blocker, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	var attempted atomic.Bool
	observe := func(request protocol.Request) {
		if request.Command.Kind == "save_provider" {
			attempted.Store(true)
		}
	}
	d.before.Store(&observe)
	choose("Claude", "Claude 备用组")
	d.await("local binding write fails", func() bool {
		return !d.account().SwitchingGroup && d.account().GroupError != ""
	})
	d.before.Store(nil)
	if attempted.Load() || d.account().LaneBindings[core.LaneClaude] != 1 || d.account().LaneBindings[core.LaneCodex] != 4 {
		t.Fatal("failed account persistence attempted to change the working route")
	}
	if after := routes("claude-sonnet-4-6", "gpt-5-mini"); !reflect.DeepEqual(after, provider) {
		t.Fatal("failed account persistence changed the active provider")
	}
	for _, fixture := range []string{blocker, path} {
		if err := os.Remove(fixture); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Rename(backup, path); err != nil {
		t.Fatal(err)
	}

	choose("Claude", "Claude 备用组")
	d.await("Claude group switch with a reusable key", func() bool {
		return !d.account().SwitchingGroup && d.account().LaneBindings[core.LaneClaude] == 5
	})
	bindings(5, 4)
	reused := routes("claude-haiku-4-5", "gpt-5-mini")
	checkKey(reused, "claude-haiku-4-5", "sk-group-5-active")
	checkKey(reused, "gpt-5-mini", "sk-group-4-minted")
	if slices.Contains(gateway.requests()[reuseRequests:], "POST /keys") {
		t.Fatal("switching Claude did not reuse its active key")
	}
	d.untouched("after switching CheapRouter groups")
}

func (d *cheapDesktop) signInWithCode(gateway *cheapGateway) {
	d.t.Helper()
	d.accountPage()
	d.account().Endpoint = gateway.URL
	beforeOpened := len(d.tester.OpenedURLs())
	d.click("使用粘贴登录码方式")
	d.await("the manual sign-in to wait for its code", func() bool {
		return d.account().Stage == cheapauthui.WaitingForCode && len(d.tester.OpenedURLs()) > beforeOpened
	})
	opened := d.tester.OpenedURLs()
	login, err := url.Parse(opened[len(opened)-1])
	if err != nil {
		d.t.Fatal(err)
	}
	gateway.set(func() { gateway.challenge = login.Query().Get("code_challenge") })
	d.account().Pasted = cheapTestCode
	d.tester.Frame()
	d.click("完成登录")
	d.await("the signed-in gateway sync to complete", func() bool {
		return d.account().Stage == cheapauthui.Ready
	})
}

func TestCheapLoginAutoBindsUnboundLanesAndIsolatesFailure(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.failCreateKeyGroup = 31
	gateway.groupsBody = `{"code":0,"data":[` +
		`{"id":10,"name":"Claude 停用组","platform":"anthropic","status":"disabled"},` +
		`{"id":11,"name":"Claude 首选组","platform":"anthropic","status":"active"},` +
		`{"id":12,"name":"Claude 备选组","platform":"anthropic","status":"active"},` +
		`{"id":21,"name":"Codex 首选组","platform":"openai","status":"active"},` +
		`{"id":31,"name":"Gemini 故障组","platform":"gemini","status":"active"},` +
		`{"id":41,"name":"国模高速组","platform":"openai","status":"active"}` +
		`]}`
	gateway.catalogBody = `{"code":0,"data":{"items":[` +
		`{"model":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","billing_mode":"token","context_window":200000,"best_group":{"id":11,"name":"Claude 首选组"}},` +
		`{"model":"gpt-5","display_name":"GPT-5","billing_mode":"token","context_window":128000,"best_group":{"id":21,"name":"Codex 首选组"}},` +
		`{"model":"gemini-2.5-pro","display_name":"Gemini 2.5 Pro","billing_mode":"token","context_window":128000,"best_group":{"id":31,"name":"Gemini 故障组"}},` +
		`{"model":"deepseek-r1","display_name":"DeepSeek R1","billing_mode":"token","context_window":128000,"best_group":{"id":41,"name":"国模高速组"}}` +
		`]}}`
	gateway.keysBody = `{"code":0,"data":{"items":[` +
		`{"id":1,"key":"` + cheapTestKey + `","status":"active"},` +
		`{"id":11,"key":"sk-group-11-reused","group_id":11,"status":"active"}` +
		`],"total":2}}`

	d := openCheapDesktop(t, nil)
	d.signInWithCode(gateway)

	wantBindings := map[string]int64{
		core.LaneClaude:   11,
		core.LaneCodex:    21,
		core.LaneDomestic: 41,
	}
	if d.account().Failure != "" {
		t.Fatalf("partial auto-bind failure blocked login: %q", d.account().Failure)
	}
	if !maps.Equal(d.account().LaneBindings, wantBindings) {
		t.Fatalf("LaneBindings = %v, want %v", d.account().LaneBindings, wantBindings)
	}
	if d.account().AutoBindFailures[core.LaneGemini] == "" {
		t.Fatalf("expected Gemini auto-bind failure recorded in state: %v", d.account().AutoBindFailures)
	}

	gateway.mu.Lock()
	created := slices.Clone(gateway.createdGroups)
	gateway.mu.Unlock()
	if slices.Contains(created, 11) {
		t.Fatalf("group 11 active key was minted instead of reused: %v", created)
	}
	for _, wantCreated := range []int64{21, 31, 41} {
		if !slices.Contains(created, wantCreated) {
			t.Errorf("expected POST /keys for group %d, got %v", wantCreated, created)
		}
	}

	for lane, want := range map[string]string{core.LaneClaude: "sk-group-11-reused", core.LaneCodex: "sk-group-21-minted", core.LaneDomestic: "sk-group-41-minted"} {
		id := providers.CheapRouterLaneID(lane)
		d.await("bound provider", func() bool { p := d.provider(id); return p != nil && p.Enabled })
		p := d.provider(id)
		secret, ok := detail[protocol.Secret](d.do(protocol.Command{Kind: "read_provider_key", Data: protocol.CommandReadProviderKeyPayload{Provider: p.ID, ExpectedRevision: p.Revision}}).Data)
		if !ok || secret.Expose() != want {
			t.Fatalf("wrong scoped credential for lane %s", lane)
		}
	}
	d.await("composer model selector refreshes with auto-bound lane models", func() bool {
		var hasClaude, hasGPT5, hasDeepSeek bool
		for _, m := range d.runtime.App.Data.Models {
			switch {
			case strings.HasSuffix(m.ID, "/claude-sonnet-4-6"):
				hasClaude = true
			case strings.HasSuffix(m.ID, "/gpt-5"):
				hasGPT5 = true
			case strings.HasSuffix(m.ID, "/deepseek-r1"):
				hasDeepSeek = true
			}
		}
		return hasClaude && hasGPT5 && hasDeepSeek
	})
}

func TestCheapAutoBindPreservesBindingsAndRespectsManualClearUntilNextLogin(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.groupsBody = `{"code":0,"data":[` +
		`{"id":11,"name":"Claude 首选组","platform":"anthropic","status":"active"},` +
		`{"id":12,"name":"Claude 用户自选组","platform":"anthropic","status":"active"},` +
		`{"id":21,"name":"Codex 首选组","platform":"openai","status":"active"}` +
		`]}`
	gateway.catalogBody = `{"code":0,"data":{"items":[` +
		`{"model":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","billing_mode":"token","context_window":200000,"best_group":{"id":11}},` +
		`{"model":"claude-opus-4-6","display_name":"Claude Opus 4.6","billing_mode":"token","context_window":200000,"best_group":{"id":12}},` +
		`{"model":"gpt-5","display_name":"GPT-5","billing_mode":"token","context_window":128000,"best_group":{"id":21}}` +
		`]}}`
	gateway.keysBody = `{"code":0,"data":{"items":[` +
		`{"id":1,"key":"` + cheapTestKey + `","status":"active"},` +
		`{"id":11,"key":"sk-group-11-active","group_id":11,"status":"active"},` +
		`{"id":12,"key":"sk-group-12-active","group_id":12,"status":"active"},` +
		`{"id":21,"key":"sk-group-21-active","group_id":21,"status":"active"}` +
		`],"total":4}}`

	d := openCheapDesktop(t, func(d *cheapDesktop) {
		store, err := core.NewStore(d.profile)
		if err != nil {
			t.Fatal(err)
		}
		if err := store.Save(core.Credentials{
			AccessToken:  cheapTestAccess,
			RefreshToken: cheapTestRefresh,
			ExpiresAt:    time.Now().Add(time.Hour).Unix(),
			Endpoint:     gateway.URL,
			APIKey:       cheapTestKey,
			LaneBindings: map[string]core.LaneBinding{
				core.LaneClaude: {GroupID: 12, APIKey: "sk-group-12-active"},
			},
		}); err != nil {
			t.Fatal(err)
		}
	})
	d.await("initial sync preserves Claude and auto-binds Codex", func() bool {
		return d.account().Stage == cheapauthui.Ready
	})
	if got := d.account().LaneBindings; !maps.Equal(got, map[string]int64{core.LaneClaude: 12, core.LaneCodex: 21}) {
		t.Fatalf("after initial sync LaneBindings = %v, want {anthropic:12 openai:21}", got)
	}

	// Manually clear Codex in this session.
	d.account().SelectLaneGroup(core.LaneCodex, nil)
	d.await("manual Codex unbind completes", func() bool {
		return !d.account().SwitchingGroup && d.account().BoundGroupID(core.LaneCodex) == nil
	})
	if got := d.account().LaneBindings; !maps.Equal(got, map[string]int64{core.LaneClaude: 12}) {
		t.Fatalf("after manual clear LaneBindings = %v, want {anthropic:12}", got)
	}

	// Refreshing groups and rendering frames must NOT trigger auto-bind.
	d.account().RefreshGroups()
	d.await("group refresh finishes", func() bool { return !d.account().GroupsLoading })
	d.tester.Frame()
	if d.account().BoundGroupID(core.LaneCodex) != nil {
		t.Fatalf("RefreshGroups/render refilled cleared Codex lane: %v", d.account().LaneBindings)
	}

	// Repeating sync in the same login session must NOT refill manually cleared Codex.
	d.runtime.startCheapSync(d.runtime.cheapAuth, d.runtime.cheapAuth.generation)
	d.await("same-session re-sync finishes", func() bool { return d.account().Stage == cheapauthui.Ready })
	if got := d.account().LaneBindings; !maps.Equal(got, map[string]int64{core.LaneClaude: 12}) {
		t.Fatalf("same-session re-sync altered bindings: %v, want {anthropic:12}", got)
	}

	// Logout and next login must auto-bind all unbound lanes (including previously cleared Codex).
	d.account().Logout()
	d.await("logout completes", func() bool {
		return d.account().Stage == cheapauthui.SignedOut && !d.account().LoggingOut
	})
	d.signInWithCode(gateway)
	if got := d.account().LaneBindings; !maps.Equal(got, map[string]int64{core.LaneClaude: 11, core.LaneCodex: 21}) {
		t.Fatalf("next login LaneBindings = %v, want {anthropic:11 openai:21}", got)
	}
}

func TestCheapManagedProvidersProjectSharedModelByBoundGroup(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.groupsBody = `{"code":0,"data":[{"id":11,"name":"Google group","platform":"gemini"},{"id":22,"name":"Claude group","platform":"anthropic"},{"id":33,"name":"OpenAI group","platform":"openai","supports_responses":true}]}`
	gateway.catalogBody = fmt.Sprintf(`{"code":0,"data":{"base_url":%q,"items":[{"model":"shared-model","best_group":{"id":22},"other_groups":[{"group":{"id":33}},{"group":{"id":11}}]}]}}`, gateway.URL+"/proxy/v1")
	cases := []struct {
		lane, group, path string
		gid               int64
		api               protocol.ConversationModelApi
	}{
		{core.LaneClaude, "Google group", "/proxy/v1beta", 11, protocol.ConversationModelApiGemini},
		{core.LaneGemini, "Claude group", "/proxy/v1/messages", 22, protocol.ConversationModelApiAnthropic},
		{core.LaneCodex, "OpenAI group", "/proxy/v1/responses", 33, protocol.ConversationModelApiResponses},
	}
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		// An earlier version left its aggregate record behind. Do not repurpose
		// that identity for a particular lane.
		d.saveGateway(gateway.URL)
		c := core.Credentials{AccessToken: cheapTestAccess, RefreshToken: cheapTestRefresh, ExpiresAt: time.Now().Add(time.Hour).Unix(), Endpoint: gateway.URL, LaneBindings: map[string]core.LaneBinding{}}
		for _, tc := range cases {
			c.LaneBindings[tc.lane] = core.LaneBinding{GroupID: tc.gid}
		}
		store, err := core.NewStore(d.profile)
		if err != nil {
			t.Fatal(err)
		}
		if err := store.Save(c); err != nil {
			t.Fatal(err)
		}
	})
	d.await("three scoped offers in composer", func() bool {
		if d.account().Stage != cheapauthui.Ready || d.account().ModelCount != 3 {
			return false
		}
		for _, tc := range cases {
			key := modelKey(providers.CheapRouterLaneID(tc.lane), "shared-model")
			found := false
			for _, m := range d.runtime.App.Data.Models {
				if m.ID == key && m.Group != nil && m.Group.GroupName == tc.group {
					found = true
				}
			}
			if !found {
				return false
			}
		}
		return true
	})
	d.gateway(false)
	for _, tc := range cases {
		p := d.provider(providers.CheapRouterLaneID(tc.lane))
		if p == nil || !p.Enabled || p.Api != tc.api || p.Endpoint != gateway.URL+tc.path {
			t.Fatalf("incorrect persisted provider for %s: %+v", tc.lane, p)
		}
		secret, ok := detail[protocol.Secret](d.do(protocol.Command{Kind: "read_provider_key", Data: protocol.CommandReadProviderKeyPayload{Provider: p.ID, ExpectedRevision: p.Revision}}).Data)
		if !ok || secret.Expose() != fmt.Sprintf("sk-group-%d-minted", tc.gid) {
			t.Fatalf("wrong group key for %s", tc.lane)
		}
	}
	d.account().SelectLaneGroup(core.LaneGemini, nil)
	d.await("unbound Gemini record disabled", func() bool {
		p := d.provider(providers.CheapRouterLaneID(core.LaneGemini))
		return !d.account().SwitchingGroup && p != nil && !p.Enabled && d.account().ModelCount == 2
	})
	d.account().Logout()
	d.await("all managed records disabled", func() bool {
		if d.account().Stage != cheapauthui.SignedOut || d.account().LoggingOut {
			return false
		}
		for _, p := range d.controller.Snapshot().Providers {
			if providers.IsCheapRouterProvider(p.ID) && p.Enabled {
				return false
			}
		}
		return true
	})
	d.untouched("after scoped provider cleanup")
}

// An earlier version kept every model on the unbound route. Lanes replaced
// it, and each of its sessions follows its model to the lane that carries it.
// A lane the user unbinds later keeps its sessions: another lane that offers
// the same model never takes them.
func TestCheapSessionsFollowTheLaneOfTheirModel(t *testing.T) {
	gateway := newCheapGateway(t)
	gateway.groupsBody = `{"code":0,"data":[` +
		`{"id":11,"name":"Claude group","platform":"anthropic","status":"active"},` +
		`{"id":21,"name":"Codex group","platform":"openai","status":"active"}` +
		`]}`
	gateway.catalogBody = `{"code":0,"data":{"items":[` +
		`{"model":"claude-sonnet-4-6","billing_mode":"token","context_window":200000,"best_group":{"id":11}},` +
		`{"model":"gpt-5","billing_mode":"token","context_window":128000,"best_group":{"id":21}},` +
		`{"model":"shared-model","billing_mode":"token","context_window":128000,"best_group":{"id":21},"other_groups":[{"group":{"id":11}}]}` +
		`]}}`
	gateway.keysBody = `{"code":0,"data":{"items":[` +
		`{"id":1,"key":"` + cheapTestKey + `","status":"active"},` +
		`{"id":11,"key":"sk-group-11-active","group_id":11,"status":"active"},` +
		`{"id":21,"key":"sk-group-21-active","group_id":21,"status":"active"}` +
		`],"total":3}}`
	claude, codex := providers.CheapRouterLaneID(core.LaneClaude), providers.CheapRouterLaneID(core.LaneCodex)
	earlier := map[string]protocol.Session{}
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		unbound, secret, err := providers.CheapRouterProvider(core.Gateway{BaseURL: gateway.URL + "/v1", APIKey: cheapTestKey, Models: []core.Model{
			{ID: "claude-sonnet-4-6", ContextWindow: 200000}, {ID: "gpt-5", ContextWindow: 128000}, {ID: "shared-model", ContextWindow: 128000}}})
		if err != nil {
			t.Fatal(err)
		}
		unbound, _ = detail[protocol.ConversationProvider](d.do(protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: unbound, Secret: secret}}).Data)
		for _, model := range unbound.Models {
			config := protocol.DefaultSessionConfig()
			config.Provider, config.Model, config.Credential, config.Effort = unbound.ID, model.ID, unbound.Credential, model.DefaultEffort
			config.Mode, config.Permission = protocol.WorkModePlan, protocol.PermissionProject
			earlier[model.ID], _ = detail[protocol.Session](d.do(protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Config: &config}}).Data)
		}
		store, err := core.NewStore(d.profile)
		if err == nil {
			err = store.Save(core.Credentials{AccessToken: cheapTestAccess, RefreshToken: cheapTestRefresh, ExpiresAt: time.Now().Add(time.Hour).Unix(), Endpoint: gateway.URL, APIKey: cheapTestKey,
				LaneBindings: map[string]core.LaneBinding{core.LaneClaude: {GroupID: 11, APIKey: "sk-group-11-active"}, core.LaneCodex: {GroupID: 21, APIKey: "sk-group-21-active"}}})
		}
		if err != nil {
			t.Fatal(err)
		}
	})
	read := func(id protocol.SessionID) protocol.Session {
		session, _ := detail[protocol.Session](d.do(protocol.Command{Kind: "read_session", Data: protocol.CommandReadSessionPayload{Session: id}}).Data)
		return session
	}
	d.await("lanes to replace the unbound route", func() bool {
		lanes := d.provider(claude) != nil && d.provider(codex) != nil && d.provider(claude).Enabled && d.provider(codex).Enabled
		return d.account().Stage == cheapauthui.Ready && lanes
	})
	d.gateway(false)
	// shared-model is offered by both lanes; the account routes it through Claude's group.
	for model, lane := range map[string]protocol.ProviderID{"claude-sonnet-4-6": claude, "shared-model": claude, "gpt-5": codex} {
		want := earlier[model].Config
		want.Provider, want.Credential = lane, d.provider(lane).Credential
		if got := read(earlier[model].ID); !reflect.DeepEqual(got.Config, want) {
			t.Fatalf("%s session did not follow its lane:\n got %+v\nwant %+v", model, got.Config, want)
		}
	}
	if own := read(d.session.ID); own.Revision != d.session.Revision {
		t.Fatalf("the user's own session changed: %+v", own)
	}

	chosen := earlier["shared-model"].Config
	chosen.Provider, chosen.Credential = codex, d.provider(codex).Credential
	onCodex, _ := detail[protocol.Session](d.do(protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Config: &chosen}}).Data)
	d.account().SelectLaneGroup(core.LaneCodex, nil)
	d.await("the unbound Codex lane to be disabled", func() bool {
		provider := d.provider(codex)
		return !d.account().SwitchingGroup && provider != nil && !provider.Enabled
	})
	for _, session := range []protocol.Session{read(onCodex.ID), read(earlier["gpt-5"].ID)} {
		if session.Config.Provider != codex {
			t.Fatalf("%s session left its unbound lane for %s", session.Config.Model, session.Config.Provider)
		}
		for _, command := range []protocol.Command{
			{Kind: "compact_context", Data: protocol.CommandCompactContextPayload{Session: session.ID, ExpectedRevision: session.Revision}},
			{Kind: "submit_turn", Data: protocol.CommandSubmitTurnPayload{Session: session.ID, ExpectedRevision: session.Revision, Message: protocol.ConversationInput{Text: "control"}}},
		} {
			_, err := d.controller.Do(t.Context(), command)
			wantMessage := "model provider is unavailable"
			if command.Kind == "compact_context" {
				wantMessage = "Select an available model before compacting this conversation."
			}
			if failure := commandFault(err); failure == nil || failure.Code != protocol.ErrorCodeNotConfigured || failure.ConfigurationReason() != protocol.ConfigurationProviderDisabled || failure.Message != wantMessage {
				t.Fatalf("%s on the unbound lane: %v", command.Kind, err)
			}
		}
	}
}
