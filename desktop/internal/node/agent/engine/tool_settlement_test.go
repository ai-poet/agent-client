package engine

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/node/agent/canonical"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func TestPendingCallSettlementEvidence(t *testing.T) {
	request := func(id string, metadata json.RawMessage) canonical.Record {
		return canonical.Record{Kind: "message", Data: canonical.NewMessage(canonical.Assistant,
			canonical.Block{Kind: "tool_request", Data: &canonical.ToolRequest{ID: id, ToolMeta: metadata}})}
	}
	admission := func(id string, allowed bool) canonical.Record {
		patch, _ := json.Marshal(map[string]bool{canonical.ExecutableKey: allowed})
		return canonical.Record{Kind: "operation", Data: &canonical.Operation{Kind: "tool_meta", Data: &canonical.ToolMeta{Call: id, Patch: patch}}}
	}
	records := []canonical.Record{
		request("reused", nil),
		admission("reused", true),
		{Kind: "message", Data: canonical.NewMessage(canonical.User,
			canonical.Block{Kind: "tool_response", Data: &canonical.ToolResponse{ID: "reused"}})},
		request("reused", nil),
		admission("reused", false),
		request("admitted", nil),
		admission("admitted", true),
		request("external", json.RawMessage(`{"sailry.external_dispatch":true}`)),
	}
	admitted := map[string]bool{}
	calls := pendingCalls(records, admitted)
	var ids []string
	for _, call := range calls {
		ids = append(ids, call.ID)
	}
	if !reflect.DeepEqual(ids, []string{"reused", "admitted"}) {
		t.Fatalf("pending local calls: %v", ids)
	}
	if interruptedTool(admitted["reused"]).Failure.Code != protocol.ErrorCodeCancelled ||
		interruptedTool(admitted["admitted"]).Failure.Code != protocol.ErrorCodeOutcomeUnknown {
		t.Fatalf("settlement evidence leaked between calls: %v", admitted)
	}
}
