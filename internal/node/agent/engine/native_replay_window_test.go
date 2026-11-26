package engine

import (
	"encoding/json"
	"net/http"
	"reflect"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNativeReplayAfterSecondCompaction(t *testing.T) {
	const secondWindow = `[
		{"type":"function_call","id":"fc_2","call_id":"call_2","status":"completed","name":"read_file","arguments":"{\"path\":\"notes.txt\"}"},
		{"type":"function_call_output","id":"out_2","call_id":"call_2","status":"completed","output":"notes content"},
		{"type":"compaction","id":"cmp_2","encrypted_content":"second-opaque-window"}
	]`
	f, x, binding, provider := nativeFixture(t, protocol.ConversationModelApiResponses, func(w http.ResponseWriter, req wireRequest) {
		switch {
		case !compacting(req):
			writeSSE(w, responsesText("Done"))
		case strings.Contains(req.raw, "Second request"):
			writeJSON(w, http.StatusOK, `{"id":"cmp2","object":"response.compaction","created_at":2,"output":`+secondWindow+`,"usage":{"input_tokens":90,"output_tokens":18,"total_tokens":108}}`)
		default:
			writeJSON(w, http.StatusOK, compactedWindow)
		}
	})
	if err := compactNow(t, f, x, binding); err != nil {
		t.Fatal(err)
	}
	addExchange(t, f, "Second request")
	if err := compactNow(t, f, x, binding); err != nil {
		t.Fatal(err)
	}
	sent := provider.all(func(wireRequest) bool { return true })
	if len(sent) != 2 || !compacting(sent[0]) || !compacting(sent[1]) || strings.Count(sent[1].raw, `"opaque-window"`) != 1 {
		t.Fatalf("want two native requests with one previous window in the second; got %v", sent)
	}
	next := sendMain(t, f, x, binding, provider)
	var want []map[string]any
	if err := json.Unmarshal([]byte(secondWindow), &want); err != nil {
		t.Fatal(err)
	}
	var replay []map[string]any
	input, _ := next.body["input"].([]any)
	for _, entry := range input {
		item, _ := entry.(map[string]any)
		switch item["type"] {
		case "function_call", "function_call_output", "compaction":
			replay = append(replay, item)
		}
	}
	if !reflect.DeepEqual(replay, want) || strings.Contains(next.raw, `"opaque-window"`) {
		t.Fatalf("next request did not carry only the latest tool pair and opaque checkpoint exactly once: %s", next.raw)
	}
}
