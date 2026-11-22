package cheapauth

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

const (
	MinSpeedTestTimeoutSecs     = 2
	MaxSpeedTestTimeoutSecs     = 30
	DefaultSpeedTestTimeoutSecs = 8
	MaxSpeedTestConcurrency     = 6

	maxProbeBodyBytes = 256 << 10
)

// ClampSpeedTestTimeout brings a requested timeout in seconds into [2, 30],
// defaulting to 8 when secs <= 0.
func ClampSpeedTestTimeout(secs int) int {
	if secs <= 0 {
		return DefaultSpeedTestTimeoutSecs
	}
	if secs < MinSpeedTestTimeoutSecs {
		return MinSpeedTestTimeoutSecs
	}
	if secs > MaxSpeedTestTimeoutSecs {
		return MaxSpeedTestTimeoutSecs
	}
	return secs
}

// LatencyTier classifies measured latency using the Rust/CC Switch thresholds
// (300 / 500 / 800 ms).
type LatencyTier uint8

const (
	LatencyFast LatencyTier = iota
	LatencyOK
	LatencySlow
	LatencyVerySlow
)

// ClassifyLatency maps a successful probe latency in milliseconds to its tier.
func ClassifyLatency(ms int64) LatencyTier {
	switch {
	case ms < 300:
		return LatencyFast
	case ms < 500:
		return LatencyOK
	case ms < 800:
		return LatencySlow
	default:
		return LatencyVerySlow
	}
}

// ProbeVerdict describes the outcome of a single models-list connectivity probe.
type ProbeVerdict uint8

const (
	ProbeOK ProbeVerdict = iota
	ProbeUnauthorized
	ProbeHTTPError
	ProbeUnreachable
)

// ProbeResult holds the measurement and verdict of probing one origin.
type ProbeResult struct {
	LatencyMS int64
	Status    int
	Verdict   ProbeVerdict
	Detail    string
	Body      string
}

// CandidateResult is one candidate's speed test outcome. Exactly one of Result
// or Invalid is non-nil.
type CandidateResult struct {
	URL     string
	Result  *ProbeResult
	Invalid error
}

// IsOK reports whether the origin answered the models listing with HTTP 2xx.
func (c CandidateResult) IsOK() bool {
	return c.Result != nil && c.Result.Verdict == ProbeOK
}

// LatencyMS returns the measured latency for a successful probe.
func (c CandidateResult) LatencyMS() (int64, bool) {
	if !c.IsOK() {
		return 0, false
	}
	return c.Result.LatencyMS, true
}

// TestCandidates concurrently probes every URL's models listing using the
// account's gateway key. Results are returned in input order. Invalid URLs are
// reported in place without hitting the network.
func TestCandidates(
	ctx context.Context,
	providerKind string,
	urls []string,
	apiKey string,
	timeoutSecs int,
	httpClient *http.Client,
) []CandidateResult {
	timeout := time.Duration(ClampSpeedTestTimeout(timeoutSecs)) * time.Second
	results := make([]CandidateResult, len(urls))
	type job struct {
		index int
		url   string
	}
	pending := make([]job, 0, len(urls))
	for i, raw := range urls {
		normalized, err := NormalizeGatewayURL(raw)
		if err != nil {
			results[i] = CandidateResult{URL: raw, Invalid: err}
			continue
		}
		pending = append(pending, job{index: i, url: normalized})
	}
	if len(pending) == 0 {
		return results
	}

	var transport *http.Transport
	baseClient := httpClient
	if baseClient == nil {
		transport = &http.Transport{
			Proxy:               http.ProxyFromEnvironment,
			MaxIdleConnsPerHost: 2,
			IdleConnTimeout:     30 * time.Second,
		}
		baseClient = &http.Client{Transport: transport}
		defer transport.CloseIdleConnections()
	}

	workers := min(len(pending), MaxSpeedTestConcurrency)
	var next atomic.Int64
	var wg sync.WaitGroup
	wg.Add(workers)
	for range workers {
		go func() {
			defer wg.Done()
			for {
				idx := int(next.Add(1) - 1)
				if idx >= len(pending) {
					return
				}
				item := pending[idx]
				probe := probeTwice(ctx, baseClient, providerKind, item.url, apiKey, timeout)
				results[item.index] = CandidateResult{
					URL:    item.url,
					Result: &probe,
				}
			}
		}()
	}
	wg.Wait()
	return results
}

// probeTwice mirrors Rust speedtest::probe_twice: one warm-up request (TLS/TCP
// setup, discarded) followed by one timed request. An unreachable warm-up fails
// immediately instead of waiting a second full timeout.
func probeTwice(
	ctx context.Context,
	client *http.Client,
	providerKind, baseURL, apiKey string,
	timeout time.Duration,
) ProbeResult {
	warmUp := probeOnce(ctx, client, providerKind, baseURL, apiKey, timeout)
	if warmUp.Verdict == ProbeUnreachable || ctx.Err() != nil {
		return warmUp
	}
	return probeOnce(ctx, client, providerKind, baseURL, apiKey, timeout)
}

func probeOnce(
	ctx context.Context,
	client *http.Client,
	providerKind, baseURL, apiKey string,
	timeout time.Duration,
) ProbeResult {
	reqCtx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	targetURL := probeModelsURL(baseURL)
	req, err := http.NewRequestWithContext(reqCtx, http.MethodGet, targetURL, nil)
	if err != nil {
		return ProbeResult{Verdict: ProbeUnreachable, Detail: err.Error()}
	}
	req.Header.Set("Accept", "application/json")
	key := strings.TrimSpace(apiKey)
	if usesAnthropicProbeShape(providerKind) {
		req.Header.Set("anthropic-version", "2023-06-01")
		if key != "" {
			req.Header.Set("x-api-key", key)
		}
	} else if key != "" {
		req.Header.Set("Authorization", "Bearer "+key)
	}

	started := time.Now()
	resp, err := client.Do(req)
	latency := time.Since(started).Milliseconds()
	if err != nil {
		return ProbeResult{
			LatencyMS: latency,
			Verdict:   ProbeUnreachable,
			Detail:    err.Error(),
		}
	}
	defer resp.Body.Close()

	raw, _ := io.ReadAll(io.LimitReader(resp.Body, maxProbeBodyBytes))
	var verdict ProbeVerdict
	switch {
	case resp.StatusCode >= 200 && resp.StatusCode < 300:
		verdict = ProbeOK
	case resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden:
		verdict = ProbeUnauthorized
	default:
		verdict = ProbeHTTPError
	}
	if verdict == ProbeOK {
		return ProbeResult{
			LatencyMS: latency,
			Status:    resp.StatusCode,
			Verdict:   verdict,
			Body:      string(raw),
		}
	}
	return ProbeResult{
		LatencyMS: latency,
		Status:    resp.StatusCode,
		Verdict:   verdict,
		Detail:    probeErrorSummary(string(raw)),
	}
}

func usesAnthropicProbeShape(providerKind string) bool {
	switch strings.ToLower(strings.TrimSpace(providerKind)) {
	case "claude", "anthropic", "native_messages":
		return true
	default:
		return false
	}
}

func probeModelsURL(baseURL string) string {
	base := strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if endsWithVersionSegment(base) {
		return base + "/models"
	}
	return base + "/v1/models"
}

// endsWithVersionSegment mirrors Rust gateway::ends_with_version_segment:
// true when the last path segment names an API version (v1, v4, v1beta, ...).
func endsWithVersionSegment(base string) bool {
	base = strings.TrimRight(strings.TrimSpace(base), "/")
	afterScheme := base
	if _, rest, ok := strings.Cut(base, "://"); ok {
		afterScheme = rest
	}
	_, path, ok := strings.Cut(afterScheme, "/")
	if !ok {
		return false
	}
	segment := path
	if idx := strings.LastIndex(path, "/"); idx >= 0 {
		segment = path[idx+1:]
	}
	segment = strings.ToLower(segment)
	rest, ok := strings.CutPrefix(segment, "v")
	if !ok || rest == "" {
		return false
	}
	digits := 0
	for digits < len(rest) && rest[digits] >= '0' && rest[digits] <= '9' {
		digits++
	}
	if digits == 0 {
		return false
	}
	suffix := rest[digits:]
	return suffix == "" || strings.HasPrefix(suffix, "alpha") || strings.HasPrefix(suffix, "beta")
}

func probeErrorSummary(body string) string {
	trimmed := strings.TrimSpace(body)
	if trimmed == "" {
		return ""
	}
	var payload map[string]any
	if json.Unmarshal([]byte(trimmed), &payload) == nil {
		if errObj, ok := payload["error"].(map[string]any); ok {
			if msg, ok := errObj["message"].(string); ok && strings.TrimSpace(msg) != "" {
				return shortenSummary(msg)
			}
		}
		for _, key := range []string{"message", "error"} {
			if msg, ok := payload[key].(string); ok && strings.TrimSpace(msg) != "" {
				return shortenSummary(msg)
			}
		}
	}
	return shortenSummary(trimmed)
}

func shortenSummary(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if len(s) > 160 {
		return s[:160]
	}
	return s
}

// FastestOK returns the index of the fastest successful candidate, or -1 if none.
func FastestOK(results []CandidateResult) int {
	bestIdx := -1
	var bestMS int64
	for i, cand := range results {
		ms, ok := cand.LatencyMS()
		if !ok {
			continue
		}
		if bestIdx == -1 || ms < bestMS {
			bestIdx = i
			bestMS = ms
		}
	}
	return bestIdx
}

// SortCandidateResults returns a sorted copy ordered by availability then
// ascending latency: OK candidates first (fastest to slowest), then failed
// probes, then invalid URLs. Relative order is preserved on ties.
func SortCandidateResults(results []CandidateResult) []CandidateResult {
	ordered := append([]CandidateResult(nil), results...)
	rank := func(c CandidateResult) (int, int64) {
		if ms, ok := c.LatencyMS(); ok {
			return 0, ms
		}
		if c.Result != nil {
			return 1, 0
		}
		return 2, 0
	}
	slices.SortStableFunc(ordered, func(a, b CandidateResult) int {
		ra, ma := rank(a)
		rb, mb := rank(b)
		if ra != rb {
			return ra - rb
		}
		if ma < mb {
			return -1
		}
		if ma > mb {
			return 1
		}
		return 0
	})
	return ordered
}

// SortOriginsByResults returns a copy of candidates ordered by their speed test
// outcomes: OK (fastest first), then non-OK, then untested.
func SortOriginsByResults(candidates []string, results []CandidateResult) []string {
	ordered := append([]string(nil), candidates...)
	if len(results) == 0 {
		return ordered
	}
	byURL := make(map[string]CandidateResult, len(results))
	for _, r := range results {
		byURL[r.URL] = r
	}
	rank := func(origin string) (int, int64) {
		r, ok := byURL[origin]
		if !ok {
			return 2, 0
		}
		if ms, ok := r.LatencyMS(); ok {
			return 0, ms
		}
		return 1, 0
	}
	slices.SortStableFunc(ordered, func(a, b string) int {
		ra, ma := rank(a)
		rb, mb := rank(b)
		if ra != rb {
			return ra - rb
		}
		if ma < mb {
			return -1
		}
		if ma > mb {
			return 1
		}
		return 0
	})
	return ordered
}

// ModelIDsFromBody extracts model IDs from OpenAI ({data:[{id}]}), Anthropic,
// {models:[...]} or bare array responses, preserving order and deduplicating.
func ModelIDsFromBody(body string) []string {
	var raw any
	if err := json.Unmarshal([]byte(body), &raw); err != nil {
		return nil
	}
	var items []any
	switch v := raw.(type) {
	case []any:
		items = v
	case map[string]any:
		if data, ok := v["data"].([]any); ok {
			items = data
		} else if models, ok := v["models"].([]any); ok {
			items = models
		}
	}
	var ids []string
	for _, item := range items {
		var id string
		switch entry := item.(type) {
		case string:
			id = strings.TrimSpace(entry)
		case map[string]any:
			if s, ok := entry["id"].(string); ok {
				id = strings.TrimSpace(s)
			}
		}
		if id != "" && !slices.Contains(ids, id) {
			ids = append(ids, id)
		}
	}
	return ids
}
