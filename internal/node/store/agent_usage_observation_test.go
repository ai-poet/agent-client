package store

import (
	"database/sql"
	"reflect"
	"slices"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestUsageObservationProjectionAndFilters(t *testing.T) {
	t.Parallel()
	storage, node, session, turn, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	credential := protocol.NewCredentialID()
	outcome := protocol.ConversationObservationOutcomeFailed
	end := protocol.ConversationObservationEndKindStreamError
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		for i := range 3 {
			message := canonical.NewMessage(canonical.Assistant, canonical.Text(0, "Answer"))
			message.Usage = &canonical.Usage{Input: ptr(uint64(20)), Output: ptr(uint64(4))}
			event := canonical.NewRecord(turn, "message", &message, now.Add(time.Duration(i)*time.Second))
			event.Provenance.Author = "assistant"
			if i == 1 {
				message.Usage.CacheWrite = ptr(uint64(0))
				attempt := protocol.ConversationObservationAttempt{Index: 2, Transport: "http", Outcome: outcome, Status: ptr(uint16(200)), EndKind: &end, GatewayRequestID: ptr("fixture-id"), RetryReason: ptr("invalid_response"), CacheWrite: message.Usage.CacheWrite}
				event.Metadata.Execution = &canonical.Execution{
					Source:      &canonical.ModelSource{ID: protocol.NewProviderID(), Model: "fixture", API: protocol.ConversationModelApiChatCompletions, Authentication: protocol.AuthenticationApiKey, Credential: &protocol.CredentialRef{Node: node, ID: credential}},
					Observation: &protocol.ConversationObservationRequest{ID: "round", Model: "fixture", Turn: turn, TimestampMs: now.UnixMilli(), Attempts: []protocol.ConversationObservationAttempt{attempt}},
				}
			}
			if i == 2 {
				message.Usage = nil
				attempt := protocol.ConversationObservationAttempt{Index: 1, Transport: "http", Outcome: outcome}
				event.Metadata.Execution = &canonical.Execution{Observation: &protocol.ConversationObservationRequest{ID: "unbilled", Turn: turn, Model: "fixture", TimestampMs: now.UnixMilli(), Attempts: []protocol.ConversationObservationAttempt{attempt}}}
			}
			if _, _, err := AppendCanonical(ctx, db, session.ID, event); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	err = storage.Node.Read(ctx, func(db *sql.Conn) error {
		query := protocol.DefaultUsageQuery()
		query.StartMs, query.EndMs, query.Dimension = now.UnixMilli(), now.Add(time.Hour).UnixMilli(), protocol.UsageDimensionModel
		output, err := ReadAgentCommand(ctx, db, node, protocol.Command{Kind: "read_usage_v2", Data: &query})
		if err != nil {
			return err
		}
		report := output.Data.(protocol.UsageReport)
		if len(report.Requests.Items) != 3 || report.Totals.Responses != 2 {
			t.Fatalf("unfiltered report: %+v", report.Requests)
		}
		unbilled := report.Requests.Items[0]
		body, err := encode(unbilled)
		if err != nil {
			return err
		}
		var decoded protocol.UsageRequest
		if err = protocol.DecodeAs(protocol.PeerData, body, &decoded); err != nil {
			return err
		}
		if decoded.Tokens != nil {
			t.Fatal("wire round trip invented usage")
		}
		if unbilled.Tokens != nil || unbilled.UsdMicros != nil || unbilled.Outcome == nil || *unbilled.Outcome != outcome {
			t.Fatalf("unbilled failure: %+v", unbilled)
		}
		legacyOutput, err := ReadAgentCommand(ctx, db, node, protocol.Command{Kind: "read_usage", Data: &query})
		if err != nil {
			return err
		}
		legacy := legacyOutput.Data.(protocol.UsageReport)
		billingGroups := slices.Clone(report.Groups)
		for i := range billingGroups {
			if billingGroups[i].Insights == nil {
				t.Fatal("missing group insights")
			}
			billingGroups[i].Insights = nil
		}
		if len(legacy.Requests.Items) != 2 || legacy.Requests.Items[0].Tokens == nil || !reflect.DeepEqual(report.Totals, legacy.Totals) || !reflect.DeepEqual(billingGroups, legacy.Groups) || !reflect.DeepEqual(report.Days, legacy.Days) || !reflect.DeepEqual(report.Recent, legacy.Recent) {
			t.Fatal("v2 observations changed legacy usage totals or billing rows")
		}
		old := report.Requests.Items[2]
		if old.Outcome != nil || old.EndKind != nil || old.CacheWrite != nil || old.RetryReason != nil || old.GatewayRequestID != nil {
			t.Fatalf("invented legacy observation: %+v", old)
		}
		query.Outcomes, query.EndKinds = []protocol.ConversationObservationOutcome{outcome}, []protocol.ConversationObservationEndKind{end}
		query.Sessions, query.Credentials = []protocol.SessionID{session.ID}, []protocol.CredentialID{credential}
		report, err = readAgentUsage(ctx, db, node, query, true)
		if err != nil {
			return err
		}
		if len(report.Requests.Items) != 1 || report.Totals.Responses != 1 || report.Totals.Tokens.Output != 4 {
			t.Fatalf("filtered totals: %+v", report.Totals)
		}
		got := report.Requests.Items[0]
		if got.Outcome == nil || *got.Outcome != outcome || got.EndKind == nil || *got.EndKind != end || got.Attempt == nil || *got.Attempt != 2 || got.Status == nil || *got.Status != 200 || got.Credential == nil || *got.Credential != credential || got.RequestID == nil || *got.RequestID != "round" || got.CacheWrite == nil || *got.CacheWrite != 0 || got.RetryReason == nil || *got.RetryReason != "invalid_response" || got.GatewayRequestID == nil || *got.GatewayRequestID != "fixture-id" {
			t.Fatalf("observed request: %+v", got)
		}
		for _, mismatch := range []func(*protocol.UsageQuery){
			func(q *protocol.UsageQuery) { q.Sessions = []protocol.SessionID{protocol.NewSessionID()} },
			func(q *protocol.UsageQuery) { q.Credentials = []protocol.CredentialID{protocol.NewCredentialID()} },
			func(q *protocol.UsageQuery) {
				q.Outcomes = []protocol.ConversationObservationOutcome{protocol.ConversationObservationOutcomeSucceeded}
			},
			func(q *protocol.UsageQuery) {
				q.EndKinds = []protocol.ConversationObservationEndKind{protocol.ConversationObservationEndKindCompleted}
			},
		} {
			q := query
			mismatch(&q)
			empty, err := readAgentUsage(ctx, db, node, q, true)
			if err != nil {
				return err
			}
			if len(empty.Requests.Items) != 0 || empty.Totals.Responses != 0 {
				t.Fatalf("mismatching filter selected requests: %+v", q)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
