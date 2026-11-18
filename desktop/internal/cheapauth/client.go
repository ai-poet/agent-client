package cheapauth

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
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
	var body io.Reader
	if input != nil {
		encoded, err := json.Marshal(input)
		if err != nil {
			return &AuthError{Kind: ErrorResponse, Op: path}
		}
		body = bytes.NewReader(encoded)
	}
	request, err := http.NewRequestWithContext(ctx, method, c.apiURL(path), body)
	if err != nil {
		return &AuthError{Kind: ErrorResponse, Op: path}
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
		return &AuthError{Kind: ErrorNetwork, Op: path}
	}
	defer response.Body.Close()
	if response.ContentLength > maxResponseSize {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	raw, err := io.ReadAll(io.LimitReader(response.Body, maxResponseSize+1))
	if err != nil || len(raw) > maxResponseSize {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	var wrapped envelope
	if len(raw) != 0 {
		if err := json.Unmarshal(raw, &wrapped); err != nil {
			return &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
		}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 || wrapped.Code != 0 {
		return apiError(path, response.StatusCode, wrapped)
	}
	if output == nil {
		return nil
	}
	if len(wrapped.Data) == 0 || string(wrapped.Data) == "null" {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	if err := json.Unmarshal(wrapped.Data, output); err != nil {
		return &AuthError{Kind: ErrorResponse, Op: path, Status: response.StatusCode}
	}
	return nil
}

func apiError(op string, status int, wrapped envelope) error {
	reason := strings.ToUpper(strings.TrimSpace(wrapped.Reason))
	message := strings.ToLower(wrapped.Message)
	switch {
	case strings.Contains(reason, "VERIFIER") || strings.Contains(reason, "PKCE") || strings.Contains(message, "verifier"):
		return &AuthError{Kind: ErrorVerifierMismatch, Op: op, Status: status}
	case strings.Contains(reason, "EXPIRED") || strings.Contains(message, "expired") || strings.Contains(message, "过期"):
		return &AuthError{Kind: ErrorCodeExpired, Op: op, Status: status}
	case reason == "DESKTOP_LOGIN_CODE_INVALID" || strings.Contains(reason, "LOGIN_CODE"):
		return &AuthError{Kind: ErrorCodeInvalid, Op: op, Status: status}
	case op == "/auth/refresh":
		return &AuthError{Kind: ErrorRefresh, Op: op, Status: status}
	case status == http.StatusUnauthorized:
		return &AuthError{Kind: ErrorUnauthorized, Op: op, Status: status}
	default:
		// Do not copy wrapped.Message: a server may echo request credentials.
		return &AuthError{Kind: ErrorResponse, Op: op, Status: status}
	}
}

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

func (c *Client) Keys(ctx context.Context, accessToken string) ([]APIKey, error) {
	var page struct {
		Items []APIKey `json:"items"`
	}
	err := c.do(ctx, http.MethodGet, "/keys?page=1&page_size=50", accessToken, nil, &page)
	return page.Items, err
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

func (c *Client) Refresh(ctx context.Context, current Credentials) (Credentials, error) {
	var pair TokenPair
	err := c.do(ctx, http.MethodPost, "/auth/refresh", "", map[string]string{"refresh_token": current.RefreshToken}, &pair)
	if err != nil {
		var auth *AuthError
		if ok := errorAs(err, &auth); ok && auth.Kind == ErrorNetwork {
			return Credentials{}, err
		}
		return Credentials{}, &AuthError{Kind: ErrorRefresh, Op: "/auth/refresh", Status: authStatus(err)}
	}
	if strings.TrimSpace(pair.AccessToken) == "" || pair.ExpiresIn <= 0 {
		return Credentials{}, &AuthError{Kind: ErrorRefresh, Op: "/auth/refresh"}
	}
	current.AccessToken = pair.AccessToken
	if token := strings.TrimSpace(pair.RefreshToken); token != "" {
		current.RefreshToken = token
	}
	current.ExpiresAt = c.now().Unix() + pair.ExpiresIn
	return current, nil
}

// errorAs is kept tiny so Refresh never formats or wraps the original error.
func errorAs(err error, target **AuthError) bool {
	for err != nil {
		if value, ok := err.(*AuthError); ok {
			*target = value
			return true
		}
		type unwrapper interface{ Unwrap() error }
		wrapped, ok := err.(unwrapper)
		if !ok {
			return false
		}
		err = wrapped.Unwrap()
	}
	return false
}

func authStatus(err error) int {
	var auth *AuthError
	if errorAs(err, &auth) {
		return auth.Status
	}
	return 0
}

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
