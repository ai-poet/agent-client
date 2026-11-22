package cheapauth

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestGatewayOriginBuiltInDedupAndLabels(t *testing.T) {
	t.Parallel()
	cfg := DefaultGatewayOriginConfig()
	builtIn := BuiltInEndpoints()
	if !slices.Equal(cfg.EffectiveCandidates(), builtIn) {
		t.Fatalf("effective = %v, want %v", cfg.EffectiveCandidates(), builtIn)
	}
	if len(builtIn) == 0 || builtIn[0] != PrimaryEndpoint {
		t.Fatalf("builtIn = %v, want PrimaryEndpoint first", builtIn)
	}

	// Same host typed differently is not duplicated.
	if added, err := cfg.Add(PrimaryEndpoint + "/"); err != nil || added {
		t.Fatalf("duplicate primary add = (%v, %v)", added, err)
	}
	if added, err := cfg.Add("mirror.example.org"); err != nil || !added {
		t.Fatalf("mirror add = (%v, %v)", added, err)
	}
	if added, err := cfg.Add("https://mirror.example.org/v1"); err != nil || added {
		t.Fatalf("duplicate mirror/v1 add = (%v, %v)", added, err)
	}
	if _, err := cfg.Add("not a url"); err == nil {
		t.Fatal("expected error for invalid URL with whitespace")
	}
	if !cfg.IsRemovable("https://mirror.example.org") {
		t.Fatal("user-added origin should be removable")
	}
	if cfg.IsRemovable(PrimaryEndpoint) || cfg.IsRemovable(CDNEndpoint) {
		t.Fatal("built-in origins must not be removable")
	}

	// Labels: Primary wins over server label; server label wins over built-in CDN.
	if got := cfg.NodeLabel(PrimaryEndpoint); got.Kind != NodeLabelPrimary {
		t.Fatalf("primary label = %+v", got)
	}
	if got := cfg.NodeLabel(CDNEndpoint); got.Kind != NodeLabelBuiltInCDN {
		t.Fatalf("cdn label = %+v", got)
	}
	cfg.AdoptServerNodes([]PublicEndpoint{
		{Name: "CDN Asian", Endpoint: CDNEndpoint, Description: "Asia"},
		{Name: "Main Override", Endpoint: PrimaryEndpoint, Description: "Main"},
	}, 10)
	if got := cfg.NodeLabel(CDNEndpoint); got.Kind != NodeLabelServer || got.Name != "CDN Asian" || got.Description != "Asia" {
		t.Fatalf("cdn server label = %+v", got)
	}
	if got := cfg.NodeLabel(PrimaryEndpoint); got.Kind != NodeLabelPrimary {
		t.Fatalf("primary label overridden by server = %+v", got)
	}
	if got := cfg.NodeLabel("https://mirror.example.org"); got.Kind != NodeLabelOther {
		t.Fatalf("custom label = %+v", got)
	}
}

func TestGatewayOriginServerNodesFilteringTTLAndCap(t *testing.T) {
	t.Parallel()
	cfg := DefaultGatewayOriginConfig()
	if !cfg.ServerNodesDue(0) {
		t.Fatal("empty config must be due for server node refresh")
	}

	// Filtering: only bare HTTPS origins kept, trailing /v1 stripped, deduped, capped at 10.
	listed := []PublicEndpoint{
		{Name: "HK", Endpoint: "https://hk.example.org/", Description: "Hong Kong"},
		{Name: "HK dup", Endpoint: "https://hk.example.org/v1", Description: ""},
		{Name: "PathPrefix", Endpoint: "https://hk.example.org/antigravity", Description: ""},
		{Name: "PlainHTTP", Endpoint: "http://plain.example.org", Description: ""},
		{Name: "Broken", Endpoint: "not a url", Description: ""},
	}
	for i := 1; i <= 15; i++ {
		listed = append(listed, PublicEndpoint{
			Name:        fmt.Sprintf("Node %d", i),
			Endpoint:    fmt.Sprintf("n%d.example.org", i),
			Description: fmt.Sprintf(" Region %d ", i),
		})
	}
	if !cfg.AdoptServerNodes(listed, 1000) {
		t.Fatal("expected initial AdoptServerNodes to report changed")
	}
	if len(cfg.ServerNodes) != MaxServerNodes {
		t.Fatalf("len(ServerNodes) = %d, want %d", len(cfg.ServerNodes), MaxServerNodes)
	}
	if cfg.ServerNodes[0].Origin != "https://hk.example.org" || cfg.ServerNodes[1].Origin != "https://n1.example.org" || cfg.ServerNodes[1].Description != "Region 1" {
		t.Fatalf("unexpected server nodes: %+v", cfg.ServerNodes[:2])
	}
	if cfg.IsRemovable("https://hk.example.org") {
		t.Fatal("server node must not be removable")
	}

	// TTL check: 6 hours.
	if cfg.ServerNodesDue(1000 + ServerNodesTTLSeconds - 1) {
		t.Fatal("server nodes should not be due before TTL")
	}
	if !cfg.ServerNodesDue(1000 + ServerNodesTTLSeconds) {
		t.Fatal("server nodes must be due at TTL boundary")
	}

	// Choosing a server node and then having the server drop it falls back to primary.
	if err := cfg.Select("https://hk.example.org"); err != nil {
		t.Fatal(err)
	}
	if cfg.Origin() != "https://hk.example.org" {
		t.Fatalf("origin = %q", cfg.Origin())
	}
	cfg.AdoptServerNodes(nil, 2000)
	if cfg.Origin() != PrimaryEndpoint || cfg.SelectedOrigin() != "" {
		t.Fatalf("dropped node still routed: origin=%q selected=%q", cfg.Origin(), cfg.SelectedOrigin())
	}
}

func TestGatewayOriginRefreshServerNodesTTLAndFallbackOrigin(t *testing.T) {
	t.Parallel()
	var loginCalls, nodeCalls atomic.Int32
	var loginDown atomic.Bool

	loginSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v1/settings/public" {
			http.NotFound(w, r)
			return
		}
		loginCalls.Add(1)
		if loginDown.Load() {
			w.WriteHeader(http.StatusBadGateway)
			return
		}
		writeEnvelope(t, w, 200, `{"code":0,"data":{"custom_endpoints":[
			{"name":"HK","endpoint":"https://hk.example.org","description":"Asia"},
			{"name":"SG","endpoint":"https://sg.example.org","description":"Singapore"}
		]}}`)
	}))
	defer loginSrv.Close()

	nodeSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v1/settings/public" {
			http.NotFound(w, r)
			return
		}
		nodeCalls.Add(1)
		writeEnvelope(t, w, 200, `{"code":0,"data":{"custom_endpoints":[
			{"name":"JP","endpoint":"https://jp.example.org","description":"Tokyo"}
		]}}`)
	}))
	defer nodeSrv.Close()

	now := time.Unix(1_700_000_000, 0)
	profile := t.TempDir()
	svc, err := NewService(profile)
	if err != nil {
		t.Fatal(err)
	}
	svc.now = func() time.Time { return now }
	creds := Credentials{AccessToken: testAccess, RefreshToken: testRefresh, Endpoint: loginSrv.URL, ExpiresAt: now.Add(time.Hour).Unix()}
	if err := svc.Establish(t.Context(), creds); err != nil {
		t.Fatal(err)
	}

	// First refresh fetches from login endpoint.
	cfg, _, err := svc.RefreshServerNodes(t.Context(), false)
	if err != nil {
		t.Fatal(err)
	}
	if loginCalls.Load() != 1 || len(cfg.ServerNodes) != 2 {
		t.Fatalf("initial refresh: calls=%d nodes=%+v", loginCalls.Load(), cfg.ServerNodes)
	}

	// Within TTL (6h - 1s): skipped without hitting network.
	now = now.Add(time.Duration(ServerNodesTTLSeconds-1) * time.Second)
	_, _, err = svc.RefreshServerNodes(t.Context(), false)
	if err != nil {
		t.Fatal(err)
	}
	if loginCalls.Load() != 1 {
		t.Fatalf("refetched before TTL expiry: calls=%d", loginCalls.Load())
	}

	// After TTL expires, with login endpoint down, falls back to the currently routed node.
	if _, _, err := svc.SelectGatewayOrigin(nodeSrv.URL); err != nil {
		t.Fatal(err)
	}
	loginDown.Store(true)
	now = now.Add(2 * time.Second)
	cfg, _, err = svc.RefreshServerNodes(t.Context(), false)
	if err != nil {
		t.Fatal(err)
	}
	if loginCalls.Load() != 2 || nodeCalls.Load() != 1 {
		t.Fatalf("expected fallback to routed node: login=%d node=%d", loginCalls.Load(), nodeCalls.Load())
	}
	if len(cfg.ServerNodes) != 1 || cfg.ServerNodes[0].Origin != "https://jp.example.org" {
		t.Fatalf("fallback server nodes = %+v", cfg.ServerNodes)
	}
}

func TestGatewayOriginAutoSelectAndFallbackRestore(t *testing.T) {
	t.Parallel()
	okResult := func(url string, ms int64) CandidateResult {
		return CandidateResult{
			URL:    url,
			Result: &ProbeResult{LatencyMS: ms, Status: 200, Verdict: ProbeOK},
		}
	}
	failResult := func(url string) CandidateResult {
		return CandidateResult{
			URL:    url,
			Result: &ProbeResult{LatencyMS: 500, Verdict: ProbeUnreachable},
		}
	}

	t.Run("auto_select picks fastest ok and keeps origin when none ok", func(t *testing.T) {
		cfg := DefaultGatewayOriginConfig()
		cfg.AutoSelect = true
		_, _ = cfg.Add("https://fast.example.org")

		decision := ApplyOriginTest(&cfg, []CandidateResult{
			okResult(PrimaryEndpoint, 420),
			okResult(CDNEndpoint, 180),
			okResult("https://fast.example.org", 65),
		})
		if decision.Action != OriginActionAutoSelected || decision.Target != "https://fast.example.org" || cfg.Origin() != "https://fast.example.org" {
			t.Fatalf("auto_select decision = %+v, origin=%q", decision, cfg.Origin())
		}

		// When all fail, keep current origin and return OriginActionNoOK.
		decision = ApplyOriginTest(&cfg, []CandidateResult{
			failResult(PrimaryEndpoint),
			failResult(CDNEndpoint),
			failResult("https://fast.example.org"),
		})
		if decision.Action != OriginActionNoOK || cfg.Origin() != "https://fast.example.org" {
			t.Fatalf("all-failed decision = %+v, origin=%q", decision, cfg.Origin())
		}
	})

	t.Run("fallback_from remembers original node and restores once healthy", func(t *testing.T) {
		cfg := DefaultGatewayOriginConfig()
		cfg.AutoSelect = false
		_, _ = cfg.Add("https://mirror.example.org")

		// Primary fails -> falls back to fastest ok (CDN) and remembers Primary.
		decision := ApplyOriginTest(&cfg, []CandidateResult{
			failResult(PrimaryEndpoint),
			okResult(CDNEndpoint, 120),
			okResult("https://mirror.example.org", 300),
		})
		if decision.Action != OriginActionFallback || decision.Target != CDNEndpoint {
			t.Fatalf("fallback decision = %+v", decision)
		}
		if cfg.Origin() != CDNEndpoint || cfg.RestoreTarget() != PrimaryEndpoint {
			t.Fatalf("after fallback: origin=%q restore=%q", cfg.Origin(), cfg.RestoreTarget())
		}

		// Second fallback to mirror still remembers the original Primary.
		decision = ApplyOriginTest(&cfg, []CandidateResult{
			failResult(PrimaryEndpoint),
			failResult(CDNEndpoint),
			okResult("https://mirror.example.org", 150),
		})
		if decision.Action != OriginActionFallback || cfg.Origin() != "https://mirror.example.org" || cfg.RestoreTarget() != PrimaryEndpoint {
			t.Fatalf("second fallback: decision=%+v origin=%q restore=%q", decision, cfg.Origin(), cfg.RestoreTarget())
		}

		// Original primary recovers -> restores back to Primary and clears fallback_from.
		decision = ApplyOriginTest(&cfg, []CandidateResult{
			okResult(PrimaryEndpoint, 240),
			okResult(CDNEndpoint, 110),
			okResult("https://mirror.example.org", 150),
		})
		if decision.Action != OriginActionRestored || decision.Target != PrimaryEndpoint {
			t.Fatalf("restore decision = %+v", decision)
		}
		if cfg.Origin() != PrimaryEndpoint || cfg.FallbackFrom != nil || cfg.RestoreTarget() != "" {
			t.Fatalf("after restore: origin=%q fallback_from=%v", cfg.Origin(), cfg.FallbackFrom)
		}

		// Deliberate selection while fallen back cancels the pending return.
		if err := cfg.FallBack(CDNEndpoint); err != nil {
			t.Fatal(err)
		}
		if err := cfg.Select("https://mirror.example.org"); err != nil {
			t.Fatal(err)
		}
		if cfg.RestoreTarget() != "" {
			t.Fatalf("deliberate select kept restore_target=%q", cfg.RestoreTarget())
		}
	})
}

func TestGatewayOriginStorePermissionsAtomicAndCorruptRecovery(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	store, err := NewGatewayOriginStore(dir)
	if err != nil {
		t.Fatal(err)
	}

	// Missing file falls back to defaults.
	if got := store.Load(); got.Origin() != PrimaryEndpoint || got.Network != DefaultNetworkTimeouts() {
		t.Fatalf("missing load = %+v", got)
	}

	// Save full config with 0600 permissions and round-trip.
	chosen := "https://mirror.example.org"
	fb := PrimaryEndpoint
	fetchedAt := int64(1_700_000_123)
	want := GatewayOriginConfig{
		Candidates:           []string{chosen},
		Chosen:               &chosen,
		AutoSelect:           true,
		ServerNodes:          []ServerNode{{Name: "HK", Origin: "https://hk.example.org", Description: "Hong Kong"}},
		ServerNodesFetchedAt: &fetchedAt,
		FallbackFrom:         &fb,
		Network:              NetworkTimeouts{ConnectSecs: 10, IdleSecs: 180},
	}
	if err := store.Save(want); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(store.Path())
	if err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" && info.Mode().Perm() != 0o600 {
		t.Fatalf("gateway-origin.json mode = %04o, want 0600", info.Mode().Perm())
	}
	// Verify atomic write left no temp files behind.
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Name() != GatewayOriginFileName {
		t.Fatalf("unexpected files in dir: %+v", entries)
	}

	got := store.Load()
	rawWant, _ := json.Marshal(want)
	rawGot, _ := json.Marshal(got)
	if string(rawWant) != string(rawGot) {
		t.Fatalf("round-trip mismatch:\n got: %s\nwant: %s", rawGot, rawWant)
	}

	// Corrupt JSON falls back cleanly to default without crashing.
	if err := os.WriteFile(filepath.Join(dir, GatewayOriginFileName), []byte("{corrupt json"), 0o600); err != nil {
		t.Fatal(err)
	}
	recovered := store.Load()
	if recovered.Origin() != PrimaryEndpoint || recovered.AutoSelect || len(recovered.Candidates) != 0 {
		t.Fatalf("corrupt recovery = %+v", recovered)
	}

	// Legacy JSON (without server_nodes/fallback_from/network) still loads cleanly.
	legacy := `{"candidates":["https://legacy.example.org"],"chosen":"https://legacy.example.org","auto_select":false}`
	if err := os.WriteFile(store.Path(), []byte(legacy), 0o600); err != nil {
		t.Fatal(err)
	}
	loadedLegacy := store.Load()
	if loadedLegacy.Origin() != "https://legacy.example.org" || loadedLegacy.Network != DefaultNetworkTimeouts() {
		t.Fatalf("legacy load = %+v", loadedLegacy)
	}
}

// Verifies the core Rust semantic:
//   - Auth requests (refresh, logout) ALWAYS hit the login endpoint, even when
//     chosen points to another gateway node.
//   - Business requests (account, catalog, groups, group-status, usage, payment,
//     images, and Gateway.BaseURL for the built-in agent) ALWAYS hit chosen.
//   - Switching gateway origin never rewrites credentials.Endpoint.
func TestAuthStaysOnLoginOriginWhileBusinessTrafficUsesChosenOrigin(t *testing.T) {
	t.Parallel()
	var loginMu, gwMu sync.Mutex
	var loginPaths, gwPaths []string

	record := func(mu *sync.Mutex, dst *[]string, r *http.Request) {
		mu.Lock()
		*dst = append(*dst, r.Method+" "+r.URL.Path)
		mu.Unlock()
	}

	loginSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		record(&loginMu, &loginPaths, r)
		switch r.URL.Path {
		case "/api/v1/auth/refresh":
			writeEnvelope(t, w, 200, `{"code":0,"data":{"access_token":"refreshed-access","refresh_token":"refreshed-refresh","expires_in":900}}`)
		case "/api/v1/auth/logout":
			writeEnvelope(t, w, 200, `{"code":0,"data":{}}`)
		default:
			t.Errorf("business request %s %s leaked to login origin", r.Method, r.URL.Path)
			http.NotFound(w, r)
		}
	}))
	defer loginSrv.Close()

	gwSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		record(&gwMu, &gwPaths, r)
		switch r.URL.Path {
		case "/api/v1/auth/refresh", "/api/v1/auth/logout", "/api/v1/auth/desktop-session/exchange":
			t.Errorf("auth request %s %s leaked to gateway node", r.Method, r.URL.Path)
			http.Error(w, "auth forbidden on gateway node", http.StatusForbidden)
		case "/api/v1/auth/me":
			writeEnvelope(t, w, 200, `{"code":0,"data":{"id":9,"email":"node@example.test","username":"NodeUser","balance":42.5}}`)
		case "/api/v1/models/catalog":
			// Even when the server reports a different primary base_url, the
			// explicit gateway origin override must win for Gateway.BaseURL.
			writeEnvelope(t, w, 200, `{"code":0,"data":{"base_url":"https://primary.example/v1","items":[{"model":"claude-sonnet-4","platform":"anthropic"}]}}`)
		case "/api/v1/keys":
			writeEnvelope(t, w, 200, `{"code":0,"data":{"items":[{"id":1,"key":"`+testGateway+`","status":"active"}],"total":1}}`)
		case "/api/v1/groups/available":
			writeEnvelope(t, w, 200, `{"code":0,"data":[{"id":1,"name":"Claude","platform":"anthropic","status":"active"}]}`)
		case "/api/v1/group-status":
			writeEnvelope(t, w, 200, `{"code":0,"data":[{"group_id":1,"latest_status":"healthy"}]}`)
		case "/api/v1/usage/stats":
			writeEnvelope(t, w, 200, `{"code":0,"data":{"total_requests":7,"total_tokens":1200,"total_cost":0.12}}`)
		case "/pay/api/orders/my":
			w.Header().Set("Content-Type", "application/json")
			_, _ = io.WriteString(w, `{"user":{"id":9,"username":"NodeUser","balance":42.5},"summary":{"pending":0}}`)
		case "/pay/api/user":
			w.Header().Set("Content-Type", "application/json")
			_, _ = io.WriteString(w, `{"config":{"enabledPaymentTypes":["alipay"]}}`)
		default:
			http.NotFound(w, r)
		}
	}))
	defer gwSrv.Close()

	now := time.Unix(1_700_000_000, 0)
	profile := t.TempDir()
	svc, err := NewService(profile)
	if err != nil {
		t.Fatal(err)
	}
	svc.now = func() time.Time { return now }

	// Establish credentials that are due for refresh so the first business call
	// exercises both auth renewal (on loginSrv) and business traffic (on gwSrv).
	creds := Credentials{
		AccessToken:  testAccess,
		RefreshToken: testRefresh,
		Endpoint:     loginSrv.URL,
		ExpiresAt:    now.Unix() + 30, // inside RefreshSkew -> triggers /auth/refresh
		APIKey:       testGateway,
	}
	if err := svc.Establish(t.Context(), creds); err != nil {
		t.Fatal(err)
	}

	// Switch gateway origin to gwSrv without changing credentials.Endpoint.
	if _, changed, err := svc.SelectGatewayOrigin(gwSrv.URL); err != nil || !changed {
		t.Fatalf("SelectGatewayOrigin = (%v, %v)", changed, err)
	}
	if svc.Endpoint() != loginSrv.URL {
		t.Fatalf("login Endpoint rewritten to %q, want %q", svc.Endpoint(), loginSrv.URL)
	}
	if svc.GatewayOrigin() != gwSrv.URL {
		t.Fatalf("GatewayOrigin = %q, want %q", svc.GatewayOrigin(), gwSrv.URL)
	}

	// Business calls all go to gwSrv (after refresh on loginSrv).
	user, err := svc.Account(t.Context())
	if err != nil || user.Email != "node@example.test" {
		t.Fatalf("Account = (%+v, %v)", user, err)
	}
	gw, err := svc.Gateway(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if gw.BaseURL != gwSrv.URL+"/v1" {
		t.Fatalf("Gateway.BaseURL = %q, want %q", gw.BaseURL, gwSrv.URL+"/v1")
	}
	groups, err := svc.AvailableGroups(t.Context())
	if err != nil || len(groups) != 1 {
		t.Fatalf("AvailableGroups = (%+v, %v)", groups, err)
	}
	statuses, err := svc.GroupStatuses(t.Context())
	if err != nil || len(statuses) != 1 {
		t.Fatalf("GroupStatuses = (%+v, %v)", statuses, err)
	}
	stats, err := svc.UsageStats(t.Context(), "today")
	if err != nil || stats.TotalRequests != 7 {
		t.Fatalf("UsageStats = (%+v, %v)", stats, err)
	}
	payClient, token, err := svc.PaySession(t.Context(), "zh-CN")
	if err != nil || payClient.Endpoint() != gwSrv.URL {
		t.Fatalf("PaySession endpoint = %q, err = %v", payClient.Endpoint(), err)
	}
	if _, err := payClient.LoadConfig(t.Context(), token); err != nil {
		t.Fatalf("PayClient.LoadConfig = %v", err)
	}
	imgOrigin, imgKey, err := svc.ImageRouteKey(t.Context(), nil)
	if err != nil || imgOrigin != gwSrv.URL || imgKey != testGateway {
		t.Fatalf("ImageRouteKey = (%q, %q, %v)", imgOrigin, imgKey, err)
	}

	// Stored credential endpoint was never altered.
	stored, err := svc.store.Load()
	if err != nil || stored == nil || stored.Endpoint != loginSrv.URL {
		t.Fatalf("stored credentials endpoint mutated: %+v, %v", stored, err)
	}

	// Logout revocation goes to loginSrv, never gwSrv.
	if err := svc.Logout(t.Context()); err != nil {
		t.Fatalf("Logout = %v", err)
	}

	loginMu.Lock()
	gotLogin := append([]string(nil), loginPaths...)
	loginMu.Unlock()
	if !slices.Contains(gotLogin, "POST /api/v1/auth/refresh") || !slices.Contains(gotLogin, "POST /api/v1/auth/logout") {
		t.Fatalf("login origin missing auth calls: %v", gotLogin)
	}

	gwMu.Lock()
	gotGW := append([]string(nil), gwPaths...)
	gwMu.Unlock()
	for _, wantPath := range []string{
		"GET /api/v1/auth/me",
		"GET /api/v1/models/catalog",
		"GET /api/v1/keys",
		"GET /api/v1/groups/available",
		"GET /api/v1/group-status",
		"GET /api/v1/usage/stats",
		"GET /pay/api/orders/my",
		"GET /pay/api/user",
	} {
		if !slices.Contains(gotGW, wantPath) {
			t.Errorf("gateway node missing business call %q; got %v", wantPath, gotGW)
		}
	}
}

func TestSpeedTestConcurrencyHeadersOrderAndSorting(t *testing.T) {
	t.Parallel()
	if got := ClampSpeedTestTimeout(0); got != DefaultSpeedTestTimeoutSecs {
		t.Fatalf("clamp(0) = %d", got)
	}
	if got := ClampSpeedTestTimeout(1); got != MinSpeedTestTimeoutSecs {
		t.Fatalf("clamp(1) = %d", got)
	}
	if got := ClampSpeedTestTimeout(999); got != MaxSpeedTestTimeoutSecs {
		t.Fatalf("clamp(999) = %d", got)
	}
	if ClassifyLatency(100) != LatencyFast || ClassifyLatency(350) != LatencyOK ||
		ClassifyLatency(600) != LatencySlow || ClassifyLatency(900) != LatencyVerySlow {
		t.Fatal("unexpected latency tier classification")
	}

	// Slow server (120ms), fast server (0ms), unauthorized server (401).
	var claudeHeaderSeen atomic.Bool
	slow := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/models" {
			http.NotFound(w, r)
			return
		}
		if r.Header.Get("x-api-key") == "sk-probe" && r.Header.Get("anthropic-version") == "2023-06-01" {
			claudeHeaderSeen.Store(true)
		}
		time.Sleep(120 * time.Millisecond)
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"data":[{"id":"claude-sonnet"}]}`)
	}))
	defer slow.Close()

	fast := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/models" {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = io.WriteString(w, `{"data":[{"id":"claude-fast"}]}`)
	}))
	defer fast.Close()

	unauth := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = io.WriteString(w, `{"error":{"message":"bad key"}}`)
	}))
	defer unauth.Close()

	urls := []string{slow.URL, "not a url", unauth.URL, fast.URL}
	results := TestCandidates(t.Context(), "claude", urls, "sk-probe", 5, nil)
	if len(results) != 4 {
		t.Fatalf("len(results) = %d, want 4", len(results))
	}
	if !claudeHeaderSeen.Load() {
		t.Fatal("claude probe did not send x-api-key + anthropic-version headers")
	}
	// Input order preserved.
	if results[0].URL != slow.URL || !results[0].IsOK() {
		t.Fatalf("results[0] = %+v", results[0])
	}
	if results[1].Invalid == nil || results[1].Result != nil {
		t.Fatalf("results[1] should be invalid: %+v", results[1])
	}
	if results[2].Result == nil || results[2].Result.Verdict != ProbeUnauthorized {
		t.Fatalf("results[2] should be unauthorized: %+v", results[2])
	}
	if results[3].URL != fast.URL || !results[3].IsOK() {
		t.Fatalf("results[3] = %+v", results[3])
	}
	if got := FastestOK(results); got != 3 {
		t.Fatalf("FastestOK = %d, want 3", got)
	}
	if ids := ModelIDsFromBody(results[3].Result.Body); !slices.Equal(ids, []string{"claude-fast"}) {
		t.Fatalf("ModelIDsFromBody = %v", ids)
	}

	// Sorted order: fast OK -> slow OK -> unauthorized -> invalid.
	sorted := SortCandidateResults(results)
	if sorted[0].URL != fast.URL || sorted[1].URL != slow.URL || sorted[2].URL != unauth.URL || sorted[3].Invalid == nil {
		t.Fatalf("unexpected sorted order: %+v", sorted)
	}

	// OpenAI probe shape sends Bearer token.
	var bearerSeen atomic.Bool
	oai := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/v1/models" && r.Header.Get("Authorization") == "Bearer sk-oai" {
			bearerSeen.Store(true)
		}
		_, _ = io.WriteString(w, `{"data":[{"id":"gpt-5"}]}`)
	}))
	defer oai.Close()
	oaiRes := TestCandidates(t.Context(), "codex", []string{oai.URL}, "sk-oai", 5, nil)
	if len(oaiRes) != 1 || !oaiRes[0].IsOK() || !bearerSeen.Load() {
		t.Fatalf("codex probe = %+v, bearer=%v", oaiRes, bearerSeen.Load())
	}

	// Unselected origin preserves self-hosted login endpoint.
	if got := DefaultGatewayOriginConfig().ResolveAPIOrigin("https://self.example.org"); got != "https://self.example.org" {
		t.Fatalf("ResolveAPIOrigin = %q", got)
	}
	if got := ProbeTargetSummary(); got == "" {
		t.Fatal("unreachable sanity")
	}
}

func ProbeTargetSummary() string {
	kind, key, ok := ProbeTarget(Credentials{
		LaneBindings: map[string]LaneBinding{
			LaneClaude: {GroupID: 1, APIKey: "sk-ant"},
		},
		APIKey: "sk-gen",
	})
	if !ok || kind != "claude" || key != "sk-ant" {
		return ""
	}
	kind2, key2, ok2 := ProbeTarget(Credentials{APIKey: "sk-gen"})
	if !ok2 || kind2 != "codex" || key2 != "sk-gen" {
		return ""
	}
	if _, _, ok3 := ProbeTarget(Credentials{APIKey: "  "}); ok3 {
		return ""
	}
	return strings.Join([]string{kind, kind2}, ",")
}
