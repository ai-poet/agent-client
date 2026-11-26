package trace

import (
	"math"
	"slices"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type Verdict = protocol.TraceVerdict

type Round struct {
	MaxGapUs, GapStartUs                    *uint64
	Provider                                Provider
	StartedMs                               int64
	Status                                  int
	Outcome, Failure, End, Gateway          string
	FirstUs, ElapsedUs, Output, Input, Read *uint64
	HasUsage, AfterCompaction, DialTimeout  bool
	RetryAfterMs, RetryDelayMs              *uint64
	RetrySequence                           []string
	Segments                                []segment
}

func finding(code, severity, action string) Verdict {
	v := Verdict{Code: code, Severity: severity, Summary: "diagnosis_" + code}
	if action != "" {
		v.Actions = []string{"diagnosis_action_" + action}
	}
	return v
}

func Diagnose(r Round, predecessor *Round) Verdict {
	v := finding("normal", "info", "")
	switch {
	case r.Failure == "cancelled" || r.Outcome == "cancelled":
		v = finding("cancelled", "info", "")
	case r.Failure == "quota" || r.Status == 402:
		v = finding("quota", "error", "balance")
	case r.Failure == "context_overflow":
		v = finding("context_overflow", "error", "compact")
	case r.Status == 401 || r.Failure == "authentication":
		v = finding("authentication", "error", "credential")
	case r.Status == 429 || r.Failure == "rate_limited":
		v = finding("rate_limited", "warning", "wait")
	case r.Status >= 500 && r.Status < 600 || r.Failure == "unavailable":
		v = finding("unavailable", "error", "route")
	case r.Failure == "invalid_response" || r.End == "stream-truncated":
		v = finding("invalid_response", "error", "protocol")
	case r.Failure == "network" && r.Status == 0:
		v = finding("network", "error", "network")
		if r.DialTimeout {
			v.CauseKey = "tcp-timeout"
		}
	case r.Status == 400 || r.Failure == "invalid_request":
		v = finding("invalid_request", "error", "protocol")
	case r.Outcome == "failed":
		v = finding("failed", "error", "route")
	case r.FirstUs != nil && *r.FirstUs > 30_000_000:
		v = finding("slow_first", "warning", "route")
	case r.FirstUs != nil && r.MaxGapUs != nil && *r.MaxGapUs > 30_000_000:
		v = finding("slow_output", "warning", "timing")
		v.MaxGapUs, v.GapStartUs = r.MaxGapUs, r.GapStartUs
	case r.FirstUs != nil && r.ElapsedUs != nil && r.Output != nil && *r.ElapsedUs > *r.FirstUs && *r.ElapsedUs-*r.FirstUs > 30_000_000 && (*r.ElapsedUs-*r.FirstUs)/max(1, *r.Output) > 1_000_000:
		v = finding("slow_output", "warning", "timing")
	case r.Outcome == "succeeded" && !r.HasUsage:
		v = finding("no_usage", "warning", "billing")
	case r.Outcome == "succeeded" && predecessor != nil:
		if cache, ok := diagnoseCache(r, *predecessor); ok {
			v = cache
		}
	}
	if r.Status == 403 {
		v.Actions = append(v.Actions, "diagnosis_action_account")
	}
	if r.Gateway != "" && r.Gateway != "[redacted]" {
		v.Actions = append(v.Actions, "connections_copy_id")
	}
	if len(r.RetrySequence) > 1 {
		v.RetrySequence = slices.Clone(r.RetrySequence)
		for _, s := range r.RetrySequence[1:] {
			if s != r.RetrySequence[0] {
				v.Actions = append(v.Actions, "diagnosis_action_fluctuation")
				break
			}
		}
	}
	v.RetryAfterMs, v.RetryDelayMs = r.RetryAfterMs, r.RetryDelayMs
	return v
}

func diagnoseCache(r, p Round) (Verdict, bool) {
	// Gemini reports cachedContentTokenCount, but that alone establishes
	// neither eligible prefix size nor cache lifetime for this model/route.
	if r.Provider.API == "gemini" {
		return Verdict{}, false
	}
	if r.Provider != p.Provider || p.Outcome != "succeeded" || r.StartedMs < p.StartedMs || r.StartedMs-p.StartedMs > 3_600_000 || len(p.Segments) == 0 || len(r.Segments) == 0 || r.Input == nil || r.Read == nil {
		return Verdict{}, false
	}
	cur, old := r.Segments, p.Segments
	shared := 0
	for shared < min(len(cur), len(old)) && cur[shared].same(old[shared]) {
		shared++
	}
	if shared == 0 {
		// Also validate a caller-supplied predecessor (including an older
		// capture with no index sketch) against its actual sidecar.
		if !matchBranches(fingerprint(cur)).score(fingerprint(old)).eligible() {
			return Verdict{}, false
		}
	}
	size := func(s []segment) float64 {
		n := 0
		for _, v := range s {
			n += len(v.text)
		}
		return float64(n)
	}
	totalSize := max(1, size(cur))
	estimate := func(s []segment) float64 { return size(s) / totalSize * float64(*r.Input) }
	reachable := estimate(cur[:shared])
	expected, possible := reachable, estimate(old)
	noBreak := false
	if r.Provider.API == "anthropic" {
		last, common := -1, -1
		for i, s := range old {
			if s.breakpoint {
				last = i
				if i < shared {
					common = i
				}
			}
		}
		possible = estimate(old[:last+1])
		valid := false
		if common >= 0 {
			for i := common; i < min(len(cur), common+21); i++ {
				valid = valid || cur[i].breakpoint
			}
		}
		expected = estimate(old[:common+1])
		if !valid {
			noBreak = true
			expected = 0
		}
	} else {
		expected = math.Floor(expected/128) * 128
	}
	read := float64(*r.Read)
	miss := func(code, cause string, estimate float64) Verdict {
		v := finding(code, "warning", "prefix")
		v.CauseKey = cause
		v.LostTokens = uint64(math.Round(max(0, estimate-read)))
		return v
	}
	firstUser := func(s []segment) int { return slices.IndexFunc(s, func(v segment) bool { return v.role == "user" }) }
	newConversation := firstUser(old) == shared && firstUser(cur) == shared && !r.AfterCompaction
	if shared < len(old) && !newConversation && possible-expected >= 1024 && read < possible {
		cause, path, volatile := divergence(old, cur, shared, r.AfterCompaction)
		v := miss("prefix_broken", cause, possible)
		v.Path, v.Volatile = path, volatile
		if shared < len(cur) && !r.AfterCompaction && onlyKeyOrder(old[shared], cur[shared]) {
			v.KeyOrderOnly = true
		}
		return v, true
	}
	if noBreak && reachable >= 1024 && read == 0 {
		return miss("cache_miss", "no-breakpoints", reachable), true
	}
	if expected < 1024 || read >= expected*.8 {
		return Verdict{}, false
	}
	code, cause := "cache_miss", "account-switch"
	if read >= expected*.2 {
		code = "cache_partial"
	}
	if read > 0 {
		cause = "cache-evicted"
	}
	if p.FirstUs != nil && r.StartedMs-p.StartedMs < int64(*p.FirstUs/1000) {
		cause = "concurrent"
	} else {
		end := p.StartedMs
		if p.ElapsedUs != nil {
			end += int64(*p.ElapsedUs / 1000)
		}
		if r.StartedMs-end > 300_000 {
			cause = "expired"
		}
	}
	v := miss(code, cause, expected)
	if cause == "account-switch" {
		v.Actions = append(v.Actions, "diagnosis_action_account")
	}
	return v, true
}
