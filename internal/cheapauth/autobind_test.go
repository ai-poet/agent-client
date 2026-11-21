package cheapauth

import (
	"encoding/json"
	"fmt"
	"maps"
	"net/http"
	"net/http/httptest"
	"slices"
	"sync"
	"testing"
	"time"
)

type autoBindFixture struct {
	server *httptest.Server

	mu              sync.Mutex
	catalog         ModelCatalog
	keys            []APIKey
	failCreate      map[int64]bool
	keysGETCount    int
	catalogGETCount int
	createdGroups   []int64
	nextKeyID       int64
}

func newAutoBindFixture(t *testing.T, catalog ModelCatalog, keys []APIKey) *autoBindFixture {
	t.Helper()
	f := &autoBindFixture{
		catalog:    catalog,
		keys:       slices.Clone(keys),
		failCreate: map[int64]bool{},
		nextKeyID:  100,
	}
	f.server = httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Authorization") != "Bearer "+testAccess {
			writeEnvelope(t, writer, http.StatusUnauthorized, `{"code":401}`)
			return
		}
		switch {
		case request.URL.Path == "/api/v1/models/catalog" && request.Method == http.MethodGet:
			f.mu.Lock()
			f.catalogGETCount++
			cat := f.catalog
			f.mu.Unlock()
			raw, _ := json.Marshal(map[string]any{"code": 0, "data": cat})
			writeEnvelope(t, writer, http.StatusOK, string(raw))
		case request.URL.Path == "/api/v1/keys" && request.Method == http.MethodGet:
			f.mu.Lock()
			f.keysGETCount++
			items := slices.Clone(f.keys)
			f.mu.Unlock()
			raw, _ := json.Marshal(map[string]any{
				"code": 0,
				"data": KeyPage{Items: items, Total: int64(len(items))},
			})
			writeEnvelope(t, writer, http.StatusOK, string(raw))
		case request.URL.Path == "/api/v1/keys" && request.Method == http.MethodPost:
			var body struct {
				Name    string `json:"name"`
				GroupID *int64 `json:"group_id"`
			}
			if err := json.NewDecoder(request.Body).Decode(&body); err != nil {
				t.Errorf("decode POST /keys body: %v", err)
			}
			var gid int64
			if body.GroupID != nil {
				gid = *body.GroupID
			}
			f.mu.Lock()
			f.createdGroups = append(f.createdGroups, gid)
			fail := f.failCreate[gid]
			if fail {
				f.mu.Unlock()
				writeEnvelope(t, writer, http.StatusInternalServerError, `{"code":500,"message":"mint failed"}`)
				return
			}
			f.nextKeyID++
			id := f.nextKeyID
			keyStr := fmt.Sprintf("sk-group-%d-minted", gid)
			created := APIKey{
				ID:      id,
				Key:     keyStr,
				Name:    body.Name,
				GroupID: cloneInt64Ptr(body.GroupID),
				Status:  "active",
			}
			f.keys = append(f.keys, created)
			f.mu.Unlock()
			raw, _ := json.Marshal(map[string]any{"code": 0, "data": created})
			writeEnvelope(t, writer, http.StatusOK, string(raw))
		default:
			http.NotFound(writer, request)
		}
	}))
	t.Cleanup(f.server.Close)
	return f
}

func (f *autoBindFixture) snapshot() (keysGET, catalogGET int, created []int64) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.keysGETCount, f.catalogGETCount, slices.Clone(f.createdGroups)
}

func newTestSignedInService(t *testing.T, profile, endpoint string, creds Credentials) *Service {
	t.Helper()
	if creds.AccessToken == "" {
		creds.AccessToken = testAccess
	}
	if creds.RefreshToken == "" {
		creds.RefreshToken = testRefresh
	}
	if creds.ExpiresAt == 0 {
		creds.ExpiresAt = time.Now().Add(time.Hour).Unix()
	}
	if creds.Endpoint == "" {
		creds.Endpoint = endpoint
	}
	if creds.APIKey == "" {
		creds.APIKey = testGateway
	}
	service, err := NewService(profile)
	if err != nil {
		t.Fatal(err)
	}
	if err := service.Establish(t.Context(), creds); err != nil {
		t.Fatal(err)
	}
	return service
}

func fiveLaneFixtureData() ([]Group, ModelCatalog) {
	groups := []Group{
		{ID: 10, Name: "Claude Disabled", Platform: "anthropic", Status: "disabled"},
		{ID: 11, Name: "Claude First", Platform: "anthropic", Status: "active"},
		{ID: 12, Name: "Claude Second", Platform: "anthropic", Status: "active"},
		{ID: 21, Name: "Codex First", Platform: "openai", Status: "active"},
		{ID: 22, Name: "Codex Second", Platform: "openai", Status: "active"},
		{ID: 31, Name: "Gemini First", Platform: "gemini", Status: "active"},
		{ID: 41, Name: "国模直连组", Platform: "openai", Status: "active"},
		{ID: 42, Name: "国模备用组", Platform: "domestic", Status: "active"},
		{ID: 51, Name: "Grok First", Platform: "xai", Status: "active"},
	}
	catalog := ModelCatalog{
		BaseURL: "https://gateway.example/v1",
		Items: []Model{
			{ID: "claude-sonnet-4-6", BillingMode: "token", ContextWindow: 200000, BestGroup: GroupRef{ID: 11}},
			{ID: "claude-opus-4-6", BillingMode: "token", ContextWindow: 200000, BestGroup: GroupRef{ID: 12}},
			{ID: "gpt-5", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 21}},
			{ID: "gemini-2.5-pro", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 31}},
			{ID: "deepseek-r1", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 41}},
			{ID: "grok-3", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 51}},
		},
	}
	return groups, catalog
}

func TestFirstRoutableGroupForLane(t *testing.T) {
	groups := []Group{
		{ID: 0, Name: "Zero ID", Platform: "anthropic", Status: "active"},
		{ID: 10, Name: "Claude Disabled", Platform: "anthropic", Status: "disabled"},
		{ID: 25, Name: "Claude First Active", Platform: "anthropic", Status: "active"},
		{ID: 15, Name: "Claude Lower ID Later", Platform: "anthropic", Status: "active"},
		{ID: 40, Name: "DeepSeek Only", Platform: "openai", Status: ""},
		{ID: 21, Name: "Codex Active", Platform: "openai", Status: "enabled"},
		{ID: 30, Name: "Gemini Inactive", Platform: "gemini", Status: "inactive"},
	}
	catalog := []Model{
		{ID: "deepseek-chat", BestGroup: GroupRef{ID: 40}},
	}

	gotClaude, ok := FirstRoutableGroupForLane(LaneClaude, groups, catalog)
	if !ok || gotClaude.ID != 25 {
		t.Fatalf("Claude first routable = (%+v, %v), want ID 25", gotClaude, ok)
	}
	gotDomestic, ok := FirstRoutableGroupForLane(LaneDomestic, groups, catalog)
	if !ok || gotDomestic.ID != 40 {
		t.Fatalf("Domestic first routable = (%+v, %v), want ID 40", gotDomestic, ok)
	}
	gotCodex, ok := FirstRoutableGroupForLane(LaneCodex, groups, catalog)
	if !ok || gotCodex.ID != 21 {
		t.Fatalf("Codex first routable = (%+v, %v), want ID 21", gotCodex, ok)
	}
	if _, ok := FirstRoutableGroupForLane(LaneGemini, groups, catalog); ok {
		t.Fatal("Gemini should have no routable group when only inactive groups exist")
	}
	if _, ok := FirstRoutableGroupForLane(LaneGrok, groups, catalog); ok {
		t.Fatal("Grok should have no routable group when absent")
	}
}

func TestAutoBindUnboundLanesBindsFirstRoutableAndReusesKeys(t *testing.T) {
	groups, catalog := fiveLaneFixtureData()
	g11, g21 := int64(11), int64(21)
	fixture := newAutoBindFixture(t, catalog, []APIKey{
		{ID: 1, Key: testGateway, Status: "active"},
		{ID: 2, Key: "sk-claude-existing", GroupID: &g11, Status: "active"},
		{ID: 3, Key: "sk-codex-disabled", GroupID: &g21, Status: "disabled"},
	})
	profile := t.TempDir()
	service := newTestSignedInService(t, profile, fixture.server.URL, Credentials{})

	gw, res, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil {
		t.Fatalf("AutoBindUnboundLanes: %v", err)
	}
	if len(res.Failures) != 0 {
		t.Fatalf("unexpected auto-bind failures: %+v", res.Failures)
	}
	wantBound := map[string]int64{
		LaneClaude:   11,
		LaneCodex:    21,
		LaneGemini:   31,
		LaneDomestic: 41,
		LaneGrok:     51,
	}
	if !maps.Equal(res.Bound, wantBound) {
		t.Fatalf("res.Bound = %v, want %v", res.Bound, wantBound)
	}
	_, catalogGET, created := fixture.snapshot()
	if catalogGET != 0 {
		t.Fatalf("catalogGET = %d, want 0 when pre-fetched catalog is supplied", catalogGET)
	}
	if !slices.Equal(created, []int64{21, 31, 41, 51}) {
		t.Fatalf("POST /keys group IDs = %v, want [21 31 41 51] (group 11 must reuse existing key)", created)
	}
	if gw.LaneKeys[LaneClaude] != "sk-claude-existing" || gw.LaneKeys[LaneCodex] != "sk-group-21-minted" {
		t.Fatalf("unexpected gw.LaneKeys: %+v", gw.LaneKeys)
	}
	if !maps.Equal(gw.LaneBindings, wantBound) {
		t.Fatalf("gw.LaneBindings = %v, want %v", gw.LaneBindings, wantBound)
	}

	reopened, err := NewService(profile)
	if err != nil || reopened.Restore() != nil {
		t.Fatalf("reopen service: %v", err)
	}
	if got := reopened.LaneBindings(); !maps.Equal(got, wantBound) {
		t.Fatalf("persisted LaneBindings = %v, want %v", got, wantBound)
	}
}

func TestAutoBindUnboundLanesPreservesExistingBindings(t *testing.T) {
	groups, catalog := fiveLaneFixtureData()
	g12, g42 := int64(12), int64(42)
	fixture := newAutoBindFixture(t, catalog, []APIKey{
		{ID: 1, Key: testGateway, Status: "active"},
		{ID: 2, Key: "sk-user-claude-12", GroupID: &g12, Status: "active"},
		{ID: 3, Key: "sk-user-domestic-42", GroupID: &g42, Status: "active"},
	})
	service := newTestSignedInService(t, t.TempDir(), fixture.server.URL, Credentials{
		LaneBindings: map[string]LaneBinding{
			LaneClaude:   {GroupID: 12, APIKey: "sk-user-claude-12"},
			LaneDomestic: {GroupID: 42, APIKey: "sk-user-domestic-42"},
		},
	})

	gw, res, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil {
		t.Fatalf("AutoBindUnboundLanes: %v", err)
	}
	wantNew := map[string]int64{
		LaneCodex:  21,
		LaneGemini: 31,
		LaneGrok:   51,
	}
	if !maps.Equal(res.Bound, wantNew) {
		t.Fatalf("res.Bound = %v, want %v", res.Bound, wantNew)
	}
	wantAll := map[string]int64{
		LaneClaude:   12,
		LaneCodex:    21,
		LaneGemini:   31,
		LaneDomestic: 42,
		LaneGrok:     51,
	}
	if got := service.LaneBindings(); !maps.Equal(got, wantAll) {
		t.Fatalf("service.LaneBindings = %v, want %v", got, wantAll)
	}
	if gw.LaneBindings[LaneClaude] != 12 || gw.LaneBindings[LaneDomestic] != 42 {
		t.Fatalf("existing user bindings overwritten in gateway: %v", gw.LaneBindings)
	}
	_, _, created := fixture.snapshot()
	if !slices.Equal(created, []int64{21, 31, 51}) {
		t.Fatalf("POST /keys group IDs = %v, want [21 31 51]", created)
	}
}

func TestAutoBindUnboundLanesPreservesLegacyGroupBinding(t *testing.T) {
	groups, catalog := fiveLaneFixtureData()
	g12 := int64(12)
	fixture := newAutoBindFixture(t, catalog, []APIKey{
		{ID: 1, Key: testGateway, Status: "active"},
		{ID: 2, Key: "sk-legacy-12", GroupID: &g12, Status: "active"},
	})
	service := newTestSignedInService(t, t.TempDir(), fixture.server.URL, Credentials{
		GroupID: &g12,
	})

	_, res, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil {
		t.Fatalf("AutoBindUnboundLanes: %v", err)
	}
	if _, reBoundClaude := res.Bound[LaneClaude]; reBoundClaude {
		t.Fatalf("legacy bound Claude lane was overwritten: %v", res.Bound)
	}
	if got := service.LaneBindings(); got[LaneClaude] != 12 || got[LaneCodex] != 21 {
		t.Fatalf("LaneBindings after legacy migration + auto-bind = %v", got)
	}
}

func TestAutoBindUnboundLanesSkipsLanesWithoutAvailableGroups(t *testing.T) {
	groups := []Group{
		{ID: 11, Name: "Claude First", Platform: "anthropic", Status: "active"},
		{ID: 31, Name: "Gemini Disabled", Platform: "gemini", Status: "disabled"},
	}
	catalog := ModelCatalog{
		BaseURL: "https://gateway.example/v1",
		Items: []Model{
			{ID: "claude-sonnet-4-6", BillingMode: "token", ContextWindow: 200000, BestGroup: GroupRef{ID: 11}},
		},
	}
	fixture := newAutoBindFixture(t, catalog, []APIKey{{ID: 1, Key: testGateway, Status: "active"}})
	service := newTestSignedInService(t, t.TempDir(), fixture.server.URL, Credentials{})

	_, res, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil {
		t.Fatalf("AutoBindUnboundLanes: %v", err)
	}
	if len(res.Failures) != 0 {
		t.Fatalf("unexpected failures: %+v", res.Failures)
	}
	if !maps.Equal(res.Bound, map[string]int64{LaneClaude: 11}) {
		t.Fatalf("res.Bound = %v, want {anthropic: 11}", res.Bound)
	}
	for _, skipped := range []string{LaneCodex, LaneGemini, LaneDomestic, LaneGrok} {
		if !slices.Contains(res.Skipped, skipped) {
			t.Errorf("expected lane %q in Skipped (%v)", skipped, res.Skipped)
		}
	}

	keysBefore, catBefore, _ := fixture.snapshot()
	_, emptyRes, err := service.AutoBindUnboundLanes(t.Context(), nil, &catalog, nil)
	if err != nil || len(emptyRes.Bound) != 0 || len(emptyRes.Failures) != 0 {
		t.Fatalf("empty groups result = %+v, err=%v", emptyRes, err)
	}
	keysAfter, catAfter, _ := fixture.snapshot()
	if keysAfter != keysBefore || catAfter != catBefore {
		t.Fatal("empty groups should make zero HTTP requests")
	}
}

func TestAutoBindUnboundLanesEnsureFailureDoesNotAffectOtherLanes(t *testing.T) {
	groups, catalog := fiveLaneFixtureData()
	fixture := newAutoBindFixture(t, catalog, []APIKey{{ID: 1, Key: testGateway, Status: "active"}})
	fixture.failCreate[21] = true
	service := newTestSignedInService(t, t.TempDir(), fixture.server.URL, Credentials{})

	gw, res, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil {
		t.Fatalf("AutoBindUnboundLanes should isolate per-lane failure, got err: %v", err)
	}
	wantBound := map[string]int64{
		LaneClaude:   11,
		LaneGemini:   31,
		LaneDomestic: 41,
		LaneGrok:     51,
	}
	if !maps.Equal(res.Bound, wantBound) {
		t.Fatalf("res.Bound = %v, want %v", res.Bound, wantBound)
	}
	if len(res.Failures) != 1 || res.Failures[0].Lane != LaneCodex || res.Failures[0].GroupID != 21 || res.Failures[0].Err == nil {
		t.Fatalf("res.Failures = %+v, want single failure for Codex group 21", res.Failures)
	}
	if !maps.Equal(gw.LaneBindings, wantBound) {
		t.Fatalf("gw.LaneBindings = %v, want %v", gw.LaneBindings, wantBound)
	}
	if got := service.LaneBindings(); !maps.Equal(got, wantBound) {
		t.Fatalf("service.LaneBindings = %v, want %v", got, wantBound)
	}

	t.Run("all lanes failing does not return fatal error", func(t *testing.T) {
		allFailGroups := []Group{
			{ID: 11, Name: "Claude", Platform: "anthropic", Status: "active"},
			{ID: 21, Name: "Codex", Platform: "openai", Status: "active"},
		}
		allFailFixture := newAutoBindFixture(t, catalog, []APIKey{{ID: 1, Key: testGateway, Status: "active"}})
		allFailFixture.failCreate[11] = true
		allFailFixture.failCreate[21] = true
		svc := newTestSignedInService(t, t.TempDir(), allFailFixture.server.URL, Credentials{})

		_, failRes, failErr := svc.AutoBindUnboundLanes(t.Context(), allFailGroups, &catalog, nil)
		if failErr != nil {
			t.Fatalf("all-lane ensure failure returned fatal err: %v", failErr)
		}
		if len(failRes.Bound) != 0 || len(failRes.Failures) != 2 || svc.LaneBindings() != nil {
			t.Fatalf("unexpected all-fail state: res=%+v bindings=%v", failRes, svc.LaneBindings())
		}
	})
}

func TestAutoBindUnboundLanesIdempotentAcrossRepeatedSyncAndRelogin(t *testing.T) {
	groups := []Group{
		{ID: 11, Name: "Claude First", Platform: "anthropic", Status: "active"},
		{ID: 12, Name: "Claude Second", Platform: "anthropic", Status: "active"},
		{ID: 21, Name: "Codex First", Platform: "openai", Status: "active"},
	}
	catalog := ModelCatalog{
		BaseURL: "https://gateway.example/v1",
		Items: []Model{
			{ID: "claude-sonnet-4-6", BillingMode: "token", ContextWindow: 200000, BestGroup: GroupRef{ID: 11}},
			{ID: "gpt-5", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 21}},
		},
	}
	fixture := newAutoBindFixture(t, catalog, []APIKey{{ID: 1, Key: testGateway, Status: "active"}})
	profile := t.TempDir()
	service := newTestSignedInService(t, profile, fixture.server.URL, Credentials{})

	// First sync auto-binds both lanes and mints keys.
	_, res1, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || !maps.Equal(res1.Bound, map[string]int64{LaneClaude: 11, LaneCodex: 21}) {
		t.Fatalf("pass 1 = %+v, err=%v", res1, err)
	}
	keysAfterPass1, _, createdAfterPass1 := fixture.snapshot()
	if !slices.Equal(createdAfterPass1, []int64{11, 21}) {
		t.Fatalf("pass 1 created = %v, want [11 21]", createdAfterPass1)
	}

	// Second sync in same session is a pure no-op (zero extra HTTP calls).
	_, res2, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || len(res2.Bound) != 0 || len(res2.Failures) != 0 {
		t.Fatalf("pass 2 should be no-op, got %+v, err=%v", res2, err)
	}
	keysAfterPass2, _, createdAfterPass2 := fixture.snapshot()
	if keysAfterPass2 != keysAfterPass1 || !slices.Equal(createdAfterPass2, createdAfterPass1) {
		t.Fatalf("pass 2 performed redundant key operations: keysGET=%d->%d created=%v", keysAfterPass1, keysAfterPass2, createdAfterPass2)
	}

	// User switches Claude to group 12; repeated sync must not switch back to 11.
	if err := service.SetLaneBinding(t.Context(), LaneClaude, &LaneBinding{GroupID: 12, APIKey: "sk-custom-12"}); err != nil {
		t.Fatal(err)
	}
	_, res3, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || len(res3.Bound) != 0 || service.LaneBindings()[LaneClaude] != 12 {
		t.Fatalf("pass 3 overwrote user switch: res=%+v bindings=%v err=%v", res3, service.LaneBindings(), err)
	}

	// Repeated login (fresh credentials established): binds first groups and reuses active server keys without re-minting.
	if err := service.Establish(t.Context(), Credentials{
		AccessToken:  testAccess,
		RefreshToken: testRefresh,
		ExpiresAt:    time.Now().Add(time.Hour).Unix(),
		Endpoint:     fixture.server.URL,
		APIKey:       testGateway,
	}); err != nil {
		t.Fatal(err)
	}
	_, res4, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || !maps.Equal(res4.Bound, map[string]int64{LaneClaude: 11, LaneCodex: 21}) {
		t.Fatalf("re-login pass = %+v, err=%v", res4, err)
	}
	_, _, createdAfterRelogin := fixture.snapshot()
	if !slices.Equal(createdAfterRelogin, []int64{11, 21}) {
		t.Fatalf("re-login minted duplicate keys: %v, want [11 21]", createdAfterRelogin)
	}
}

func TestAutoBindUnboundLanesRespectsManualClearUntilNextLogin(t *testing.T) {
	groups := []Group{
		{ID: 11, Name: "Claude First", Platform: "anthropic", Status: "active"},
		{ID: 21, Name: "Codex First", Platform: "openai", Status: "active"},
	}
	catalog := ModelCatalog{
		BaseURL: "https://gateway.example/v1",
		Items: []Model{
			{ID: "claude-sonnet-4-6", BillingMode: "token", ContextWindow: 200000, BestGroup: GroupRef{ID: 11}},
			{ID: "gpt-5", BillingMode: "token", ContextWindow: 128000, BestGroup: GroupRef{ID: 21}},
		},
	}
	fixture := newAutoBindFixture(t, catalog, []APIKey{{ID: 1, Key: testGateway, Status: "active"}})
	profile := t.TempDir()
	service := newTestSignedInService(t, profile, fixture.server.URL, Credentials{})

	if _, _, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil); err != nil {
		t.Fatal(err)
	}
	// User manually clears Claude.
	if err := service.SetLaneBinding(t.Context(), LaneClaude, nil); err != nil {
		t.Fatal(err)
	}
	if !service.LaneCleared(LaneClaude) {
		t.Fatal("manual clear was not recorded")
	}
	if got := service.LaneBindings(); !maps.Equal(got, map[string]int64{LaneCodex: 21}) {
		t.Fatalf("LaneBindings after manual clear = %v", got)
	}

	// Same-session sync must NOT refill manually cleared Claude.
	_, resSameSession, err := service.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || len(resSameSession.Bound) != 0 || service.LaneBindings()[LaneClaude] != 0 {
		t.Fatalf("same-session sync refilled cleared lane: res=%+v bindings=%v err=%v", resSameSession, service.LaneBindings(), err)
	}

	// Restoring the same login from disk must continue to respect manual clear.
	reopened, err := NewService(profile)
	if err != nil || reopened.Restore() != nil {
		t.Fatalf("reopen: %v", err)
	}
	if !reopened.LaneCleared(LaneClaude) {
		t.Fatal("manual clear was not persisted across Restore")
	}
	_, resRestored, err := reopened.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || len(resRestored.Bound) != 0 || reopened.LaneBindings()[LaneClaude] != 0 {
		t.Fatalf("restored sync refilled cleared lane: res=%+v bindings=%v err=%v", resRestored, reopened.LaneBindings(), err)
	}

	// Rollback via RestoreLaneBinding(..., nil, false) must NOT mark a lane cleared.
	if err := reopened.RestoreLaneBinding(t.Context(), LaneGemini, nil, false); err != nil {
		t.Fatal(err)
	}
	if reopened.LaneCleared(LaneGemini) {
		t.Fatal("RestoreLaneBinding with cleared=false marked lane as cleared")
	}

	// Next login clears the previous login's manual-clear markers and auto-binds Claude again.
	if err := reopened.Establish(t.Context(), Credentials{
		AccessToken:  testAccess,
		RefreshToken: testRefresh,
		ExpiresAt:    time.Now().Add(time.Hour).Unix(),
		Endpoint:     fixture.server.URL,
		APIKey:       testGateway,
	}); err != nil {
		t.Fatal(err)
	}
	if reopened.LaneCleared(LaneClaude) {
		t.Fatal("new login did not reset manual-clear state")
	}
	_, resNextLogin, err := reopened.AutoBindUnboundLanes(t.Context(), groups, &catalog, nil)
	if err != nil || !maps.Equal(resNextLogin.Bound, map[string]int64{LaneClaude: 11, LaneCodex: 21}) {
		t.Fatalf("next login should auto-bind cleared lane again: res=%+v err=%v", resNextLogin, err)
	}
}
