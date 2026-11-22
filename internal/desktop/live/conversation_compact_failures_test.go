package live

import (
	"encoding/json"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/conversation"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestCompactionFaultKeepsEngineMessage(t *testing.T) {
	fault := protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "Compaction failed: the model returned an empty summary after retry.", Details: &protocol.FaultDetails{
		Kind: "model_request", Data: protocol.FaultDetailsModelRequestPayload{Reason: protocol.ModelFailureInvalidResponse},
	}}
	page := protocol.DefaultConversationPage()
	page.Runs = []protocol.ConversationRun{{Turn: protocol.TurnID{1}, Kind: "compaction", Status: "failed", Error: &fault}}
	turns, _, _, _ := Conversation(client.ConversationState{Connected: true, Snapshot: protocol.ConversationSnapshot{Page: page}})
	row := turns[0]
	if row.Compaction.State != conversation.CompactionFailed || row.Compaction.Cause != fault.Message || len(row.Reply.Blocks) != 1 ||
		row.Reply.Blocks[0].Label != "chat_compact_failed" || row.Reply.Blocks[0].Text != fault.Message {
		t.Fatalf("wrong title or detail: marker=%+v blocks=%+v", row.Compaction, row.Reply.Blocks)
	}
	fault.Message = "Invalid model response"
	if got := failureKey(fault); got != "chat_response_invalid" {
		t.Fatalf("unrelated model failure mislabeled: %s", got)
	}
}

func TestCompactionResourceExplainsMissingRunFault(t *testing.T) {
	for _, row := range []struct{ stage, reason, cause, want string }{
		{"request", "request_failed", "Model could not be opened", "Model could not be opened"},
		{"budget", "no_fitting_prefix", "Previous summary does not fit", "Previous summary does not fit"},
		{"request", "request_failed", "", "摘要模型请求失败"},
		{"budget", "no_fitting_prefix", "", "没有完整历史片段符合摘要预算"},
	} {
		page := protocol.DefaultConversationPage()
		id := protocol.TurnID{1}
		page.Runs = []protocol.ConversationRun{{Turn: id, Kind: "compaction", Status: "failed"}}
		body, err := json.Marshal(map[string]any{"type": "compaction_failure", "failure": protocol.ConversationCompactionFailure{
			Source: "manual", Stage: protocol.ConversationCompactionStage(row.stage), Reason: protocol.ConversationCompactionReason(row.reason), Cause: &row.cause,
		}})
		if err != nil {
			t.Fatal(err)
		}
		page.Entries = []protocol.ConversationEntry{{Turn: id, Author: "system", Parts: []protocol.ConversationPart{{Kind: "resource", Data: json.RawMessage(body)}}}}
		turns, _, _, _ := Conversation(client.ConversationState{Connected: true, Snapshot: protocol.ConversationSnapshot{Page: page}}, "zh-CN")
		if k, blocks := turns[0].Compaction, turns[0].Reply.Blocks; k.Cause != row.want || len(blocks) != 1 || blocks[0].Label != "chat_compact_failed" || blocks[0].Text != row.want {
			t.Fatalf("%s: marker=%+v blocks=%+v", row.reason, k, blocks)
		}
	}
}

func TestCompactLogMessage(t *testing.T) {
	for _, message := range []string{"Authorization: Bearer fixture", "X-API-Key: fixture", "https://fixture.invalid/?token=fixture"} {
		if got := compactLogMessage(message); got != "[redacted]" {
			t.Fatalf("unsafe diagnostic: %q", got)
		}
	}
	const safe = "Compaction failed: the summary reached the output token limit"
	if got := compactLogMessage(safe); got != safe {
		t.Fatalf("lost safe diagnostic: %q", got)
	}
}
