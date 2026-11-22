package cheapauth

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"unicode"
)

const (
	// GatewayOriginFileName is stored in the profile directory alongside credentials.
	GatewayOriginFileName = "gateway-origin.json"

	// ServerNodesTTLSeconds is how long the server's public node list is trusted
	// before it is requested again (6 hours, matching the Rust client).
	ServerNodesTTLSeconds int64 = 6 * 60 * 60

	// MaxServerNodes caps how many public server nodes are retained.
	MaxServerNodes = 10

	defaultConnectTimeoutSecs uint32 = 15
	defaultIdleTimeoutSecs    uint32 = 300
)

// URLErrorKind describes why a candidate gateway URL was rejected.
type URLErrorKind uint8

const (
	URLErrorEmpty URLErrorKind = iota + 1
	URLErrorWhitespace
	URLErrorScheme
	URLErrorNoHost
)

// URLError reports a rejected candidate gateway URL without echoing secrets.
type URLError struct {
	Kind   URLErrorKind
	Scheme string
}

func (e *URLError) Error() string {
	if e == nil {
		return "invalid gateway URL"
	}
	switch e.Kind {
	case URLErrorEmpty:
		return "gateway URL is empty"
	case URLErrorWhitespace:
		return "gateway URL must not contain whitespace"
	case URLErrorScheme:
		return fmt.Sprintf("unsupported URL scheme %q (expected http or https)", e.Scheme)
	case URLErrorNoHost:
		return "gateway URL has no host"
	default:
		return "invalid gateway URL"
	}
}

// BuiltInEndpoints returns the managed service origins shipped with this build:
// primary first, then the CDN node (deduplicated).
func BuiltInEndpoints() []string {
	out := make([]string, 0, 2)
	for _, raw := range []string{PrimaryEndpoint, CDNEndpoint} {
		norm, err := NormalizeGatewayURL(raw)
		if err != nil {
			continue
		}
		if !slices.Contains(out, norm) {
			out = append(out, norm)
		}
	}
	return out
}

// NormalizeGatewayURL turns user or server input into the canonical origin used
// by gateway routing and speed tests: prepends https:// when no scheme is
// supplied, lowercases the scheme, strips trailing slashes and a trailing /v1,
// and rejects non-http(s) URLs, whitespace, userinfo, query strings and fragments.
func NormalizeGatewayURL(raw string) (string, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return "", &URLError{Kind: URLErrorEmpty}
	}
	if strings.IndexFunc(trimmed, unicode.IsSpace) != -1 {
		return "", &URLError{Kind: URLErrorWhitespace}
	}
	withScheme := trimmed
	if !strings.Contains(withScheme, "://") {
		withScheme = "https://" + withScheme
	}
	schemeRaw, rest, ok := strings.Cut(withScheme, "://")
	if !ok {
		return "", &URLError{Kind: URLErrorNoHost}
	}
	scheme := strings.ToLower(schemeRaw)
	if scheme != "http" && scheme != "https" {
		return "", &URLError{Kind: URLErrorScheme, Scheme: scheme}
	}
	hostPart, _, _ := strings.Cut(rest, "/")
	if hostPart == "" {
		return "", &URLError{Kind: URLErrorNoHost}
	}
	path := strings.TrimRight(rest, "/")
	if len(path) >= 3 && strings.EqualFold(path[len(path)-3:], "/v1") {
		path = strings.TrimRight(path[:len(path)-3], "/")
	}
	candidate := scheme + "://" + path
	parsed, err := url.Parse(candidate)
	if err != nil || parsed.Hostname() == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", &URLError{Kind: URLErrorNoHost}
	}
	return candidate, nil
}

// serverNodeOrigin accepts only bare HTTPS origins (a trailing /v1 is stripped
// by NormalizeGatewayURL). Entries with extra path prefixes are ignored because
// /settings/public can also list protocol-specific paths for the web console.
func serverNodeOrigin(raw string) (string, bool) {
	origin, err := NormalizeGatewayURL(raw)
	if err != nil {
		return "", false
	}
	host, ok := strings.CutPrefix(origin, "https://")
	if !ok || host == "" || strings.Contains(host, "/") {
		return "", false
	}
	return origin, true
}

// PublicEndpoint is one entry from GET /api/v1/settings/public custom_endpoints.
type PublicEndpoint struct {
	Name        string `json:"name"`
	Endpoint    string `json:"endpoint"`
	Description string `json:"description"`
}

// ServerNode is one cached node adopted from the service's public node list.
type ServerNode struct {
	Name        string `json:"name"`
	Origin      string `json:"origin"`
	Description string `json:"description"`
}

// NetworkTimeouts mirrors Rust GatewayOriginConfig.network for model requests.
type NetworkTimeouts struct {
	ConnectSecs uint32 `json:"connect_secs"`
	IdleSecs    uint32 `json:"idle_secs"`
}

// DefaultNetworkTimeouts returns the Rust-compatible defaults (15s connect, 300s idle).
func DefaultNetworkTimeouts() NetworkTimeouts {
	return NetworkTimeouts{
		ConnectSecs: defaultConnectTimeoutSecs,
		IdleSecs:    defaultIdleTimeoutSecs,
	}
}

func (n *NetworkTimeouts) UnmarshalJSON(data []byte) error {
	type wire struct {
		ConnectSecs *uint32 `json:"connect_secs"`
		IdleSecs    *uint32 `json:"idle_secs"`
	}
	var w wire
	if err := json.Unmarshal(data, &w); err != nil {
		return err
	}
	*n = DefaultNetworkTimeouts()
	if w.ConnectSecs != nil {
		n.ConnectSecs = *w.ConnectSecs
	}
	if w.IdleSecs != nil {
		n.IdleSecs = *w.IdleSecs
	}
	return nil
}

// GatewayOriginConfig persists which managed-service origin carries business
// API, agent and image traffic. Authentication (login, exchange, refresh,
// logout) always stays on the login origin in Credentials.Endpoint.
type GatewayOriginConfig struct {
	Candidates           []string        `json:"candidates"`
	Chosen               *string         `json:"chosen"`
	AutoSelect           bool            `json:"auto_select"`
	ServerNodes          []ServerNode    `json:"server_nodes"`
	ServerNodesFetchedAt *int64          `json:"server_nodes_fetched_at"`
	FallbackFrom         *string         `json:"fallback_from"`
	Network              NetworkTimeouts `json:"network"`
}

// DefaultGatewayOriginConfig returns a clean default configuration.
func DefaultGatewayOriginConfig() GatewayOriginConfig {
	return GatewayOriginConfig{
		Candidates:  []string{},
		ServerNodes: []ServerNode{},
		Network:     DefaultNetworkTimeouts(),
	}
}

func (c *GatewayOriginConfig) UnmarshalJSON(data []byte) error {
	type wire GatewayOriginConfig
	v := wire(DefaultGatewayOriginConfig())
	if err := json.Unmarshal(data, &v); err != nil {
		return err
	}
	if v.Candidates == nil {
		v.Candidates = []string{}
	}
	if v.ServerNodes == nil {
		v.ServerNodes = []ServerNode{}
	}
	v.Chosen = normalizeOptionalString(v.Chosen)
	v.FallbackFrom = normalizeOptionalString(v.FallbackFrom)
	*c = GatewayOriginConfig(v)
	return nil
}

func normalizeOptionalString(ptr *string) *string {
	if ptr == nil {
		return nil
	}
	trimmed := strings.TrimSpace(*ptr)
	if trimmed == "" {
		return nil
	}
	return &trimmed
}

// Clone returns a deep copy of the configuration.
func (c GatewayOriginConfig) Clone() GatewayOriginConfig {
	out := c
	out.Candidates = append([]string{}, c.Candidates...)
	out.ServerNodes = append([]ServerNode{}, c.ServerNodes...)
	if c.Chosen != nil {
		v := *c.Chosen
		out.Chosen = &v
	}
	if c.ServerNodesFetchedAt != nil {
		v := *c.ServerNodesFetchedAt
		out.ServerNodesFetchedAt = &v
	}
	if c.FallbackFrom != nil {
		v := *c.FallbackFrom
		out.FallbackFrom = &v
	}
	return out
}

// NodeLabelKind identifies how a candidate origin is presented in the UI.
type NodeLabelKind uint8

const (
	NodeLabelPrimary NodeLabelKind = iota
	NodeLabelServer
	NodeLabelBuiltInCDN
	NodeLabelOther
)

// NodeLabel carries the display classification and admin metadata for a node.
type NodeLabel struct {
	Kind        NodeLabelKind
	Name        string
	Description string
}

// EffectiveCandidates returns every origin that may be probed or chosen:
// built-in origins first, then cached server nodes, then user-added origins,
// normalized and deduplicated.
func (c GatewayOriginConfig) EffectiveCandidates() []string {
	var origins []string
	push := func(raw string) {
		norm, err := NormalizeGatewayURL(raw)
		if err != nil {
			return
		}
		if !slices.Contains(origins, norm) {
			origins = append(origins, norm)
		}
	}
	for _, ep := range BuiltInEndpoints() {
		push(ep)
	}
	for _, node := range c.ServerNodes {
		push(node.Origin)
	}
	for _, cand := range c.Candidates {
		push(cand)
	}
	return origins
}

// IsRemovable reports whether origin was added by the user and may be removed.
// Built-in and server-listed nodes are permanent.
func (c GatewayOriginConfig) IsRemovable(origin string) bool {
	norm, err := NormalizeGatewayURL(origin)
	if err != nil {
		norm = origin
	}
	for _, ep := range BuiltInEndpoints() {
		if ep == norm {
			return false
		}
	}
	for _, node := range c.ServerNodes {
		if nodeNorm, err := NormalizeGatewayURL(node.Origin); err == nil && nodeNorm == norm {
			return false
		}
		if node.Origin == norm {
			return false
		}
	}
	return true
}

// NodeLabel classifies origin for picker presentation. Primary keeps its own
// label; otherwise server-provided naming takes precedence over the built-in
// CDN label so admins can update wording without a client release.
func (c GatewayOriginConfig) NodeLabel(origin string) NodeLabel {
	norm, err := NormalizeGatewayURL(origin)
	if err != nil {
		norm = origin
	}
	is := func(raw string) bool {
		if strings.TrimSpace(raw) == "" {
			return false
		}
		known, err := NormalizeGatewayURL(raw)
		return err == nil && known == norm
	}
	if is(PrimaryEndpoint) {
		return NodeLabel{Kind: NodeLabelPrimary}
	}
	for _, node := range c.ServerNodes {
		if is(node.Origin) {
			return NodeLabel{
				Kind:        NodeLabelServer,
				Name:        node.Name,
				Description: node.Description,
			}
		}
	}
	if is(CDNEndpoint) {
		return NodeLabel{Kind: NodeLabelBuiltInCDN}
	}
	return NodeLabel{Kind: NodeLabelOther}
}

// ServerNodesDue reports whether the cached public server node list is missing
// or older than ServerNodesTTLSeconds (6 hours).
func (c GatewayOriginConfig) ServerNodesDue(now int64) bool {
	if c.ServerNodesFetchedAt == nil {
		return true
	}
	return now-*c.ServerNodesFetchedAt >= ServerNodesTTLSeconds
}

// AdoptServerNodes replaces the cached public node list with valid bare HTTPS
// entries (deduplicated and capped at MaxServerNodes = 10) and records now as
// server_nodes_fetched_at. Returns whether ServerNodes changed.
func (c *GatewayOriginConfig) AdoptServerNodes(listed []PublicEndpoint, now int64) bool {
	nodes := make([]ServerNode, 0, min(len(listed), MaxServerNodes))
	for _, entry := range listed {
		origin, ok := serverNodeOrigin(entry.Endpoint)
		if !ok {
			continue
		}
		if slices.ContainsFunc(nodes, func(known ServerNode) bool { return known.Origin == origin }) {
			continue
		}
		nodes = append(nodes, ServerNode{
			Name:        strings.TrimSpace(entry.Name),
			Origin:      origin,
			Description: strings.TrimSpace(entry.Description),
		})
		if len(nodes) == MaxServerNodes {
			break
		}
	}
	ts := now
	c.ServerNodesFetchedAt = &ts
	if slices.Equal(nodes, c.ServerNodes) {
		return false
	}
	c.ServerNodes = nodes
	return true
}

// SelectedOrigin returns the explicitly chosen origin while it is still present
// in EffectiveCandidates, or "" otherwise. Unlike Origin(), this does not fall
// back to PrimaryEndpoint, preserving self-hosted login endpoints until the
// user picks a node.
func (c GatewayOriginConfig) SelectedOrigin() string {
	if c.Chosen == nil {
		return ""
	}
	chosen, err := NormalizeGatewayURL(*c.Chosen)
	if err != nil {
		return ""
	}
	if slices.Contains(c.EffectiveCandidates(), chosen) {
		return chosen
	}
	return ""
}

// Origin returns the routed gateway origin: SelectedOrigin when set and still
// a candidate, otherwise the build's primary origin.
func (c GatewayOriginConfig) Origin() string {
	if selected := c.SelectedOrigin(); selected != "" {
		return selected
	}
	candidates := c.EffectiveCandidates()
	if len(candidates) > 0 {
		return candidates[0]
	}
	return ""
}

// ResolveAPIOrigin returns the business API origin for a session logged in on
// loginEndpoint: the explicitly selected gateway origin if valid, otherwise
// loginEndpoint. Mirrors Rust sub2api::api_origin_with.
func (c GatewayOriginConfig) ResolveAPIOrigin(loginEndpoint string) string {
	if selected := c.SelectedOrigin(); selected != "" {
		return selected
	}
	return loginEndpoint
}

// Select chooses origin (adding it as a candidate if new) and clears any
// pending fallback_from because this is a deliberate selection.
func (c *GatewayOriginConfig) Select(origin string) error {
	normalized, err := NormalizeGatewayURL(origin)
	if err != nil {
		return err
	}
	if _, err := c.Add(normalized); err != nil {
		return err
	}
	c.Chosen = &normalized
	c.FallbackFrom = nil
	return nil
}

// FallBack moves traffic to origin because the current origin stopped
// answering, remembering the previous origin in FallbackFrom so traffic can
// return once it recovers. Repeated fallbacks preserve the original origin.
func (c *GatewayOriginConfig) FallBack(origin string) error {
	from := ""
	if c.FallbackFrom != nil {
		from = *c.FallbackFrom
	} else {
		from = c.Origin()
	}
	if err := c.Select(origin); err != nil {
		return err
	}
	if from != "" && (c.Chosen == nil || from != *c.Chosen) {
		saved := from
		c.FallbackFrom = &saved
	}
	return nil
}

// RestoreTarget returns the origin recorded in FallbackFrom when it is still a
// candidate and not already the active origin.
func (c GatewayOriginConfig) RestoreTarget() string {
	if c.FallbackFrom == nil {
		return ""
	}
	from, err := NormalizeGatewayURL(*c.FallbackFrom)
	if err != nil || from == "" {
		return ""
	}
	if c.Origin() == from {
		return ""
	}
	if slices.Contains(c.EffectiveCandidates(), from) {
		return from
	}
	return ""
}

// Add records a user-supplied candidate origin. Returns (false, nil) when it is
// already known.
func (c *GatewayOriginConfig) Add(origin string) (bool, error) {
	normalized, err := NormalizeGatewayURL(origin)
	if err != nil {
		return false, err
	}
	if slices.Contains(c.EffectiveCandidates(), normalized) {
		return false, nil
	}
	c.Candidates = append(c.Candidates, normalized)
	return true, nil
}

// Remove deletes a user-added candidate origin. Removing the chosen origin
// resets Chosen to nil (falling back to primary) and clears FallbackFrom if it
// matched the removed origin.
func (c *GatewayOriginConfig) Remove(origin string) {
	norm, err := NormalizeGatewayURL(origin)
	if err != nil {
		norm = origin
	}
	c.Candidates = slices.DeleteFunc(c.Candidates, func(known string) bool {
		if known == norm {
			return true
		}
		kn, err := NormalizeGatewayURL(known)
		return err == nil && kn == norm
	})
	if c.Chosen != nil {
		chosenNorm, err := NormalizeGatewayURL(*c.Chosen)
		if *c.Chosen == norm || (err == nil && chosenNorm == norm) {
			c.Chosen = nil
		}
	}
	if c.FallbackFrom != nil {
		fbNorm, err := NormalizeGatewayURL(*c.FallbackFrom)
		if *c.FallbackFrom == norm || (err == nil && fbNorm == norm) {
			c.FallbackFrom = nil
		}
	}
}

// OriginTestAction describes what routing action a speed test result warrants.
type OriginTestAction uint8

const (
	OriginActionKeep OriginTestAction = iota
	OriginActionFallback
	OriginActionRestored
	OriginActionAutoSelected
	OriginActionNoOK
)

// OriginTestDecision is the outcome of applying speed test results against a
// GatewayOriginConfig.
type OriginTestDecision struct {
	Action OriginTestAction
	Target string
}

// ApplyOriginTest applies the Rust speed test routing rules in place:
//  1. If the currently routed origin failed and another candidate answered OK,
//     fall back to the fastest available candidate (recording fallback_from).
//  2. Else if auto_select is off and fallback_from now answers OK, restore to
//     fallback_from (clearing fallback_from).
//  3. Else if auto_select is on and a faster available candidate differs from
//     the routed origin, select it.
//  4. If no candidate answered OK, keep the current origin and report
//     OriginActionNoOK.
func ApplyOriginTest(cfg *GatewayOriginConfig, results []CandidateResult) OriginTestDecision {
	routed := cfg.Origin()
	fastestIdx := FastestOK(results)
	fastest := ""
	if fastestIdx >= 0 {
		fastest = results[fastestIdx].URL
	}
	answers := func(origin string) bool {
		if origin == "" {
			return false
		}
		for _, r := range results {
			if r.URL == origin && r.IsOK() {
				return true
			}
		}
		return false
	}

	routedOK := routed != "" && answers(routed)
	if !routedOK && fastest != "" && routed != fastest {
		if err := cfg.FallBack(fastest); err == nil {
			return OriginTestDecision{Action: OriginActionFallback, Target: fastest}
		}
	}
	if !cfg.AutoSelect {
		if home := cfg.RestoreTarget(); home != "" && answers(home) {
			if err := cfg.Select(home); err == nil {
				return OriginTestDecision{Action: OriginActionRestored, Target: home}
			}
		}
		if fastest == "" {
			return OriginTestDecision{Action: OriginActionNoOK}
		}
		return OriginTestDecision{Action: OriginActionKeep, Target: routed}
	}
	if fastest == "" {
		return OriginTestDecision{Action: OriginActionNoOK}
	}
	if routed != fastest {
		if err := cfg.Select(fastest); err == nil {
			return OriginTestDecision{Action: OriginActionAutoSelected, Target: fastest}
		}
	}
	return OriginTestDecision{Action: OriginActionKeep, Target: routed}
}

// GatewayOriginStore owns gateway-origin.json inside the profile directory.
type GatewayOriginStore struct {
	path string
}

// NewGatewayOriginStore creates a store backed by <profileDir>/gateway-origin.json.
func NewGatewayOriginStore(profileDir string) (*GatewayOriginStore, error) {
	if strings.TrimSpace(profileDir) == "" {
		return nil, &AuthError{Kind: ErrorStorage, Op: "gateway_origin"}
	}
	return &GatewayOriginStore{path: filepath.Join(profileDir, GatewayOriginFileName)}, nil
}

func (s *GatewayOriginStore) Path() string { return s.path }

// Load reads gateway-origin.json. Missing, empty, unreadable, or corrupt files
// fall back to DefaultGatewayOriginConfig without failing.
func (s *GatewayOriginStore) Load() GatewayOriginConfig {
	if s == nil || s.path == "" {
		return DefaultGatewayOriginConfig()
	}
	raw, err := os.ReadFile(s.path)
	if err != nil || len(bytes.TrimSpace(raw)) == 0 {
		return DefaultGatewayOriginConfig()
	}
	var cfg GatewayOriginConfig
	if err := json.Unmarshal(raw, &cfg); err != nil {
		return DefaultGatewayOriginConfig()
	}
	return cfg
}

// Save atomically writes gateway-origin.json with 0600 permissions.
func (s *GatewayOriginStore) Save(cfg GatewayOriginConfig) error {
	if s == nil || s.path == "" {
		return &AuthError{Kind: ErrorStorage, Op: "gateway_origin"}
	}
	if cfg.Candidates == nil {
		cfg.Candidates = []string{}
	}
	if cfg.ServerNodes == nil {
		cfg.ServerNodes = []ServerNode{}
	}
	if cfg.Network.ConnectSecs == 0 && cfg.Network.IdleSecs == 0 {
		cfg.Network = DefaultNetworkTimeouts()
	}
	encoded, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "gateway_origin_encode"}
	}
	encoded = append(encoded, '\n')
	if err := WritePrivateFile(s.path, encoded); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "gateway_origin_save"}
	}
	return nil
}

// ErrNoGatewayKey is returned when no gateway API key is available to probe origins.
var ErrNoGatewayKey = errors.New("no gateway API key available for speed test")

// ProbeTarget selects the wire protocol ("claude" or "codex") and gateway API
// key to probe candidate origins with, mirroring Rust gateway_origin::probe_target.
func ProbeTarget(credentials Credentials) (providerKind, apiKey string, ok bool) {
	if binding, exists := credentials.LaneBindings[LaneClaude]; exists {
		if key := strings.TrimSpace(binding.APIKey); key != "" {
			return "claude", key, true
		}
	}
	if binding, exists := credentials.LaneBindings[LaneCodex]; exists {
		if key := strings.TrimSpace(binding.APIKey); key != "" {
			return "codex", key, true
		}
	}
	if key := strings.TrimSpace(credentials.APIKey); key != "" {
		return "codex", key, true
	}
	for _, lane := range CanonicalLanes {
		if binding, exists := credentials.LaneBindings[lane]; exists {
			if key := strings.TrimSpace(binding.APIKey); key != "" {
				if lane == LaneClaude {
					return "claude", key, true
				}
				return "codex", key, true
			}
		}
	}
	return "", "", false
}
