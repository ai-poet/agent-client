// Package cheapauth implements CheapRouter desktop sign-in and its API client.
// OAuth credentials are kept out of URLs, logs and error messages. Only the
// derived gateway API key is handed to the model provider configuration.
package cheapauth

import (
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
)

const (
	// PrimaryEndpoint and CDNEndpoint select the same CheapRouter gateway.
	PrimaryEndpoint = "https://cheaprouter.cc"
	CDNEndpoint     = "https://cdn.cheaprouter.cc"

	loginBridgePath = "/auth/paseo"
	credentialName  = "cheaprouter-account.json"
	maxResponseSize = 2 << 20
)

// RefreshSkew renews an access token before a request can race its expiry.
const RefreshSkew = 2 * time.Minute

// ErrorKind is a stable, actionable failure category for the sign-in UI.
type ErrorKind string

const (
	ErrorNetwork          ErrorKind = "network"
	ErrorCodeExpired      ErrorKind = "code_expired"
	ErrorVerifierMismatch ErrorKind = "verifier_mismatch"
	ErrorCodeInvalid      ErrorKind = "code_invalid"
	ErrorRefresh          ErrorKind = "refresh_failed"
	ErrorUnauthorized     ErrorKind = "unauthorized"
	ErrorResponse         ErrorKind = "invalid_response"
	ErrorStorage          ErrorKind = "storage"
)

// AuthError deliberately contains no server response body or transport cause:
// either could repeat a credential supplied in a malicious test/server reply.
type AuthError struct {
	Kind   ErrorKind
	Op     string
	Status int
}

func (e *AuthError) Error() string {
	if e == nil {
		return "CheapRouter 请求失败"
	}
	message := map[ErrorKind]string{
		ErrorNetwork:          "无法连接 CheapRouter，请检查网络或切换主站/CDN 后重试",
		ErrorCodeExpired:      "登录码已过期，请重新登录",
		ErrorVerifierMismatch: "登录码与本次登录不匹配，请使用当前登录页显示的登录码",
		ErrorCodeInvalid:      "登录码无效或已使用，请重新登录",
		ErrorRefresh:          "登录状态续期失败，请重新登录",
		ErrorUnauthorized:     "登录状态已失效，请重新登录",
		ErrorResponse:         "CheapRouter 返回了无法识别的响应，请稍后重试",
		ErrorStorage:          "无法安全保存 CheapRouter 登录凭据",
	}[e.Kind]
	if message == "" {
		message = "CheapRouter 请求失败"
	}
	if e.Status != 0 {
		return fmt.Sprintf("%s（HTTP %d）", message, e.Status)
	}
	return message
}

// Is lets callers use errors.Is with the sentinel categories below.
func (e *AuthError) Is(target error) bool {
	var other *AuthError
	return errors.As(target, &other) && other.Kind != "" && e.Kind == other.Kind
}

var (
	ErrNetwork          = &AuthError{Kind: ErrorNetwork}
	ErrCodeExpired      = &AuthError{Kind: ErrorCodeExpired}
	ErrVerifierMismatch = &AuthError{Kind: ErrorVerifierMismatch}
	ErrCodeInvalid      = &AuthError{Kind: ErrorCodeInvalid}
	ErrRefresh          = &AuthError{Kind: ErrorRefresh}
	ErrUnauthorized     = &AuthError{Kind: ErrorUnauthorized}
)

// Credentials is serialized only by Store. String and GoString prevent an
// accidental fmt/slog field dump from exposing either token.
type Credentials struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresAt    int64  `json:"expires_at"`
	Endpoint     string `json:"endpoint"`
	APIKey       string `json:"api_key,omitempty"`
	GroupID      *int64 `json:"group_id,omitempty"`
}

func (Credentials) String() string   { return "CheapRouter 凭据（已隐藏）" }
func (Credentials) GoString() string { return "CheapRouter 凭据（已隐藏）" }

// NeedsRefresh reports whether the token is inside its renewal window.
func (c Credentials) NeedsRefresh(now time.Time) bool {
	return now.Unix() >= c.ExpiresAt-int64(RefreshSkew/time.Second)
}

func (c Credentials) valid() bool {
	return strings.TrimSpace(c.AccessToken) != "" && strings.TrimSpace(c.RefreshToken) != "" &&
		c.ExpiresAt > 0 && strings.TrimSpace(c.Endpoint) != ""
}

// TokenPair is returned by /auth/refresh.
type TokenPair struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int64  `json:"expires_in"`
}

// DesktopSession is returned by the one-time-code exchange.
type DesktopSession struct {
	TokenPair
	APIKey string `json:"api_key"`
}

// User is the account projection used by the Chinese account page.
type User struct {
	ID               int64   `json:"id"`
	Email            string  `json:"email"`
	Username         string  `json:"username"`
	Role             string  `json:"role"`
	Balance          float64 `json:"balance"`
	Status           string  `json:"status"`
	Plan             string  `json:"plan"`
	PlanName         string  `json:"plan_name"`
	Subscription     string  `json:"subscription"`
	SubscriptionName string  `json:"subscription_name"`
}

// UnmarshalJSON accepts both the flat plan labels used by older gateways and
// additive nested subscription/plan objects, without making sign-in brittle.
func (u *User) UnmarshalJSON(data []byte) error {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	labels := map[string]json.RawMessage{}
	for _, key := range []string{"plan", "plan_name", "subscription", "subscription_name"} {
		labels[key] = fields[key]
		delete(fields, key)
	}
	plain, _ := json.Marshal(fields)
	type userAlias User
	if err := json.Unmarshal(plain, (*userAlias)(u)); err != nil {
		return err
	}
	u.Plan = planLabel(labels["plan"])
	u.PlanName = planLabel(labels["plan_name"])
	u.Subscription = planLabel(labels["subscription"])
	u.SubscriptionName = planLabel(labels["subscription_name"])
	return nil
}

func planLabel(raw json.RawMessage) string {
	if len(raw) == 0 || string(raw) == "null" {
		return ""
	}
	var text string
	if json.Unmarshal(raw, &text) == nil {
		return text
	}
	var object map[string]json.RawMessage
	if json.Unmarshal(raw, &object) != nil {
		return ""
	}
	for _, key := range []string{"name", "display_name", "title", "plan_name", "plan", "subscription"} {
		if value := planLabel(object[key]); value != "" {
			return value
		}
	}
	return ""
}

// DisplayName prefers a human account name and falls back to the email.
func (u User) DisplayName() string {
	if name := strings.TrimSpace(u.Username); name != "" {
		return name
	}
	return strings.TrimSpace(u.Email)
}

// PlanDisplay is the plan label supplied by auth/me, or a useful fallback.
func (u User) PlanDisplay() string {
	for _, value := range []string{u.PlanName, u.SubscriptionName, u.Plan, u.Subscription} {
		if value = strings.TrimSpace(value); value != "" {
			return value
		}
	}
	return "按量付费"
}

// APIKey is one gateway key. Key is never included in an AuthError.
type APIKey struct {
	ID      int64  `json:"id"`
	Key     string `json:"key"`
	Name    string `json:"name"`
	GroupID *int64 `json:"group_id"`
	Status  string `json:"status"`
}

// KeyPage is the first page of the account's gateway keys. Total counts the
// keys on every page.
type KeyPage struct {
	Items []APIKey `json:"items"`
	Total int64    `json:"total"`
}

// Group is one model routing group returned by GET /api/v1/groups/available.
type Group struct {
	ID               int64   `json:"id"`
	Name             string  `json:"name"`
	Description      string  `json:"description"`
	Platform         string  `json:"platform"`
	RateMultiplier   float64 `json:"rate_multiplier"`
	Status           string  `json:"status"`
	Category         string  `json:"category"`
	SubscriptionType string  `json:"subscription_type"`
}

// GroupRef identifies the primary group for a catalog entry.
type GroupRef struct {
	ID             int64   `json:"id"`
	Name           string  `json:"name"`
	RateMultiplier float64 `json:"rate_multiplier"`
}

// GroupCompanion is an alternate group serving the same catalog model.
type GroupCompanion struct {
	Group GroupRef `json:"group"`
}

// Model is the part of model_catalog needed to configure sailry.
type Model struct {
	ID              string           `json:"model"`
	DisplayName     string           `json:"display_name"`
	Platform        string           `json:"platform"`
	BillingMode     string           `json:"billing_mode"`
	ContextWindow   int64            `json:"context_window"`
	MaxOutputTokens int64            `json:"max_output_tokens,omitempty"`
	BestGroup       GroupRef         `json:"best_group,omitempty"`
	OtherGroups     []GroupCompanion `json:"other_groups,omitempty"`
}

// ServesGroup reports whether this catalog item is offered through groupID.
func (m Model) ServesGroup(groupID int64) bool {
	if m.BestGroup.ID == groupID {
		return true
	}
	for _, other := range m.OtherGroups {
		if other.Group.ID == groupID {
			return true
		}
	}
	return false
}

// ModelCatalog accepts the service's additive base_url field. Older gateways
// omit it, in which case the service endpoint plus /v1 is used.
type ModelCatalog struct {
	BaseURL string  `json:"base_url"`
	Items   []Model `json:"items"`
}

// UsageStats mirrors /usage/stats for clients that need account consumption.
type UsageStats struct {
	TotalRequests     int64   `json:"total_requests"`
	TotalInputTokens  int64   `json:"total_input_tokens"`
	TotalOutputTokens int64   `json:"total_output_tokens"`
	TotalTokens       int64   `json:"total_tokens"`
	TotalCost         float64 `json:"total_cost"`
}

// Gateway is the complete model-provider handoff. Its APIKey must only be
// passed to sailry's protected credential field, never written to logs.
type Gateway struct {
	BaseURL string
	APIKey  string
	Models  []Model
	GroupID *int64
}

func (g Gateway) String() string {
	return fmt.Sprintf("CheapRouter 网关（%d 个模型，密钥已隐藏）", len(g.Models))
}
func (g Gateway) GoString() string { return g.String() }
