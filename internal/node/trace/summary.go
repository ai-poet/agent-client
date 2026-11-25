package trace

import (
	"bytes"
	"fmt"
	"maps"
	"slices"
	"strconv"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func safeFailure(s string) string {
	switch s {
	case "not_configured", "authentication", "context_overflow", "rate_limited", "quota", "network", "unavailable", "invalid_request", "invalid_response", "unsupported", "cancelled", "other":
		return s
	}
	return "unknown"
}

// Report only captured evidence. An HTTP status or missing usage isn't proof of
// a particular upstream account, cache eviction, or unreported token loss.
func conclusion(r row) string {
	if r.Verdict != nil {
		// Never project arbitrary index strings into the Markdown report.
		switch r.Verdict.Code {
		case "normal", "cancelled", "quota", "context_overflow", "authentication", "rate_limited", "unavailable", "invalid_response", "network", "invalid_request", "failed", "slow_first", "slow_output", "no_usage", "prefix_broken", "cache_miss", "cache_partial":
			return r.Verdict.Code
		}
	}
	if r.failed() {
		if r.Failure != nil {
			return safeFailure(r.Failure.Kind)
		}
		return "failed"
	}
	if r.Outcome == "cancelled" {
		return "cancelled"
	}
	if r.Outcome == "succeeded" {
		if r.Usage.Input == nil && r.Usage.Output == nil {
			return "succeeded; usage unknown"
		}
		return "succeeded"
	}
	return "unknown"
}
func summary(rows []row) []byte {
	type turn struct {
		count, failed, retries int
		elapsed                []uint64
	}
	turns := map[protocol.TurnID]*turn{}
	var order []protocol.TurnID
	dist := map[string]int{}
	lost := map[string]uint64{}
	var failures strings.Builder
	var times []uint64
	failed, retries, succeeded := 0, 0, 0
	for _, r := range rows {
		t := turns[r.Turn]
		if t == nil {
			t = &turn{}
			turns[r.Turn] = t
			order = append(order, r.Turn)
		}
		t.count++
		if r.Attempt > 1 {
			retries++
			t.retries++
		}
		if r.Outcome == "succeeded" {
			succeeded++
		}
		label := conclusion(r)
		dist[label]++
		if r.Verdict != nil {
			lost[label] += r.Verdict.LostTokens
		}
		if r.failed() {
			failed++
			t.failed++
			fmt.Fprintf(&failures, "- Turn %s, attempt %d: %s (HTTP %s)\n", r.Turn, r.Attempt, label, httpStatus(r.Status))
		}
		if r.Timing.Elapsed != nil {
			t.elapsed = append(t.elapsed, *r.Timing.Elapsed)
		}
		if r.Timing.First != nil {
			times = append(times, *r.Timing.First)
		}
	}
	var b bytes.Buffer
	fmt.Fprintf(&b, "# Request trace\n\n## KPIs\n\nAttempts: %d; successful: %d; failed: %d; retry attempts: %d.\n\nFirst-token p50 (us): %s (%d samples).\n\n", len(rows), succeeded, failed, retries, p50(times), len(times))
	b.WriteString("## Conclusion distribution\n\n")
	for _, s := range slices.Sorted(maps.Keys(dist)) {
		fmt.Fprintf(&b, "- %s: %d; estimated lost tokens: %d\n", s, dist[s], lost[s])
	}
	b.WriteString("\n## Turns\n\n| Turn | Attempts | Failed | Retry attempts | Elapsed p50 (us) |\n|---|---:|---:|---:|---:|\n")
	for _, id := range order {
		t := turns[id]
		fmt.Fprintf(&b, "| %s | %d | %d | %d | %s |\n", id, t.count, t.failed, t.retries, p50(t.elapsed))
	}
	b.WriteString("\n## Failed attempts\n\n")
	if failed == 0 {
		b.WriteString("None.\n")
	} else {
		b.WriteString(failures.String())
	}
	return b.Bytes()
}
func p50(samples []uint64) string {
	if len(samples) == 0 {
		return "unknown"
	}
	slices.Sort(samples)
	return strconv.FormatUint(samples[(len(samples)-1)/2], 10)
}
func httpStatus(status int) string {
	if status == 0 {
		return "unknown"
	}
	return strconv.Itoa(status)
}
