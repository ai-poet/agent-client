package cheapauth

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

const (
	testCode     = "K7QM3XPD"
	testAccess   = "access-secret-never-log"
	testRefresh  = "refresh-secret-never-log"
	testGateway  = "sk-gateway-never-log"
	testVerifier = "correct-verifier"
)

func writeEnvelope(t *testing.T, writer http.ResponseWriter, status int, body string) {
	t.Helper()
	writer.Header().Set("Content-Type", "application/json")
	writer.WriteHeader(status)
	_, _ = writer.Write([]byte(body))
}

func exchangeServer(t *testing.T, mutate func(http.ResponseWriter, *http.Request) bool) *httptest.Server {
	t.Helper()
	return httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/api/v1/auth/desktop-session/exchange" || request.Method != http.MethodPost {
			http.NotFound(writer, request)
			return
		}
		if mutate != nil && mutate(writer, request) {
			return
		}
		var body map[string]string
		if err := json.NewDecoder(request.Body).Decode(&body); err != nil {
			t.Error(err)
		}
		if body["code"] != testCode || body["code_verifier"] != testVerifier {
			writeEnvelope(t, writer, http.StatusBadRequest, `{"code":400,"reason":"DESKTOP_LOGIN_CODE_VERIFIER_MISMATCH","message":"verifier mismatch"}`)
			return
		}
		writeEnvelope(t, writer, http.StatusOK, `{"code":0,"message":"ok","data":{"access_token":"`+testAccess+`","refresh_token":"`+testRefresh+`","expires_in":900,"api_key":"`+testGateway+`"}}`)
	}))
}

func TestPKCEExchangePersistsOwnerOnlyCredentials(t *testing.T) {
	server := exchangeServer(t, nil)
	defer server.Close()
	profile := t.TempDir()
	service, err := NewService(profile)
	if err != nil {
		t.Fatal(err)
	}
	flow, err := service.StartLogin(t.Context(), server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer flow.Close()
	// The fake service knows this attempt's verifier.
	flow.pkce.verifier = testVerifier
	flow.pkce.challenge = PKCEChallenge(testVerifier)
	parsed, err := url.Parse(flow.LoginURL())
	if err != nil {
		t.Fatal(err)
	}
	query := parsed.Query()
	if parsed.Path != "/auth/paseo" || query.Get("endpoint") != server.URL || query.Get("code_challenge_method") != "S256" ||
		query.Get("redirect_to") != fmt.Sprintf("http://127.0.0.1:%d/callback", flow.Port()) || query.Get("code_challenge") != PKCEChallenge(testVerifier) {
		t.Fatalf("登录 URL 不符合契约: %s", flow.LoginURL())
	}
	credentials, err := flow.ExchangePasted(t.Context(), "k7qm-3xpd")
	if err != nil {
		t.Fatal(err)
	}
	if credentials.AccessToken != testAccess || credentials.Endpoint != server.URL || credentials.APIKey != testGateway {
		t.Fatalf("兑换结果不完整: %s", credentials)
	}
	// 兑换只产出候选凭据；当前 attempt 的持有者负责落盘。
	if err := service.Establish(t.Context(), credentials); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(service.store.Path())
	if err != nil {
		t.Fatal(err)
	}
	if got := info.Mode().Perm(); got != 0o600 {
		t.Fatalf("凭据权限 = %04o, want 0600", got)
	}
	stored, err := service.store.Load()
	if err != nil || stored == nil || stored.RefreshToken != testRefresh {
		t.Fatalf("凭据未落盘: %v, %v", stored, err)
	}
	if !service.SignedIn() {
		t.Fatal("兑换后 Service 未进入登录态")
	}
}

func TestExchangeRejectsVerifierMismatch(t *testing.T) {
	server := exchangeServer(t, nil)
	defer server.Close()
	client, err := NewClient(server.URL, server.Client())
	if err != nil {
		t.Fatal(err)
	}
	_, err = client.Exchange(t.Context(), testCode, "wrong-verifier")
	if !errors.Is(err, ErrVerifierMismatch) {
		t.Fatalf("错误分类 = %v, want verifier_mismatch", err)
	}
	if strings.Contains(err.Error(), "wrong-verifier") {
		t.Fatal("错误泄露 verifier")
	}
}

func TestExchangeRejectsExpiredCode(t *testing.T) {
	server := exchangeServer(t, func(writer http.ResponseWriter, request *http.Request) bool {
		writeEnvelope(t, writer, http.StatusBadRequest, `{"code":400,"reason":"DESKTOP_LOGIN_CODE_EXPIRED","message":"code expired"}`)
		return true
	})
	defer server.Close()
	client, _ := NewClient(server.URL, server.Client())
	_, err := client.Exchange(t.Context(), testCode, testVerifier)
	if !errors.Is(err, ErrCodeExpired) {
		t.Fatalf("错误分类 = %v, want code_expired", err)
	}
}

func TestRefreshSuccessAndFailure(t *testing.T) {
	var refreshCalls atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/api/v1/auth/refresh" {
			http.NotFound(writer, request)
			return
		}
		var body map[string]string
		_ = json.NewDecoder(request.Body).Decode(&body)
		if body["refresh_token"] != testRefresh {
			t.Error("刷新请求未携带原 refresh token")
		}
		if refreshCalls.Add(1) == 1 {
			writeEnvelope(t, writer, http.StatusOK, `{"code":0,"data":{"access_token":"new-access-secret","refresh_token":"","expires_in":3600}}`)
			return
		}
		writeEnvelope(t, writer, http.StatusUnauthorized, `{"code":401,"reason":"REFRESH_TOKEN_INVALID","message":"`+testRefresh+`"}`)
	}))
	defer server.Close()
	service, err := NewService(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	client, _ := NewClient(server.URL, server.Client())
	fixed := time.Unix(1_000_000, 0)
	service.now = func() time.Time { return fixed }
	client.now = service.now
	service.client = client
	service.credentials = &Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: fixed.Unix() + 60}
	if err := service.store.Save(*service.credentials); err != nil {
		t.Fatal(err)
	}
	token, err := service.AccessToken(t.Context())
	if err != nil || token != "new-access-secret" {
		t.Fatalf("提前刷新失败: token=%q err=%v", token, err)
	}
	stored, err := service.store.Load()
	if err != nil || stored.RefreshToken != testRefresh || stored.ExpiresAt != fixed.Unix()+3600 {
		t.Fatalf("刷新后凭据错误: %s, %v", stored, err)
	}
	service.credentials.ExpiresAt = fixed.Unix() + 60
	if err := service.store.Save(*service.credentials); err != nil {
		t.Fatal(err)
	}
	_, err = service.AccessToken(t.Context())
	if !errors.Is(err, ErrRefresh) {
		t.Fatalf("刷新失败分类 = %v, want refresh_failed", err)
	}
	if strings.Contains(err.Error(), testRefresh) {
		t.Fatal("刷新错误泄露 token")
	}
}

// rotatingServer renews as the service does: presenting the live refresh
// token rotates it and the old one dies at once; a dead one is refused. It
// records every token presented for renewal and for revocation.
type rotatingServer struct {
	*httptest.Server
	mu        sync.Mutex
	live      string
	presented []string
	revoked   []string
}

func newRotatingServer(t *testing.T, live string) *rotatingServer {
	rotating := &rotatingServer{live: live}
	rotating.Server = httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		var body map[string]string
		_ = json.NewDecoder(request.Body).Decode(&body)
		rotating.mu.Lock()
		defer rotating.mu.Unlock()
		switch request.URL.Path {
		case "/api/v1/auth/refresh":
			rotating.presented = append(rotating.presented, body["refresh_token"])
			if body["refresh_token"] != rotating.live {
				writeEnvelope(t, writer, http.StatusUnauthorized, `{"code":401,"reason":"REFRESH_TOKEN_INVALID","message":"invalid refresh token"}`)
				return
			}
			generation := len(rotating.presented) + 1
			rotating.live = fmt.Sprintf("rt_%d", generation)
			writeEnvelope(t, writer, http.StatusOK, fmt.Sprintf(`{"code":0,"data":{"access_token":"at_%d","refresh_token":"%s","expires_in":900}}`, generation, rotating.live))
		case "/api/v1/auth/logout":
			rotating.revoked = append(rotating.revoked, body["refresh_token"])
			writeEnvelope(t, writer, http.StatusOK, `{"code":0,"data":{}}`)
		default:
			http.NotFound(writer, request)
		}
	}))
	t.Cleanup(rotating.Close)
	return rotating
}

// windows restores count Services from one profile holding an expired
// session of server, as that many windows of one desktop do.
func (r *rotatingServer) windows(t *testing.T, count int) []*Service {
	t.Helper()
	profile := t.TempDir()
	store, err := NewStore(profile)
	if err == nil {
		err = store.Save(Credentials{AccessToken: "at_1", RefreshToken: r.live, Endpoint: r.URL, ExpiresAt: time.Now().Add(-time.Hour).Unix()})
	}
	if err != nil {
		t.Fatal(err)
	}
	services := make([]*Service, count)
	for index := range services {
		if services[index], err = NewService(profile); err == nil {
			err = services[index].Restore()
		}
		if err != nil {
			t.Fatal(err)
		}
	}
	return services
}

// A window whose copy of the session went stale while another window renewed
// it takes the renewed pair from the file. Presenting its dead refresh token
// would be refused, and the refusal would clear the session for every window.
func TestRenewalAdoptsAnotherWindowsPair(t *testing.T) {
	server := newRotatingServer(t, "rt_1")
	windows := server.windows(t, 2)
	for index, window := range windows {
		if token, err := window.AccessToken(t.Context()); err != nil || token != "at_2" {
			t.Fatalf("window %d holds %q, %v", index, token, err)
		}
	}
	if !slices.Equal(server.presented, []string{"rt_1"}) {
		t.Fatalf("the windows presented %q for renewal", server.presented)
	}
	if stored, err := windows[0].store.Load(); err != nil || stored == nil || stored.RefreshToken != "rt_2" {
		t.Fatalf("the renewed session was not kept: %v, %v", stored, err)
	}
}

// Windows that renew at once do so one after the other, the later ones on
// the first one's pair.
func TestConcurrentRenewalsRotateOnce(t *testing.T) {
	server := newRotatingServer(t, "rt_1")
	windows := server.windows(t, 4)
	failed := make(chan error, len(windows))
	for _, window := range windows {
		go func() {
			token, err := window.AccessToken(t.Context())
			if err == nil && token != "at_2" {
				err = fmt.Errorf("a window holds %q", token)
			}
			failed <- err
		}()
	}
	for range windows {
		if err := <-failed; err != nil {
			t.Fatal(err)
		}
	}
	if !slices.Equal(server.presented, []string{"rt_1"}) {
		t.Fatalf("the windows presented %q for renewal", server.presented)
	}
}

// A sign-out in one window ends the session in the others without a request,
// and revokes the pair that is stored, not the one the window last saw.
func TestSignOutReachesOtherWindows(t *testing.T) {
	server := newRotatingServer(t, "rt_1")
	windows := server.windows(t, 3)
	if _, err := windows[0].AccessToken(t.Context()); err != nil {
		t.Fatal(err)
	}
	if err := windows[1].Logout(t.Context()); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(server.revoked, []string{"rt_2"}) {
		t.Fatalf("the sign-out revoked %q", server.revoked)
	}
	for index, window := range windows {
		if _, err := window.AccessToken(t.Context()); !errors.Is(err, ErrUnauthorized) || window.SignedIn() {
			t.Fatalf("window %d after the sign-out: %v", index, err)
		}
	}
	if !slices.Equal(server.presented, []string{"rt_1"}) {
		t.Fatalf("the signed-out windows presented %q for renewal", server.presented)
	}
}

func TestPastedCodeUsesCurrentPKCEAndStore(t *testing.T) {
	server := exchangeServer(t, nil)
	defer server.Close()
	service, _ := NewService(t.TempDir())
	flow, err := service.StartManualLogin(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	flow.pkce.verifier = testVerifier
	flow.pkce.challenge = PKCEChallenge(testVerifier)
	if strings.Contains(flow.LoginURL(), "redirect_to=") {
		t.Fatal("粘贴模式不应携带 loopback 回跳")
	}
	credentials, err := flow.ExchangePasted(t.Context(), "http://127.0.0.1:65535/callback#code=K7QM-3XPD&endpoint="+url.QueryEscape(server.URL))
	if err != nil {
		t.Fatal(err)
	}
	if err := service.Establish(t.Context(), credentials); err != nil {
		t.Fatal(err)
	}
	if !service.SignedIn() {
		t.Fatal("粘贴 code 未建立登录态")
	}
}

func TestLoopbackRelayExchangesPostedFragment(t *testing.T) {
	server := exchangeServer(t, nil)
	defer server.Close()
	service, _ := NewService(t.TempDir())
	flow, err := service.StartLogin(t.Context(), server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer flow.Close()
	flow.pkce.verifier = testVerifier
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	waited := make(chan error, 1)
	go func() {
		_, err := flow.Wait(ctx)
		waited <- err
	}()
	response, err := http.Get(fmt.Sprintf("http://127.0.0.1:%d/callback", flow.Port()))
	if err != nil {
		t.Fatal(err)
	}
	page, _ := io.ReadAll(response.Body)
	response.Body.Close()
	if !strings.Contains(string(page), "location.hash") || !strings.Contains(string(page), "fetch('/deliver'") {
		t.Fatal("中继页没有 POST 回传 fragment")
	}
	request, _ := http.NewRequest(http.MethodPost, fmt.Sprintf("http://127.0.0.1:%d/deliver", flow.Port()), strings.NewReader("#code="+testCode))
	request.Header.Set("Origin", fmt.Sprintf("http://127.0.0.1:%d", flow.Port()))
	response, err = http.DefaultClient.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	response.Body.Close()
	if response.StatusCode != http.StatusOK {
		t.Fatalf("回传状态 = %d", response.StatusCode)
	}
	if err := <-waited; err != nil {
		t.Fatal(err)
	}
}

// A delivery the service refuses, as a stale tab's code is, leaves the
// listener waiting: the next delivery signs in, and only then is it over.
func TestLoopbackKeepsWaitingAfterRefusedCode(t *testing.T) {
	server := exchangeServer(t, nil)
	defer server.Close()
	service, _ := NewService(t.TempDir())
	flow, err := service.StartLogin(t.Context(), server.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer flow.Close()
	flow.pkce.verifier = testVerifier
	waited := make(chan error, 1)
	go func() {
		credentials, err := flow.Wait(t.Context())
		if err == nil && credentials.RefreshToken != testRefresh {
			err = errors.New("the wait ended without the redeemed session")
		}
		waited <- err
	}()
	deliver := func(fragment string) int {
		t.Helper()
		response, err := http.Post(fmt.Sprintf("http://127.0.0.1:%d/deliver", flow.Port()), "text/plain", strings.NewReader(fragment))
		if err != nil {
			t.Fatal(err)
		}
		response.Body.Close()
		return response.StatusCode
	}
	for _, refused := range []string{"#code=STALE123", "#state=only", "#code=" + testCode + "&endpoint=https%3A%2F%2Felsewhere.example"} {
		if status := deliver(refused); status != http.StatusBadRequest {
			t.Fatalf("delivery %q answered %d", refused, status)
		}
	}
	if status := deliver("#code=" + testCode); status != http.StatusOK {
		t.Fatalf("the redeemable delivery after the refused ones answered %d", status)
	}
	if err := <-waited; err != nil {
		t.Fatal(err)
	}
	if status := deliver("#code=" + testCode); status != http.StatusConflict {
		t.Fatalf("a delivery after the sign-in answered %d", status)
	}
}

func TestTokensNeverAppearInErrorsOrLogs(t *testing.T) {
	server := exchangeServer(t, func(writer http.ResponseWriter, request *http.Request) bool {
		writeEnvelope(t, writer, http.StatusBadRequest, `{"code":400,"reason":"DESKTOP_LOGIN_CODE_INVALID","message":"bad `+testAccess+` `+testRefresh+` `+testVerifier+`"}`)
		return true
	})
	defer server.Close()
	client, _ := NewClient(server.URL, server.Client())
	_, err := client.Exchange(t.Context(), testCode, testVerifier)
	var logs bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&logs, nil))
	credentials := Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: 100}
	logger.Error("请求失败", "error", err, "credentials", credentials, "pkce", PKCE{verifier: testVerifier, challenge: "public"})
	formatted := fmt.Sprintf("%v %+v %#v %s", credentials, credentials, credentials, credentials)
	for _, secret := range []string{testAccess, testRefresh, testVerifier} {
		if strings.Contains(err.Error(), secret) || strings.Contains(logs.String(), secret) || strings.Contains(formatted, secret) {
			t.Fatalf("敏感信息泄漏: err=%v logs=%s formatted=%s", err, logs.String(), formatted)
		}
	}
}

func TestLogoutRevokesAndClearsLocalCredentials(t *testing.T) {
	var revoked atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/api/v1/auth/logout" || request.Method != http.MethodPost {
			http.NotFound(writer, request)
			return
		}
		var body map[string]string
		_ = json.NewDecoder(request.Body).Decode(&body)
		revoked.Store(body["refresh_token"] == testRefresh)
		writeEnvelope(t, writer, 200, `{"code":0,"data":{}}`)
	}))
	defer server.Close()
	service, _ := NewService(t.TempDir())
	service.credentials = &Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: time.Now().Add(time.Hour).Unix()}
	service.client, _ = NewClient(server.URL, server.Client())
	if err := service.store.Save(*service.credentials); err != nil {
		t.Fatal(err)
	}
	if err := service.Logout(t.Context()); err != nil {
		t.Fatal(err)
	}
	if !revoked.Load() || service.SignedIn() {
		t.Fatal("logout 未撤销会话")
	}
	if _, err := os.Stat(service.store.Path()); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("本地凭据未删除: %v", err)
	}
}

func TestAccountAcceptsNestedPlan(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/api/v1/auth/me" || request.Header.Get("Authorization") != "Bearer "+testAccess {
			t.Errorf("账号请求不符合契约: %s", request.URL.Path)
		}
		writeEnvelope(t, writer, 200, `{"code":0,"data":{"id":7,"email":"dev@example.test","plan":{"name":"专业版"},"balance":12.5}}`)
	}))
	defer server.Close()
	client, _ := NewClient(server.URL, server.Client())
	user, err := client.Me(t.Context(), testAccess)
	if err != nil || user.DisplayName() != "dev@example.test" || user.PlanDisplay() != "专业版" || user.Balance != 12.5 {
		t.Fatalf("账号套餐解析失败: %+v, %v", user, err)
	}
}

func TestPKCEMatchesRFC7636(t *testing.T) {
	const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
	if got := PKCEChallenge(verifier); got != "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM" {
		t.Fatalf("challenge = %q", got)
	}
}

func TestGatewayFetchesCatalogAndSelectsListedKey(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Authorization") != "Bearer "+testAccess {
			t.Errorf("Authorization = %q", request.Header.Get("Authorization"))
		}
		switch request.URL.Path {
		case "/api/v1/models/catalog":
			writeEnvelope(t, writer, 200, `{"code":0,"data":{"base_url":"https://gateway.example/v1","items":[{"model":"gpt-test","display_name":"GPT Test","context_window":64000}]}}`)
		case "/api/v1/keys":
			writeEnvelope(t, writer, 200, `{"code":0,"data":{"items":[{"id":1,"key":"sk-other","status":"active"},{"id":2,"key":"`+testGateway+`","status":"active"}],"total":2}}`)
		default:
			http.NotFound(writer, request)
		}
	}))
	defer server.Close()
	service, _ := NewService(t.TempDir())
	service.credentials = &Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: time.Now().Add(time.Hour).Unix(), APIKey: testGateway}
	service.client, _ = NewClient(server.URL, server.Client())
	if err := service.store.Save(*service.credentials); err != nil {
		t.Fatal(err)
	}
	gateway, err := service.Gateway(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if gateway.BaseURL != "https://gateway.example/v1" || gateway.APIKey != testGateway || len(gateway.Models) != 1 {
		t.Fatalf("网关配置不完整: %s", gateway)
	}
}

// The catalog is routed with one key: the bridge's while the account still
// lists it usable, else an account-level key before one bound to a group.
func TestAccountKeySelection(t *testing.T) {
	group := int64(3)
	key := func(value, status string, bound bool) APIKey {
		listed := APIKey{Key: value, Status: status}
		if bound {
			listed.GroupID = &group
		}
		return listed
	}
	for name, tc := range map[string]struct {
		page         KeyPage
		bridge, want string
	}{
		"the listed bridge key":             {KeyPage{Items: []APIKey{key("sk-account", "active", false), key("sk-bridge", "", true)}, Total: 2}, "sk-bridge", "sk-bridge"},
		"an account-level key over a bound": {KeyPage{Items: []APIKey{key("sk-bound", "active", true), key("sk-account", "active", false)}, Total: 2}, "", "sk-account"},
		"a bound key when it is the only":   {KeyPage{Items: []APIKey{key("sk-bound", "enabled", true)}, Total: 1}, "", "sk-bound"},
		"a deleted bridge key":              {KeyPage{Items: []APIKey{key("sk-account", "active", false)}, Total: 1}, "sk-bridge", "sk-account"},
		"a disabled bridge key":             {KeyPage{Items: []APIKey{key("sk-bridge", "disabled", false), key("sk-bound", "active", true)}, Total: 60}, "sk-bridge", "sk-bound"},
		"a bridge key past the first page":  {KeyPage{Items: []APIKey{key("sk-bound", "active", true)}, Total: 51}, "sk-bridge", "sk-bridge"},
		"a bridge key not listed yet":       {KeyPage{}, "sk-bridge", "sk-bridge"},
		"no usable key":                     {KeyPage{Items: []APIKey{key(" ", "active", false), key("sk-off", "disabled", false), key("sk-spent", "quota_exhausted", true)}, Total: 3}, "", ""},
	} {
		if got := accountKey(tc.page, tc.bridge); got != tc.want {
			t.Errorf("%s: %q, want %q", name, got, tc.want)
		}
	}
}

// The listing's group bindings and total decide the key as the service sends
// them, and a key that can be reused is not minted again.
func TestGatewayReusesAccountLevelKey(t *testing.T) {
	for name, tc := range map[string]struct{ keys, want string }{
		"an account-level key over a bound one": {`{"items":[{"id":1,"key":"sk-bound","group_id":3,"status":"active"},{"id":2,"key":"sk-account","group_id":null,"status":"active"}],"total":2}`, "sk-account"},
		"the bridge key past the first page":    {`{"items":[{"id":1,"key":"sk-bound","group_id":3,"status":"active"}],"total":51}`, testGateway},
	} {
		t.Run(name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
				switch {
				case request.URL.Path == "/api/v1/models/catalog":
					writeEnvelope(t, writer, 200, `{"code":0,"data":{"items":[{"model":"gpt-test"}]}}`)
				case request.URL.Path == "/api/v1/keys" && request.Method == http.MethodGet:
					writeEnvelope(t, writer, 200, `{"code":0,"data":`+tc.keys+`}`)
				default:
					t.Errorf("unexpected %s %s", request.Method, request.URL.Path)
					http.NotFound(writer, request)
				}
			}))
			defer server.Close()
			service, _ := NewService(t.TempDir())
			service.credentials = &Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: time.Now().Add(time.Hour).Unix(), APIKey: testGateway}
			service.client, _ = NewClient(server.URL, server.Client())
			if err := service.store.Save(*service.credentials); err != nil {
				t.Fatal(err)
			}
			gateway, err := service.Gateway(t.Context())
			if err != nil || gateway.APIKey != tc.want {
				t.Fatalf("gateway key %q, %v; want %q", gateway.APIKey, err, tc.want)
			}
		})
	}
}

// TestGatewayMintsKeyWhenAccountOwnsNone pins the new-account path: a
// sign-in must not fail because no key exists yet, and the minted key is the
// one the provider receives.
func TestGatewayMintsKeyWhenAccountOwnsNone(t *testing.T) {
	minted := 0
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Authorization") != "Bearer "+testAccess {
			t.Errorf("Authorization = %q", request.Header.Get("Authorization"))
		}
		switch {
		case request.URL.Path == "/api/v1/models/catalog":
			writeEnvelope(t, writer, 200, `{"code":0,"data":{"base_url":"https://gateway.example/v1","items":[{"model":"gpt-test"}]}}`)
		case request.URL.Path == "/api/v1/keys" && request.Method == http.MethodGet:
			writeEnvelope(t, writer, 200, `{"code":0,"data":{"items":[],"total":0}}`)
		case request.URL.Path == "/api/v1/keys" && request.Method == http.MethodPost:
			minted++
			var body struct {
				Name string `json:"name"`
			}
			if err := json.NewDecoder(request.Body).Decode(&body); err != nil {
				t.Errorf("create key body: %v", err)
			}
			if body.Name != CheapRouterKeyName {
				t.Errorf("minted key name = %q", body.Name)
			}
			writeEnvelope(t, writer, 200, `{"code":0,"data":{"id":9,"key":"sk-minted","status":"active"}}`)
		default:
			http.NotFound(writer, request)
		}
	}))
	defer server.Close()
	service, _ := NewService(t.TempDir())
	service.credentials = &Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: server.URL, ExpiresAt: time.Now().Add(time.Hour).Unix()}
	service.client, _ = NewClient(server.URL, server.Client())
	if err := service.store.Save(*service.credentials); err != nil {
		t.Fatal(err)
	}
	gateway, err := service.Gateway(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if minted != 1 || gateway.APIKey != "sk-minted" {
		t.Fatalf("minted = %d, key = %q", minted, gateway.APIKey)
	}
}
