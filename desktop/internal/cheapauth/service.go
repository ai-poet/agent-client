package cheapauth

import (
	"context"
	"errors"
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

// Gateway fetches catalog and keys after login. The key is the one accountKey
// picks among the listed ones (honoring any bound group), or a newly minted one.
func (s *Service) Gateway(ctx context.Context) (Gateway, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return Gateway{}, err
	}
	return readGatewayForGroup(ctx, client, credentials, credentials.GroupID)
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

// AvailableGroups returns the routing groups available to the signed-in account.
func (s *Service) AvailableGroups(ctx context.Context) ([]Group, error) {
	credentials, client, err := s.session(ctx)
	if err != nil {
		return nil, err
	}
	return client.AvailableGroups(ctx, credentials.AccessToken)
}

// SelectedGroup returns a copy of the currently bound group ID, or nil for default.
func (s *Service) SelectedGroup() *int64 {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil {
		return nil
	}
	return cloneInt64Ptr(s.credentials.GroupID)
}

// SetGroup persists the bound group ID after a group switch succeeds.
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
	return readGatewayForGroup(ctx, client, credentials, credentials.GroupID)
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
	return Gateway{BaseURL: baseURL, APIKey: key, Models: models, GroupID: cloneInt64Ptr(groupID)}, nil
}

func groupKey(page KeyPage, groupID int64) string {
	for _, candidate := range page.Items {
		key := strings.TrimSpace(candidate.Key)
		if key == "" || candidate.GroupID == nil || *candidate.GroupID != groupID {
			continue
		}
		if candidate.Status != "" && !strings.EqualFold(candidate.Status, "active") && !strings.EqualFold(candidate.Status, "enabled") {
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
	if a.GroupID == nil || b.GroupID == nil {
		return a.GroupID == b.GroupID
	}
	return *a.GroupID == *b.GroupID
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
