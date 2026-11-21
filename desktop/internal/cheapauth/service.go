package cheapauth

import (
	"context"
	"errors"
	"log/slog"
	"maps"
	"slices"
	"strings"
	"sync"
	"time"
)

// Service serializes credential refresh and persistence for background UI
// workers. Its public snapshots never contain OAuth tokens.
type Service struct {
	mu          sync.Mutex
	store       *Store
	client      *Client
	credentials *Credentials
	now         func() time.Time
}

// credentialLock serializes every renewal and every write of the credential
// file across the Services of this process. Each window owns a Service over
// the same file and the service rotates refresh tokens: two renewals racing
// each other would leave the loser with a dead pair, which it would then
// clear from disk.
var credentialLock sync.Mutex

func NewService(profileDir string) (*Service, error) {
	store, err := NewStore(profileDir)
	if err != nil {
		return nil, err
	}
	return &Service{store: store, now: time.Now}, nil
}

// Restore loads the saved account without making network requests.
func (s *Service) Restore() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	credentials, err := s.store.Load()
	if err != nil {
		return err
	}
	if credentials == nil {
		s.credentials, s.client = nil, nil
		return nil
	}
	client, err := NewClient(credentials.Endpoint, nil)
	if err != nil {
		return err
	}
	client.now = s.now
	s.credentials, s.client = credentials, client
	return nil
}

func (s *Service) SignedIn() bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.credentials != nil && s.credentials.valid()
}

func (s *Service) Endpoint() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil {
		return ""
	}
	return s.credentials.Endpoint
}

func (s *Service) StartLogin(ctx context.Context, endpoint string) (*LoginFlow, error) {
	client, err := NewClient(endpoint, nil)
	if err != nil {
		return nil, err
	}
	client.now = s.now
	return StartLogin(ctx, client)
}

// StartManualLogin skips loopback for environments where the user already
// knows browser-to-loopback delivery is unavailable.
func (s *Service) StartManualLogin(endpoint string) (*LoginFlow, error) {
	client, err := NewClient(endpoint, nil)
	if err != nil {
		return nil, err
	}
	client.now = s.now
	return StartManualLogin(client)
}

// Establish persists and adopts a candidate. The caller must first admit it
// through the current attempt's serialized commit path.
func (s *Service) Establish(ctx context.Context, credentials Credentials) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	client, err := NewClient(credentials.Endpoint, nil)
	if err != nil {
		return err
	}
	if err := s.store.Save(credentials); err != nil {
		return err
	}
	client.now = s.now
	s.credentials, s.client = &credentials, client
	return nil
}

// AccessToken returns a renewable bearer token for API calls. It must remain
// inside background API/provider glue and never be presented in the UI.
func (s *Service) AccessToken(ctx context.Context) (string, error) {
	credentials, _, err := s.session(ctx)
	return credentials.AccessToken, err
}

func (s *Service) session(ctx context.Context) (Credentials, *Client, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ctx.Err() != nil {
		return Credentials{}, nil, ctx.Err()
	}
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := s.adoptLocked(); err != nil {
		return Credentials{}, nil, err
	}
	if s.credentials.NeedsRefresh(s.now()) {
		if err := s.refreshLocked(ctx); err != nil {
			return Credentials{}, nil, err
		}
	}
	return *s.credentials, s.client, nil
}

// adoptLocked takes over what another window stored since this Service last
// looked: a renewed pair, another sign-in, or a sign-out. A Service only
// changes its session after saving it, so the file is never the older of the
// two. The caller holds credentialLock.
func (s *Service) adoptLocked() error {
	if s.credentials == nil || s.client == nil {
		return ErrUnauthorized
	}
	stored, err := s.store.Load()
	if err != nil {
		// An unreadable file is replaced by the next renewal's save.
		return nil
	}
	if stored == nil {
		s.credentials, s.client = nil, nil
		return ErrUnauthorized
	}
	if credentialsEqual(*stored, *s.credentials) {
		return nil
	}
	client := s.client
	if stored.Endpoint != s.credentials.Endpoint {
		if client, err = NewClient(stored.Endpoint, nil); err != nil {
			return err
		}
		client.now = s.now
	}
	s.credentials, s.client = stored, client
	return nil
}

// Refresh forces renewal and persists the rotated pair before returning.
func (s *Service) Refresh(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := s.adoptLocked(); err != nil {
		return err
	}
	return s.refreshLocked(ctx)
}

func (s *Service) refreshLocked(ctx context.Context) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}
	if s.credentials == nil || s.client == nil {
		return ErrUnauthorized
	}
	credentials, err := s.client.Refresh(ctx, *s.credentials)
	if ctx.Err() != nil {
		return ctx.Err()
	}
	if err != nil {
		if errors.Is(err, ErrRefresh) || errors.Is(err, ErrUnauthorized) {
			return errors.Join(err, s.clearLocked())
		}
		return err
	}
	if err := s.store.Save(credentials); err != nil {
		return err
	}
	*s.credentials = credentials
	return nil
}

func (s *Service) Account(ctx context.Context) (User, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return User{}, err
	}
	return client.Me(ctx, credentials.AccessToken)
}

// ReadSetup shares one credential snapshot across both reads, so an optional
// account failure cannot trigger a second refresh or prevent gateway setup.
func (s *Service) ReadSetup(ctx context.Context) (User, error, Gateway, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return User{}, nil, Gateway{}, err
	}
	user, accountErr := client.Me(ctx, credentials.AccessToken)
	if errors.Is(accountErr, ErrUnauthorized) || ctx.Err() != nil {
		return user, accountErr, Gateway{}, ctx.Err()
	}
	gateway, gatewayErr := readGateway(ctx, client, credentials)
	return user, accountErr, gateway, gatewayErr
}

// CheapRouterKeyName is the gateway key this client mints for an account
// that owns none, matching the name the Rust client uses.
const CheapRouterKeyName = "CheapRouter desktop"

// Gateway fetches catalog and keys after login, honoring all bound lanes (or
// legacy single GroupID / account default when no lane is bound).
func (s *Service) Gateway(ctx context.Context) (Gateway, error) {
	return s.GatewayWithGroups(ctx, nil)
}

// GatewayWithGroups fetches a fresh model catalog and ensures active gateway
// keys for all bound lanes, using groups to classify legacy bindings and silent
// groups.
func (s *Service) GatewayWithGroups(ctx context.Context, groups []Group) (Gateway, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return Gateway{}, err
	}
	return readGatewayWithGroups(ctx, client, credentials, groups)
}

// GatewayForGroup fetches a fresh model catalog and ensures an active gateway
// key for groupID (nil means account default / unbound). Equivalent to legacy
// ensure_key_for_group semantics: reuses an active key bound to groupID before
// minting one.
func (s *Service) GatewayForGroup(ctx context.Context, groupID *int64) (Gateway, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return Gateway{}, err
	}
	return readGatewayForGroup(ctx, client, credentials, groupID)
}

// GatewayForLaneChange previews and prepares a single lane's group switch or
// unbind without modifying persisted credentials. When groupID is non-nil it
// reuses an active key for that group before minting one (ensure_key_for_group).
func (s *Service) GatewayForLaneChange(ctx context.Context, lane string, groupID *int64, groups []Group) (Gateway, *LaneBinding, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return Gateway{}, nil, err
	}
	laneKey := canonicalLaneKey(lane)
	if laneKey == "" {
		return Gateway{}, nil, &AuthError{Kind: ErrorResponse, Op: "lane"}
	}
	token := credentials.AccessToken
	catalog, err := client.ModelCatalog(ctx, token)
	if err != nil {
		return Gateway{}, nil, err
	}
	keys, err := client.Keys(ctx, token)
	if err != nil {
		return Gateway{}, nil, err
	}
	nextBindings := effectiveLaneBindings(credentials, groups, catalog.Items)
	var target *LaneBinding
	if groupID != nil && *groupID > 0 {
		ensured, err := ensureGroupKey(ctx, client, token, &keys, *groupID)
		if err != nil {
			return Gateway{}, nil, err
		}
		target = &LaneBinding{GroupID: *groupID, APIKey: ensured}
		nextBindings[laneKey] = *target
	} else {
		delete(nextBindings, laneKey)
	}
	gateway, err := buildGatewayForBindings(ctx, client, token, credentials.APIKey, catalog, &keys, nextBindings, groups)
	if err != nil {
		return Gateway{}, nil, err
	}
	return gateway, target, nil
}

// AutoBindFailure records a non-fatal per-lane auto-bind failure.
type AutoBindFailure struct {
	Lane    string
	GroupID int64
	Err     error
}

// AutoBindResult summarizes one post-sync auto-bind pass over canonical lanes.
type AutoBindResult struct {
	Bound    map[string]int64
	Skipped  []string
	Failures []AutoBindFailure
}

// AutoBindUnboundLanes binds each unbound, non-cleared canonical lane to its
// first routable group using the same key-ensure and lane-binding path as
// manual switches. Already-bound lanes are untouched, per-lane failures are
// recorded without aborting other lanes, and the returned Gateway reflects all
// newly bound lanes when len(result.Bound) > 0.
func (s *Service) AutoBindUnboundLanes(
	ctx context.Context,
	groups []Group,
	catalog *ModelCatalog,
	skipLanes map[string]bool,
) (Gateway, AutoBindResult, error) {
	result := AutoBindResult{}
	if len(groups) == 0 {
		result.Skipped = append(result.Skipped, CanonicalLanes...)
		return Gateway{}, result, nil
	}
	credentials, client, err := s.session(ctx)
	if err != nil {
		return Gateway{}, result, err
	}
	token := credentials.AccessToken
	var resolvedCatalog ModelCatalog
	if catalog != nil {
		resolvedCatalog = *catalog
	} else {
		fetched, err := client.ModelCatalog(ctx, token)
		if err != nil {
			return Gateway{}, result, err
		}
		resolvedCatalog = fetched
	}

	if credentials.GroupID != nil && *credentials.GroupID > 0 && len(credentials.LaneBindings) == 0 {
		_ = s.MigrateLegacyGroup(ctx, groups, resolvedCatalog.Items)
		if refreshed, _, sessionErr := s.session(ctx); sessionErr == nil {
			credentials = refreshed
		}
	}

	currentBindings := effectiveLaneBindings(credentials, groups, resolvedCatalog.Items)
	skippedSet := make(map[string]bool, len(credentials.ClearedLanes)+len(skipLanes))
	for _, lane := range credentials.ClearedLanes {
		if key := canonicalLaneKey(lane); key != "" {
			skippedSet[key] = true
		}
	}
	for lane, skip := range skipLanes {
		if skip {
			if key := canonicalLaneKey(lane); key != "" {
				skippedSet[key] = true
			}
		}
	}

	type laneCandidate struct {
		lane  string
		group Group
	}
	var candidates []laneCandidate
	for _, lane := range CanonicalLanes {
		if _, bound := currentBindings[lane]; bound {
			result.Skipped = append(result.Skipped, lane)
			continue
		}
		if skippedSet[lane] {
			result.Skipped = append(result.Skipped, lane)
			continue
		}
		group, ok := FirstRoutableGroupForLane(lane, groups, resolvedCatalog.Items)
		if !ok {
			slog.Debug("cheaprouter auto-bind skipped lane without routable group", "lane", lane)
			result.Skipped = append(result.Skipped, lane)
			continue
		}
		candidates = append(candidates, laneCandidate{lane: lane, group: group})
	}
	if len(candidates) == 0 {
		return Gateway{}, result, nil
	}

	keys, err := client.Keys(ctx, token)
	if err != nil {
		return Gateway{}, result, err
	}
	for _, cand := range candidates {
		if ctx.Err() != nil {
			break
		}
		ensured, err := ensureGroupKey(ctx, client, token, &keys, cand.group.ID)
		if err != nil {
			slog.Warn("cheaprouter auto-bind key ensure failed", "lane", cand.lane, "group_id", cand.group.ID, "error", err.Error())
			result.Failures = append(result.Failures, AutoBindFailure{
				Lane:    cand.lane,
				GroupID: cand.group.ID,
				Err:     err,
			})
			if errors.Is(err, ErrUnauthorized) || errors.Is(err, ErrRefresh) || ctx.Err() != nil {
				break
			}
			continue
		}
		binding := LaneBinding{GroupID: cand.group.ID, APIKey: ensured}
		if err := s.SetLaneBinding(ctx, cand.lane, &binding); err != nil {
			slog.Warn("cheaprouter auto-bind persist failed", "lane", cand.lane, "group_id", cand.group.ID, "error", err.Error())
			result.Failures = append(result.Failures, AutoBindFailure{
				Lane:    cand.lane,
				GroupID: cand.group.ID,
				Err:     err,
			})
			if errors.Is(err, ErrUnauthorized) || ctx.Err() != nil {
				break
			}
			continue
		}
		if result.Bound == nil {
			result.Bound = map[string]int64{}
		}
		result.Bound[cand.lane] = cand.group.ID
		currentBindings[cand.lane] = binding
	}
	if len(result.Bound) == 0 {
		return Gateway{}, result, nil
	}
	gateway, err := buildGatewayForBindings(ctx, client, token, credentials.APIKey, resolvedCatalog, &keys, currentBindings, groups)
	if err != nil {
		return Gateway{}, result, err
	}
	return gateway, result, nil
}

// AvailableGroups returns the routing groups available to the signed-in account.
func (s *Service) AvailableGroups(ctx context.Context) ([]Group, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return nil, err
	}
	return client.AvailableGroups(ctx, credentials.AccessToken)
}

// SelectedGroup returns a copy of the legacy single bound group ID, or nil.
func (s *Service) SelectedGroup() *int64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil {
		return nil
	}
	return cloneInt64Ptr(s.credentials.GroupID)
}

// LaneBindings returns a UI-safe snapshot of lane -> bound group ID (no keys).
func (s *Service) LaneBindings() map[string]int64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil || len(s.credentials.LaneBindings) == 0 {
		return nil
	}
	out := make(map[string]int64, len(s.credentials.LaneBindings))
	for lane, binding := range s.credentials.LaneBindings {
		if key := canonicalLaneKey(lane); key != "" && binding.GroupID > 0 {
			out[key] = binding.GroupID
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

// LaneBindingFor returns a copy of the currently persisted binding for lane.
func (s *Service) LaneBindingFor(lane string) *LaneBinding {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil {
		return nil
	}
	key := canonicalLaneKey(lane)
	if binding, ok := s.credentials.LaneBindings[key]; ok && binding.GroupID > 0 {
		copied := binding
		return &copied
	}
	return nil
}

// LaneCleared reports whether lane was manually cleared in the current login.
func (s *Service) LaneCleared(lane string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil || len(s.credentials.ClearedLanes) == 0 {
		return false
	}
	key := canonicalLaneKey(lane)
	if key == "" {
		return false
	}
	for _, cleared := range s.credentials.ClearedLanes {
		if canonicalLaneKey(cleared) == key {
			return true
		}
	}
	return false
}

// SetLaneBinding atomically updates or clears only the specified lane's binding.
// Clearing a lane records it in ClearedLanes so post-sync auto-bind will not
// refill it until the next login.
func (s *Service) SetLaneBinding(ctx context.Context, lane string, binding *LaneBinding) error {
	markCleared := binding == nil || binding.GroupID <= 0
	return s.setLaneBinding(ctx, lane, binding, markCleared)
}

// RestoreLaneBinding restores a lane's prior binding and manual-clear marker
// after a failed provider update.
func (s *Service) RestoreLaneBinding(ctx context.Context, lane string, binding *LaneBinding, cleared bool) error {
	return s.setLaneBinding(ctx, lane, binding, cleared)
}

func (s *Service) setLaneBinding(ctx context.Context, lane string, binding *LaneBinding, cleared bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ctx.Err() != nil {
		return ctx.Err()
	}
	key := canonicalLaneKey(lane)
	if key == "" {
		return &AuthError{Kind: ErrorResponse, Op: "lane"}
	}
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := s.adoptLocked(); err != nil {
		return err
	}
	updated := *s.credentials
	updated.LaneBindings = cloneLaneBindings(s.credentials.LaneBindings)
	if binding != nil && binding.GroupID > 0 {
		if updated.LaneBindings == nil {
			updated.LaneBindings = map[string]LaneBinding{}
		}
		updated.LaneBindings[key] = *binding
		cleared = false
	} else {
		delete(updated.LaneBindings, key)
	}
	if len(updated.LaneBindings) == 0 {
		updated.LaneBindings = nil
	}
	updated.ClearedLanes = updateClearedLanes(s.credentials.ClearedLanes, key, cleared)
	// Clear legacy single-group field once per-lane bindings are mutated.
	updated.GroupID = nil
	if err := s.store.Save(updated); err != nil {
		return err
	}
	*s.credentials = updated
	return nil
}

func updateClearedLanes(existing []string, lane string, cleared bool) []string {
	set := make(map[string]bool, len(existing)+1)
	for _, item := range existing {
		if key := canonicalLaneKey(item); key != "" {
			set[key] = true
		}
	}
	if cleared {
		set[lane] = true
	} else {
		delete(set, lane)
	}
	if len(set) == 0 {
		return nil
	}
	out := make([]string, 0, len(set))
	for key := range set {
		out = append(out, key)
	}
	slices.Sort(out)
	return out
}

// MigrateLegacyGroup moves a legacy single GroupID binding into its inferred
// lane once groups/catalog are available.
func (s *Service) MigrateLegacyGroup(ctx context.Context, groups []Group, catalog []Model) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ctx.Err() != nil {
		return ctx.Err()
	}
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := s.adoptLocked(); err != nil {
		return err
	}
	if s.credentials.GroupID == nil || *s.credentials.GroupID <= 0 || len(s.credentials.LaneBindings) > 0 {
		return nil
	}
	lane := inferLaneForGroupID(*s.credentials.GroupID, groups, catalog)
	if lane == "" {
		return nil
	}
	updated := *s.credentials
	updated.LaneBindings = map[string]LaneBinding{
		lane: {GroupID: *s.credentials.GroupID},
	}
	updated.GroupID = nil
	if err := s.store.Save(updated); err != nil {
		return err
	}
	*s.credentials = updated
	return nil
}

// SetGroup persists the bound group ID after a legacy group switch succeeds.
func (s *Service) SetGroup(ctx context.Context, groupID *int64) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ctx.Err() != nil {
		return ctx.Err()
	}
	credentialLock.Lock()
	defer credentialLock.Unlock()
	if err := s.adoptLocked(); err != nil {
		return err
	}
	updated := *s.credentials
	updated.GroupID = cloneInt64Ptr(groupID)
	if err := s.store.Save(updated); err != nil {
		return err
	}
	*s.credentials = updated
	return nil
}

func readGateway(ctx context.Context, client *Client, credentials Credentials) (Gateway, error) {
	return readGatewayWithGroups(ctx, client, credentials, nil)
}

func readGatewayWithGroups(ctx context.Context, client *Client, credentials Credentials, groups []Group) (Gateway, error) {
	if len(credentials.LaneBindings) == 0 && credentials.GroupID != nil {
		return readGatewayForGroup(ctx, client, credentials, credentials.GroupID)
	}
	token := credentials.AccessToken
	catalog, err := client.ModelCatalog(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	keys, err := client.Keys(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	bindings := effectiveLaneBindings(credentials, groups, catalog.Items)
	return buildGatewayForBindings(ctx, client, token, credentials.APIKey, catalog, &keys, bindings, groups)
}

func buildGatewayForBindings(
	ctx context.Context,
	client *Client,
	token, bridgeKey string,
	catalog ModelCatalog,
	keys *KeyPage,
	bindings map[string]LaneBinding,
	groups []Group,
) (Gateway, error) {
	baseURL := strings.TrimRight(strings.TrimSpace(catalog.BaseURL), "/")
	if baseURL == "" {
		baseURL = client.Endpoint() + "/v1"
	}
	if len(bindings) == 0 {
		key := accountKey(*keys, bridgeKey)
		if key == "" {
			created, err := client.CreateKeyForGroup(ctx, token, CheapRouterKeyName, nil)
			if err != nil {
				return Gateway{}, err
			}
			key = strings.TrimSpace(created.Key)
			if key == "" {
				return Gateway{}, &AuthError{Kind: ErrorResponse, Op: "/keys"}
			}
		}
		return Gateway{
			BaseURL: baseURL,
			APIKey:  key,
			Models:  catalog.Items,
			Catalog: catalog.Items,
		}, nil
	}

	laneKeys := make(map[string]string, len(bindings))
	laneGroupIDs := make(map[string]int64, len(bindings))
	for lane, binding := range bindings {
		if binding.GroupID <= 0 {
			continue
		}
		key := reusableLaneKey(*keys, binding)
		if key == "" {
			ensured, err := ensureGroupKey(ctx, client, token, keys, binding.GroupID)
			if err != nil {
				return Gateway{}, err
			}
			key = ensured
		}
		laneKeys[lane] = key
		laneGroupIDs[lane] = binding.GroupID
	}

	models, modelLanes := SelectCatalogForBindings(catalog.Items, laneGroupIDs, groups)
	fallbackKey := accountKey(*keys, bridgeKey)
	if fallbackKey == "" {
		for _, lane := range CanonicalLanes {
			if k := laneKeys[lane]; k != "" {
				fallbackKey = k
				break
			}
		}
		if fallbackKey == "" {
			for _, k := range laneKeys {
				if k != "" {
					fallbackKey = k
					break
				}
			}
		}
	}
	return Gateway{
		BaseURL:      baseURL,
		APIKey:       fallbackKey,
		LaneKeys:     laneKeys,
		ModelLanes:   modelLanes,
		Models:       models,
		Catalog:      catalog.Items,
		LaneBindings: laneGroupIDs,
	}, nil
}

func readGatewayForGroup(ctx context.Context, client *Client, credentials Credentials, groupID *int64) (Gateway, error) {
	token, bridgeKey := credentials.AccessToken, credentials.APIKey
	catalog, err := client.ModelCatalog(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	keys, err := client.Keys(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	var key string
	if groupID != nil {
		key = groupKey(keys, *groupID)
	} else {
		key = accountKey(keys, bridgeKey)
	}
	if key == "" {
		// Reuse failed: mint a key for this group (or account-level if nil),
		// matching the Rust client's ensure_key_for_group semantics.
		created, err := client.CreateKeyForGroup(ctx, token, CheapRouterKeyName, groupID)
		if err != nil {
			return Gateway{}, err
		}
		key = strings.TrimSpace(created.Key)
		if key == "" {
			return Gateway{}, &AuthError{Kind: ErrorResponse, Op: "/keys"}
		}
	}
	models := filterCatalogModelsForGroup(catalog.Items, groupID)
	baseURL := strings.TrimRight(strings.TrimSpace(catalog.BaseURL), "/")
	if baseURL == "" {
		baseURL = client.Endpoint() + "/v1"
	}
	return Gateway{BaseURL: baseURL, APIKey: key, Models: models, Catalog: catalog.Items, GroupID: cloneInt64Ptr(groupID)}, nil
}

func ensureGroupKey(ctx context.Context, client *Client, token string, keys *KeyPage, groupID int64) (string, error) {
	if key := groupKey(*keys, groupID); key != "" {
		return key, nil
	}
	gid := groupID
	created, err := client.CreateKeyForGroup(ctx, token, CheapRouterKeyName, &gid)
	if err != nil {
		return "", err
	}
	key := strings.TrimSpace(created.Key)
	if key == "" {
		return "", &AuthError{Kind: ErrorResponse, Op: "/keys"}
	}
	keys.Items = append(keys.Items, APIKey{
		ID:      created.ID,
		Key:     key,
		Name:    created.Name,
		GroupID: &gid,
		Status:  "active",
	})
	return key, nil
}

func reusableLaneKey(page KeyPage, binding LaneBinding) string {
	saved := strings.TrimSpace(binding.APIKey)
	if saved != "" {
		for _, candidate := range page.Items {
			key := strings.TrimSpace(candidate.Key)
			if key != saved {
				continue
			}
			if candidate.GroupID != nil && *candidate.GroupID == binding.GroupID && keyStatusActive(candidate.Status) {
				return key
			}
			return ""
		}
		if len(page.Items) == 0 || page.Total > int64(len(page.Items)) {
			return saved
		}
	}
	return groupKey(page, binding.GroupID)
}

func keyStatusActive(status string) bool {
	return status == "" || strings.EqualFold(status, "active") || strings.EqualFold(status, "enabled")
}

func effectiveLaneBindings(credentials Credentials, groups []Group, catalog []Model) map[string]LaneBinding {
	out := make(map[string]LaneBinding, len(credentials.LaneBindings))
	for lane, binding := range credentials.LaneBindings {
		if key := canonicalLaneKey(lane); key != "" && binding.GroupID > 0 {
			out[key] = binding
		}
	}
	if len(out) == 0 && credentials.GroupID != nil && *credentials.GroupID > 0 {
		if lane := inferLaneForGroupID(*credentials.GroupID, groups, catalog); lane != "" {
			out[lane] = LaneBinding{GroupID: *credentials.GroupID}
		}
	}
	return out
}

func inferLaneForGroupID(groupID int64, groups []Group, catalog []Model) string {
	for _, g := range groups {
		if g.ID == groupID {
			if lane := GroupLane(g, catalog); lane != "" {
				return lane
			}
		}
	}
	for _, item := range catalog {
		if item.ServesGroup(groupID) {
			if lane := ModelLane(item); lane != "" {
				return lane
			}
		}
	}
	return ""
}

func canonicalLaneKey(lane string) string {
	if normalized := NormalizeLane(lane); normalized != "" {
		return normalized
	}
	return strings.ToLower(strings.TrimSpace(lane))
}

func cloneLaneBindings(src map[string]LaneBinding) map[string]LaneBinding {
	if len(src) == 0 {
		return nil
	}
	dst := make(map[string]LaneBinding, len(src))
	maps.Copy(dst, src)
	return dst
}

func groupKey(page KeyPage, groupID int64) string {
	for _, candidate := range page.Items {
		key := strings.TrimSpace(candidate.Key)
		if key == "" || candidate.GroupID == nil || *candidate.GroupID != groupID {
			continue
		}
		if !keyStatusActive(candidate.Status) {
			continue
		}
		return key
	}
	return ""
}

func filterCatalogModelsForGroup(items []Model, groupID *int64) []Model {
	if groupID == nil || len(items) == 0 {
		return items
	}
	hasGroupMetadata := false
	matched := make([]Model, 0, len(items))
	for _, item := range items {
		if item.BestGroup.ID > 0 || len(item.OtherGroups) > 0 {
			hasGroupMetadata = true
		}
		if item.ServesGroup(*groupID) {
			matched = append(matched, item)
		}
	}
	// If the catalog carries group bindings and lists models for this group,
	// narrow to them; otherwise keep the full catalog (some groups map models
	// dynamically without explicit catalog entries).
	if hasGroupMetadata && len(matched) > 0 {
		return matched
	}
	return items
}

func cloneInt64Ptr(v *int64) *int64 {
	if v == nil {
		return nil
	}
	copied := *v
	return &copied
}

func credentialsEqual(a, b Credentials) bool {
	if a.AccessToken != b.AccessToken || a.RefreshToken != b.RefreshToken ||
		a.ExpiresAt != b.ExpiresAt || a.Endpoint != b.Endpoint || a.APIKey != b.APIKey {
		return false
	}
	if (a.GroupID == nil) != (b.GroupID == nil) || (a.GroupID != nil && *a.GroupID != *b.GroupID) {
		return false
	}
	return maps.Equal(a.LaneBindings, b.LaneBindings) && slices.Equal(a.ClearedLanes, b.ClearedLanes)
}

// accountKey picks the key the whole catalog is routed with: the bridge's
// while the listing shows it usable, else an account-level key, which no
// group restricts, before one bound to a group. A bridge key the listing
// cannot have shown, because it is empty or holds more keys than its first
// page, is merely unseen and kept. No usable key is the empty string.
func accountKey(page KeyPage, bridgeKey string) string {
	var unbound, bound string
	for _, candidate := range page.Items {
		key := strings.TrimSpace(candidate.Key)
		if key == "" {
			continue
		}
		if candidate.Status != "" && !strings.EqualFold(candidate.Status, "active") && !strings.EqualFold(candidate.Status, "enabled") {
			if key == bridgeKey {
				bridgeKey = ""
			}
			continue
		}
		switch {
		case key == bridgeKey:
			return key
		case candidate.GroupID == nil && unbound == "":
			unbound = key
		case candidate.GroupID != nil && bound == "":
			bound = key
		}
	}
	if bridgeKey != "" && (len(page.Items) == 0 || page.Total > int64(len(page.Items))) {
		return bridgeKey
	}
	if unbound != "" {
		return unbound
	}
	return bound
}

// Logout always clears local credentials, even when remote revocation cannot
// be reached. The returned error is sanitized by Client/Store.
func (s *Service) Logout(ctx context.Context) error {
	s.mu.Lock()
	credentialLock.Lock()
	// Revoke the pair that is stored now, not one another window renewed away.
	_ = s.adoptLocked()
	credentials, client := s.credentials, s.client
	local := s.clearLocked()
	credentialLock.Unlock()
	s.mu.Unlock()
	var remote error
	if credentials != nil && client != nil {
		remote = client.Logout(ctx, credentials.RefreshToken)
	}
	return errors.Join(local, remote)
}

// Invalidate drops a rejected local session without trying to revoke it.
func (s *Service) Invalidate() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	credentialLock.Lock()
	defer credentialLock.Unlock()
	return s.clearLocked()
}

func (s *Service) clearLocked() error {
	s.credentials, s.client = nil, nil
	return s.store.Clear()
}
