package insights

import (
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestObservedFilterAndPageCursor(t *testing.T) {
	now := time.Date(2026, 12, 1, 9, 0, 0, 0, time.FixedZone("fixture", 8*3600))
	f := Filters{Days: 7, Status: "429", FailureKind: "rate-limit", Outcome: "failed", CacheMissOnly: true, ObservedOnly: true, WithoutCredential: true}
	q, err := f.Query(now)
	if err != nil || q.StartMs != now.AddDate(0, 0, -7).UnixMilli() || q.EndMs != now.UnixMilli() || q.UtcOffsetMinutes != 480 || q.Before != nil || q.Statuses[0] != 429 || q.FailureKinds[0] != "rate-limit" || q.Outcomes[0] != "failed" || !q.ObservedOnly || !q.CacheMissOnly || !q.WithoutCredential {
		t.Fatal(q, err)
	}
	f.Status = "429x"
	if _, err = f.Query(now); err == nil {
		t.Fatal("invalid status silently broadened the query")
	}
	pos := protocol.UsagePosition{Node: protocol.NodeID{1}, Sequence: 21, TimestampMs: now.UnixMilli() - 1, Ordinal: 2}
	s := State{Report: &protocol.UsageReport{Requests: protocol.UsagePage{HasMore: true, Items: []protocol.UsageRequest{{Position: pos}}}}}
	if s.Next() == nil || *s.Next() != pos {
		t.Fatal("lost composite cursor")
	}
	s.Report.Requests.HasMore = false
	if s.Next() != nil {
		t.Fatal("continued past final page")
	}
}

func TestGroupToIndexPreservesOwnerAndModel(t *testing.T) {
	m := protocol.UsageKeyModelPayload{Model: "same-name", Provider: protocol.ProviderID{42}}
	for _, data := range []any{m, &m} {
		f, ok := narrow(Filters{Days: 7, Outcome: "failed"}, protocol.UsageKey{Kind: "model", Data: data})
		if !ok || f.Model != m.Model || f.Provider != m.Provider.String() || f.Outcome != "failed" {
			t.Fatal(f)
		}
	}
	if percent(0, 0) == "0.0%" || percent(0, 3) != "0.0%" {
		t.Fatal("missing denominator conflated with zero")
	}
}

func TestCustomRangeIncludesBothLocalDates(t *testing.T) {
	zone := time.FixedZone("fixture", -3*3600)
	now := time.Date(2026, 12, 1, 2, 0, 0, 0, zone)
	f := Filters{StartDate: "2026-11-02", EndDate: "2026-11-03"}
	q, err := f.Query(now)
	if err != nil || q.StartMs != time.Date(2026, 11, 2, 0, 0, 0, 0, zone).UnixMilli() || q.EndMs != time.Date(2026, 11, 4, 0, 0, 0, 0, zone).UnixMilli() {
		t.Fatal(q, err)
	}
	f.EndDate = "2026-10-31"
	if _, err := f.Query(now); err == nil {
		t.Fatal("accepted reversed range")
	}
}

func TestIndexOpensExactAttempt(t *testing.T) {
	id := "req-same"
	attempt := uint64(2)
	want := protocol.UsageRequest{Session: protocol.SessionID{2}, Turn: protocol.TurnID{3}, RequestID: &id, Attempt: &attempt}
	var opened protocol.UsageRequest
	state := State{Tab: 1, Draft: Filters{Days: 7, ObservedOnly: true}, Report: &protocol.UsageReport{Requests: protocol.UsagePage{Items: []protocol.UsageRequest{want}}}}
	tt := ui.NewTester(func(c *ui.Context) {
		Page(c, &state, Actions{Host: func(*ui.Context) {}, Apply: func() {}, Open: func(row protocol.UsageRequest) { opened = row }})
	}, 360, 680)
	tt.SetLocale("en")
	tt.Frame()
	label := i18n.In("en", "trace_view") + ": " + want.Session.String() + " / " + time.UnixMilli(want.Position.TimestampMs).Local().Format("2006-01-02 15:04:05") + " #2"
	b, ok := tt.AccessByRole(ui.RoleButton, label)
	if !ok {
		t.Fatal(tt.Texts())
	}
	tt.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
	tt.Frame()
	if opened.Session != want.Session || opened.Turn != want.Turn || opened.RequestID != want.RequestID || opened.Attempt != want.Attempt {
		t.Fatal("row lost its target", opened)
	}
}

func TestGroupExpandsMeasuredTotalsAndOpensFilteredTraces(t *testing.T) {
	p95 := uint64(32000)
	id := protocol.ProviderID{4}
	name := "test-channel"
	g := protocol.UsageGroup{Key: protocol.UsageKey{Kind: "provider", Data: id}, Name: &name,
		Metrics:  protocol.UsageMetrics{Tokens: &protocol.ConversationUsage{Input: 50, Output: 14}, Cost: &protocol.UsageCost{UsdMicros: 100000}},
		Insights: &protocol.UsageInsights{Attempts: 2, CacheReadSamples: 2, CacheRead: 10, ElapsedUs: protocol.UsagePercentiles{Samples: 2, P95: &p95}}}
	s := State{Draft: Filters{Days: 7, Outcome: "failed"}}
	applied := false
	tt := ui.NewTester(func(c *ui.Context) { usageGroup(c, &s, g, func() { applied = true }) }, 360, 620)
	tt.SetLocale("en")
	tt.Frame()
	click := func(label string) {
		t.Helper()
		b, ok := tt.AccessByRole(ui.RoleButton, label)
		if !ok {
			t.Fatal("missing button", label, tt.Texts())
		}
		tt.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
		tt.Frame()
	}
	cost := i18n.In("en", "usage_cost") + ": $0.100000"
	if tt.HasText(cost) {
		t.Fatal("collapsed group leaked details")
	}
	click(name)
	for _, label := range []string{cost, i18n.In("en", "meter_input") + ": 50", i18n.In("en", "meter_cache_read") + ": 10", i18n.In("en", "usage_duration") + " P95: " + i18n.In("en", "usage_duration_ms", "value", "32.0")} {
		if !tt.HasText(label) {
			t.Fatal("missing group measurement", label, tt.Texts())
		}
	}
	click(i18n.In("en", "insights_index"))
	if !applied || s.Tab != 1 || s.Draft.Provider != id.String() || s.Draft.Outcome != "failed" || !s.Draft.ObservedOnly {
		t.Fatal("group did not constrain traces", s.Draft)
	}
}
