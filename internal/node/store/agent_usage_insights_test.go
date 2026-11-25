package store

import (
	"database/sql"
	"math"
	"reflect"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestUsageInsightsVisibleAttemptPages(t *testing.T) {
	t.Parallel()
	storage, node, session, turn, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	var reader protocol.User
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		reader, err = CreateUser(ctx, db, "Other reader")
		if err != nil {
			return err
		}
		obs := protocol.ConversationObservationRequest{ID: "round", Model: "fixture", Turn: turn, TimestampMs: now.UnixMilli()}
		for i := uint64(1); i <= 22; i++ {
			a := protocol.ConversationObservationAttempt{Transport: "http", Index: i, Outcome: protocol.ConversationObservationOutcomeFailed, FailureKind: ptr("rate_limit"), EndKind: ptr(protocol.ConversationObservationEndKindHTTPError), Status: ptr(uint16(429)), FirstTokenUs: ptr(i), ElapsedUs: ptr(100 * i)}
			if i == 22 {
				a.Outcome = protocol.ConversationObservationOutcomeSucceeded
				a.EndKind = ptr(protocol.ConversationObservationEndKindCompleted)
				a.FailureKind = nil
				a.Status = ptr(uint16(200))
				a.Usage = &protocol.ConversationUsage{Input: 10, Output: 5, CachedInput: 3, Reasoning: 1}
				a.CacheWrite = ptr(uint64(2))
				a.Verdict = &protocol.TraceVerdict{Code: "cache_partial", Severity: "warning", Summary: "diagnosis_cache_partial", LostTokens: 1098}
				a.CacheReported, a.ReasoningReported = true, true
			}
			obs.Attempts = append(obs.Attempts, a)
		}
		msg := canonical.NewMessage(canonical.Assistant, canonical.Text(0, "Answer"))
		event := canonical.NewRecord(turn, "message", &msg, now)
		event.Provenance.Author = "assistant"
		event.Metadata.Execution = &canonical.Execution{Observation: &obs}
		_, _, err = AppendCanonical(ctx, db, session.ID, event)
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	err = storage.Node.Read(ctx, func(db *sql.Conn) error {
		q := protocol.DefaultUsageQuery()
		q.StartMs, q.EndMs, q.Dimension, q.ObservedOnly = now.UnixMilli(), now.Add(time.Hour).UnixMilli(), protocol.UsageDimensionSession, true
		r, err := readAgentUsage(ctx, db, node, q, true)
		if err != nil {
			return err
		}
		m := r.Insights
		if m == nil || m.Attempts != 22 || m.Failed != 21 || m.Succeeded != 1 || m.Retries != 21 || m.EndKinds["completed"] != 1 || m.FailureKinds["rate_limit"] != 21 || m.CacheRead != 3 || m.CacheWrite != 2 || m.CacheInputTokens != 10 || m.FirstTokenUs.Samples != 22 || *m.FirstTokenUs.P50 != 11 || *m.FirstTokenUs.P95 != 21 || *m.ElapsedUs.P95 != 2100 {
			t.Fatalf("unexpected insights: %+v", m)
		}
		if len(r.Groups) != 1 || r.Groups[0].Key.Kind != "session" || !reflect.DeepEqual(r.Groups[0].Insights, m) || r.Totals.Responses != 1 || r.Totals.Tokens == nil || r.Totals.Tokens.Input != 10 || !r.Requests.HasMore || len(r.Requests.Items) != 20 {
			t.Fatalf("unexpected report: %+v", r)
		}
		q.Before = &r.Requests.Items[19].Position
		first := r.Requests.Items[0]
		if first.Verdict == nil || first.Verdict.Code != "cache_partial" || first.CacheRead == nil || *first.CacheRead != 3 || r.Requests.Items[1].CacheRead != nil {
			t.Fatalf("lost indexed diagnostic or invented cache usage: %+v", first)
		}
		next, err := readAgentUsage(ctx, db, node, q, true)
		if err != nil {
			return err
		}
		if next.Requests.HasMore || len(next.Requests.Items) != 2 || *next.Requests.Items[0].Attempt != 2 || !reflect.DeepEqual(next.Insights, m) {
			t.Fatalf("unexpected next page: %+v", next.Requests)
		}
		q.Before = nil
		q.Dimension, q.WithoutCredential = protocol.UsageDimensionCredential, true
		q.Statuses, q.FailureKinds = []uint16{429}, []string{"rate_limit"}
		failed, err := readAgentUsage(ctx, db, node, q, true)
		if err != nil {
			return err
		}
		if failed.Insights.Attempts != 21 || failed.Totals.Responses != 0 || len(failed.Groups) != 1 || failed.Groups[0].Key.Kind != "unassigned" {
			t.Fatalf("unexpected filtered report: %+v", failed)
		}
		hidden, err := readAgentUsage(WithUser(ctx, reader), db, node, q, true)
		if err != nil {
			return err
		}
		if hidden.Insights.Attempts != 0 || len(hidden.Requests.Items) != 0 || len(hidden.Groups) != 0 || len(hidden.Resources) != 0 {
			t.Fatal("another user's attempts leaked")
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestUsagePercentileHistogramBoundary(t *testing.T) {
	var d usageDistribution
	for i := 0; i < exactUsageSamples; i++ {
		d.add(ptr(uint64(30)))
	}
	p := d.finish()
	if p.Approximate || *p.P95 != 30 {
		t.Fatalf("exact: %+v", p)
	}
	d.add(ptr(uint64(math.MaxUint64)))
	p = d.finish()
	if !p.Approximate || p.Samples != exactUsageSamples+1 || *p.P95 != 31 || len(d.values) != 0 {
		t.Fatalf("histogram: %+v", p)
	}
}
