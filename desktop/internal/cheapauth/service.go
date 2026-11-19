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

func (s *Service) StartLogin(endpoint string) (*LoginFlow, error) {
	client, err := NewClient(endpoint, nil)
	if err != nil {
		return nil, err
	}
	client.now = s.now
	flow, err := StartLogin(client, s.store)
	if err == nil {
		flow.onLogin = s.adopt
	}
	return flow, err
}

// StartManualLogin skips loopback for environments where the user already
// knows browser-to-loopback delivery is unavailable.
func (s *Service) StartManualLogin(endpoint string) (*LoginFlow, error) {
	client, err := NewClient(endpoint, nil)
	if err != nil {
		return nil, err
	}
	client.now = s.now
	flow, err := StartManualLogin(client, s.store)
	if err == nil {
		flow.onLogin = s.adopt
	}
	return flow, err
}

func (s *Service) adopt(credentials Credentials) {
	s.mu.Lock()
	defer s.mu.Unlock()
	client, _ := NewClient(credentials.Endpoint, nil)
	client.now = s.now
	copy := credentials
	s.credentials, s.client = &copy, client
}

// AccessToken returns a renewable bearer token for API calls. It must remain
// inside background API/provider glue and never be presented in the UI.
func (s *Service) AccessToken(ctx context.Context) (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil || s.client == nil {
		return "", ErrUnauthorized
	}
	if s.credentials.NeedsRefresh(s.now()) {
		credentials, err := s.client.Refresh(ctx, *s.credentials)
		if err != nil {
			return "", err
		}
		if err := s.store.Save(credentials); err != nil {
			return "", err
		}
		*s.credentials = credentials
	}
	return s.credentials.AccessToken, nil
}

// Refresh forces renewal and persists the rotated pair before returning.
func (s *Service) Refresh(ctx context.Context) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.credentials == nil || s.client == nil {
		return ErrUnauthorized
	}
	credentials, err := s.client.Refresh(ctx, *s.credentials)
	if err != nil {
		return err
	}
	if err := s.store.Save(credentials); err != nil {
		return err
	}
	*s.credentials = credentials
	return nil
}

func (s *Service) Account(ctx context.Context) (User, error) {
	token, err := s.AccessToken(ctx)
	if err != nil {
		return User{}, err
	}
	s.mu.Lock()
	client := s.client
	s.mu.Unlock()
	return client.Me(ctx, token)
}

// CheapRouterKeyName is the gateway key this client mints for an account
// that owns none, matching the name the Rust client uses.
const CheapRouterKeyName = "CheapRouter desktop"

// Gateway fetches catalog and keys after login. It prefers the bridge-minted
// key when it is still listed, otherwise the first active listed key.
func (s *Service) Gateway(ctx context.Context) (Gateway, error) {
	token, err := s.AccessToken(ctx)
	if err != nil {
		return Gateway{}, err
	}
	s.mu.Lock()
	client := s.client
	bridgeKey := ""
	if s.credentials != nil {
		bridgeKey = s.credentials.APIKey
	}
	s.mu.Unlock()
	catalog, err := client.ModelCatalog(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	keys, err := client.Keys(ctx, token)
	if err != nil {
		return Gateway{}, err
	}
	key := ""
	available := map[string]bool{}
	for _, candidate := range keys {
		candidate.Key = strings.TrimSpace(candidate.Key)
		if candidate.Key == "" || candidate.Status != "" && !strings.EqualFold(candidate.Status, "active") && !strings.EqualFold(candidate.Status, "enabled") {
			continue
		}
		available[candidate.Key] = true
		if key == "" {
			key = candidate.Key
		}
	}
	if available[bridgeKey] {
		key = bridgeKey
	}
	if key == "" && bridgeKey != "" && len(keys) == 0 {
		key = bridgeKey
	}
	if key == "" {
		// A new account owns no key yet: mint one, as the Rust client does,
		// instead of failing a sign-in the user cannot fix from here.
		created, err := client.CreateKey(ctx, token, CheapRouterKeyName)
		if err != nil {
			return Gateway{}, err
		}
		key = strings.TrimSpace(created.Key)
		if key == "" {
			return Gateway{}, &AuthError{Kind: ErrorResponse, Op: "/keys"}
		}
	}
	baseURL := strings.TrimRight(strings.TrimSpace(catalog.BaseURL), "/")
	if baseURL == "" {
		baseURL = client.Endpoint() + "/v1"
	}
	return Gateway{BaseURL: baseURL, APIKey: key, Models: catalog.Items}, nil
}

// Logout always clears local credentials, even when remote revocation cannot
// be reached. The returned error is sanitized by Client/Store.
func (s *Service) Logout(ctx context.Context) error {
	s.mu.Lock()
	credentials, client := s.credentials, s.client
	s.credentials, s.client = nil, nil
	s.mu.Unlock()
	var remote error
	if credentials != nil && client != nil {
		remote = client.Logout(ctx, credentials.RefreshToken)
	}
	return errors.Join(remote, s.store.Clear())
}
