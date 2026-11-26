package cheapauth

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// Client is a blocking CheapRouter API client. Call it from background work,
// never from a UI frame.
type Client struct {
	endpoint string
	http     *http.Client
	now      func() time.Time
}

func NewClient(endpoint string, httpClient *http.Client) (*Client, error) {
	normalized, err := NormalizeEndpoint(endpoint)
	if err != nil {
		return nil, err
	}
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 20 * time.Second}
	}
	return &Client{endpoint: normalized, http: httpClient, now: time.Now}, nil
}

func NormalizeEndpoint(endpoint string) (string, error) {
	value := strings.TrimRight(strings.TrimSpace(endpoint), "/")
	parsed, err := url.Parse(value)
	if err != nil || value == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") ||
		parsed.Hostname() == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", &AuthError{Kind: ErrorResponse, Op: "endpoint"}
	}
	return value, nil
}

func (c *Client) Endpoint() string { return c.endpoint }

func (c *Client) apiURL(path string) string {
	return c.endpoint + "/api/v1/" + strings.TrimLeft(path, "/")
}

type envelope struct {
	Code    int64           `json:"code"`
	Message string          `json:"message"`
	Reason  string          `json:"reason"`
	Data    json.RawMessage `json:"data"`
}

func (c *Client) do(ctx context.Context, method, path, accessToken string, input, output any) error {
	data, status, err := c.doData(ctx, method, path, accessToken, input)
	if err != nil {
		return err
	}
	if output == nil {
		return nil
	}
	if len(data) == 0 || string(data) == "null" {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: status}
	}
	if err := json.Unmarshal(data, output); err != nil {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: status}
	}
	return nil
}

func (c *Client) doData(ctx context.Context, method, path, accessToken string, input any) (json.RawMessage, int, error) {
	var body io.Reader
	if input != nil {
		encoded, err := json.Marshal(input)
		if err != nil {
			return nil, 0, &AuthError{Kind: ErrorResponse, Op: path}
		}
		body = bytes.NewReader(encoded)
	}
	request, err := http.NewRequestWithContext(ctx, method, c.apiURL(path), body)
	if err != nil {
		return nil, 0, &AuthError{Kind: ErrorResponse, Op: path}
	}
	request.Header.Set("Accept", "application/json")
	if input != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	if accessToken != "" {
		request.Header.Set("Authorization", "Bearer "+accessToken)
	}
	response, err := c.http.Do(request)
	if err != nil {
		if ctx.Err() != nil {
			return nil, 0, ctx.Err()
		}
		return nil, 0, &AuthError{Kind: ErrorNetwork, Op: path}
	}
	defer response.Body.Close()
	authStatus := response.StatusCode == http.StatusUnauthorized ||
		path == "/auth/refresh" && response.StatusCode == http.StatusForbidden
	if authStatus && path != "/auth/desktop-session/exchange" {
		return nil, response.StatusCode, apiError(path, response.StatusCode, envelope{})
	}
	if response.ContentLength > maxResponseSize {
		return nil, response.StatusCode, &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, maxResponseSize+1))
	if ctx.Err() != nil {
		return nil, response.StatusCode, ctx.Err()
	}
	if err != nil || len(raw) > maxResponseSize {
		return nil, response.StatusCode, &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	var wrapped envelope
	if len(raw) != 0 {
		if err := json.Unmarshal(raw, &wrapped); err != nil {
			return nil, response.StatusCode, &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
		}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 || wrapped.Code != 0 {
		return nil, response.StatusCode, apiError(path, response.StatusCode, wrapped)
	}
	return wrapped.Data, response.StatusCode, nil
}

func apiError(op string, status int, wrapped envelope) error {
	reason := strings.ToUpper(strings.TrimSpace(wrapped.Reason))
	message := strings.ToLower(wrapped.Message)
	if op == "/auth/refresh" {
		switch {
		case status == http.StatusUnauthorized || status == http.StatusForbidden ||
			wrapped.Code == http.StatusUnauthorized || wrapped.Code == http.StatusForbidden:
			return &AuthError{Kind: ErrorRefresh, Op: op, Status: status}
		case reason == "REFRESH_TOKEN_INVALID", reason == "REFRESH_TOKEN_EXPIRED", reason == "REFRESH_TOKEN_REUSED",
			reason == "TOKEN_REVOKED", reason == "SESSION_BINDING_MISMATCH", reason == "USER_NOT_ACTIVE":
			return &AuthError{Kind: ErrorRefresh, Op: op, Status: status}
		default:
			return &AuthError{Kind: ErrorResponse, Op: op, Status: status}
		}
	}
	if op != "/auth/desktop-session/exchange" {
		if status == http.StatusUnauthorized || wrapped.Code == http.StatusUnauthorized {
			return &AuthError{Kind: ErrorUnauthorized, Op: op, Status: status}
		}
		return &AuthError{Kind: ErrorResponse, Op: op, Status: status}
	}
	switch {
	case strings.Contains(reason, "VERIFIER") || strings.Contains(reason, "PKCE") || strings.Contains(message, "verifier"):
		return &AuthError{Kind: ErrorVerifierMismatch, Op: op, Status: status}
	case strings.Contains(reason, "EXPIRED") || strings.Contains(message, "expired") || strings.Contains(message, "过期"):
		return &AuthError{Kind: ErrorCodeExpired, Op: op, Status: status}
	case reason == "DESKTOP_LOGIN_CODE_INVALID" || strings.Contains(reason, "LOGIN_CODE"):
		return &AuthError{Kind: ErrorCodeInvalid, Op: op, Status: status}
	case status == http.StatusUnauthorized:
		return &AuthError{Kind: ErrorUnauthorized, Op: op, Status: status}
	default:
		// Do not copy wrapped.Message: a server may echo request credentials.
		return &AuthError{Kind: ErrorResponse, Op: op, Status: status}
	}
}

// Exchange redeems a one-time login code on the login origin.
func (c *Client) Exchange(ctx context.Context, code, verifier string) (Credentials, error) {
	var session DesktopSession
	err := c.do(ctx, http.MethodPost, "/auth/desktop-session/exchange", "", map[string]string{
		"code": code, "code_verifier": verifier,
	}, &session)
	if err != nil {
		return Credentials{}, err
	}
	if strings.TrimSpace(session.AccessToken) == "" || strings.TrimSpace(session.RefreshToken) == "" || session.ExpiresIn <= 0 {
		return Credentials{}, &AuthError{Kind: ErrorResponse, Op: "/auth/desktop-session/exchange"}
	}
	return Credentials{
		AccessToken: session.AccessToken, RefreshToken: session.RefreshToken,
		ExpiresAt: c.now().Unix() + session.ExpiresIn, Endpoint: c.endpoint, APIKey: strings.TrimSpace(session.APIKey),
	}, nil
}

func (c *Client) Me(ctx context.Context, accessToken string) (User, error) {
	var user User
	err := c.do(ctx, http.MethodGet, "/auth/me", accessToken, nil, &user)
	return user, err
}

func (c *Client) Keys(ctx context.Context, accessToken string) (KeyPage, error) {
	var page KeyPage
	err := c.do(ctx, http.MethodGet, "/keys?page=1&page_size=50", accessToken, nil, &page)
	return page, err
}

// CreateKey mints an account-level gateway key. Only a key, never the account
// tokens, is handed to the model provider.
func (c *Client) CreateKey(ctx context.Context, accessToken, name string) (APIKey, error) {
	return c.CreateKeyForGroup(ctx, accessToken, name, nil)
}

// CreateKeyForGroup mints a gateway key optionally bound to groupID.
func (c *Client) CreateKeyForGroup(ctx context.Context, accessToken, name string, groupID *int64) (APIKey, error) {
	var key APIKey
	payload := map[string]any{"name": name, "client_source": CheapRouterClientSource}
	if groupID != nil {
		payload["group_id"] = *groupID
	}
	err := c.do(ctx, http.MethodPost, "/keys", accessToken, payload, &key)
	return key, err
}

// AvailableGroups lists the routing groups this account may switch to.
func (c *Client) AvailableGroups(ctx context.Context, accessToken string) ([]Group, error) {
	var groups []Group
	err := c.do(ctx, http.MethodGet, "/groups/available", accessToken, nil, &groups)
	return groups, err
}

// GroupStatuses fetches upstream availability and runtime status for monitored
// groups via GET /api/v1/group-status, accepting both flat and nested
// {group, summary} payloads as well as data: null (empty monitored list).
func (c *Client) GroupStatuses(ctx context.Context, accessToken string) ([]GroupStatus, error) {
	data, status, err := c.doData(ctx, http.MethodGet, "/group-status", accessToken, nil)
	if err != nil {
		return nil, err
	}
	trimmed := strings.TrimSpace(string(data))
	if trimmed == "" || trimmed == "null" {
		return nil, nil
	}
	var rawItems []json.RawMessage
	if err := json.Unmarshal(data, &rawItems); err != nil {
		var wrapped struct {
			Items []json.RawMessage `json:"items"`
		}
		if err2 := json.Unmarshal(data, &wrapped); err2 != nil {
			return nil, &AuthError{Kind: ErrorResponse, Op: "/group-status", Status: status}
		}
		rawItems = wrapped.Items
	}
	items := make([]GroupStatus, 0, len(rawItems))
	for _, raw := range rawItems {
		if item, ok := normalizeGroupStatus(raw); ok {
			items = append(items, item)
		}
	}
	return items, nil
}

func normalizeGroupStatus(raw json.RawMessage) (GroupStatus, bool) {
	var top map[string]json.RawMessage
	if err := json.Unmarshal(raw, &top); err != nil || len(top) == 0 {
		return GroupStatus{}, false
	}
	summary := decodeJSONObject(top["summary"])
	group := decodeJSONObject(top["group"])

	firstFloat := func(sources ...json.RawMessage) *float64 {
		for _, src := range sources {
			if len(src) == 0 || string(src) == "null" {
				continue
			}
			var v float64
			if err := json.Unmarshal(src, &v); err == nil && !math.IsNaN(v) && !math.IsInf(v, 0) {
				return &v
			}
		}
		return nil
	}
	firstInt64 := func(sources ...json.RawMessage) int64 {
		for _, src := range sources {
			if len(src) == 0 || string(src) == "null" {
				continue
			}
			var v int64
			if err := json.Unmarshal(src, &v); err == nil {
				return v
			}
			var f float64
			if err := json.Unmarshal(src, &f); err == nil && !math.IsNaN(f) && !math.IsInf(f, 0) {
				return int64(f)
			}
		}
		return 0
	}
	firstString := func(sources ...json.RawMessage) string {
		for _, src := range sources {
			if len(src) == 0 || string(src) == "null" {
				continue
			}
			var s string
			if err := json.Unmarshal(src, &s); err == nil && strings.TrimSpace(s) != "" {
				return s
			}
		}
		return ""
	}

	item := GroupStatus{
		GroupID: firstInt64(
			top["group_id"],
			summary["group_id"],
			group["id"],
			group["ID"],
		),
		GroupName: firstString(
			top["group_name"],
			group["name"],
			group["Name"],
			summary["group_name"],
		),
		LatestStatus: firstString(top["latest_status"], summary["latest_status"]),
		StableStatus: firstString(top["stable_status"], summary["stable_status"]),
		LatencyMS:    firstFloat(top["latency_ms"], summary["latency_ms"]),
		Availability24h: firstFloat(
			top["availability_24h"],
			top["availability24"],
			summary["availability_24h"],
			summary["availability24"],
		),
		Availability7d: firstFloat(
			top["availability_7d"],
			top["availability7d"],
			summary["availability_7d"],
			summary["availability7d"],
		),
	}
	return item, true
}

func decodeJSONObject(raw json.RawMessage) map[string]json.RawMessage {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(raw, &obj); err != nil {
		return nil
	}
	return obj
}

func (c *Client) ModelCatalog(ctx context.Context, accessToken string) (ModelCatalog, error) {
	var catalog ModelCatalog
	err := c.do(ctx, http.MethodGet, "/models/catalog", accessToken, nil, &catalog)
	return catalog, err
}

func (c *Client) UsageStats(ctx context.Context, accessToken, period string) (UsageStats, error) {
	var stats UsageStats
	switch period {
	case "today", "week", "month":
	default:
		period = "today"
	}
	err := c.do(ctx, http.MethodGet, "/usage/stats?period="+url.QueryEscape(period), accessToken, nil, &stats)
	return stats, err
}

// PublicCustomEndpoints reads the service's unauthenticated public custom
// endpoints list from GET /api/v1/settings/public, tolerating null or absent
// custom_endpoints fields.
func (c *Client) PublicCustomEndpoints(ctx context.Context) ([]PublicEndpoint, error) {
	reqCtx := ctx
	if _, hasDeadline := ctx.Deadline(); !hasDeadline {
		var cancel context.CancelFunc
		reqCtx, cancel = context.WithTimeout(ctx, 10*time.Second)
		defer cancel()
	}
	data, status, err := c.doData(reqCtx, http.MethodGet, "/settings/public", "", nil)
	if err != nil {
		return nil, err
	}
	trimmed := strings.TrimSpace(string(data))
	if trimmed == "" || trimmed == "null" {
		return nil, nil
	}
	var settings struct {
		CustomEndpoints []PublicEndpoint `json:"custom_endpoints"`
	}
	if err := json.Unmarshal(data, &settings); err != nil {
		return nil, &AuthError{Kind: ErrorResponse, Op: "/settings/public", Status: status}
	}
	return settings.CustomEndpoints, nil
}

// Refresh renews the session on the login origin. Callers must invoke this on
// the login client, never on a switched gateway node.
func (c *Client) Refresh(ctx context.Context, current Credentials) (Credentials, error) {
	var pair TokenPair
	err := c.do(ctx, http.MethodPost, "/auth/refresh", "", map[string]string{"refresh_token": current.RefreshToken}, &pair)
	if err != nil {
		return Credentials{}, err
	}
	if strings.TrimSpace(pair.AccessToken) == "" || pair.ExpiresIn <= 0 {
		return Credentials{}, &AuthError{Kind: ErrorResponse, Op: "/auth/refresh"}
	}
	current.AccessToken = pair.AccessToken
	if token := strings.TrimSpace(pair.RefreshToken); token != "" {
		current.RefreshToken = token
	}
	current.ExpiresAt = c.now().Unix() + pair.ExpiresIn
	return current, nil
}

// Logout revokes the refresh token on the login origin. Callers must invoke
// this on the login client, never on a switched gateway node.
func (c *Client) Logout(ctx context.Context, refreshToken string) error {
	return c.do(ctx, http.MethodPost, "/auth/logout", "", map[string]string{"refresh_token": refreshToken}, nil)
}

// LoginURL builds the exact browser bridge contract. endpoint is echoed back
// so a callback for another service can be rejected.
func LoginURL(endpoint string, port int, challenge string) string {
	redirect := fmt.Sprintf("http://127.0.0.1:%d/callback", port)
	return buildLoginURL(endpoint, redirect, challenge)
}

func ManualLoginURL(endpoint, challenge string) string {
	return buildLoginURL(endpoint, "", challenge)
}

func buildLoginURL(endpoint, redirect, challenge string) string {
	// Keep the contractual parameter order rather than url.Values.Encode order.
	result := endpoint + loginBridgePath + "?endpoint=" + url.QueryEscape(endpoint)
	if redirect != "" {
		result += "&redirect_to=" + url.QueryEscape(redirect)
	}
	return result + "&code_challenge=" + url.QueryEscape(challenge) + "&code_challenge_method=S256"
}
