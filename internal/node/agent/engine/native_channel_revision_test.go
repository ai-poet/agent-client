package engine

import (
	"net/http"
	"sync/atomic"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNativeCompactionAfterChannelRevision(t *testing.T) {
	var supported atomic.Bool
	f, x, binding, provider := nativeFixture(t, protocol.ConversationModelApiResponses, func(w http.ResponseWriter, req wireRequest) {
		switch {
		case !compacting(req):
			writeSSE(w, responsesText(nativeFixtureHandoff))
		case supported.Load():
			writeJSON(w, http.StatusOK, compactedWindow)
		default:
			writeJSON(w, http.StatusNotFound, `{"error":{"type":"invalid_request_error","message":"Not found"}}`)
		}
	})
	if err := compactNow(t, f, x, binding); err != nil {
		t.Fatal(err)
	}
	supported.Store(true)
	addExchange(t, f, "Same revision")
	if err := compactNow(t, f, x, binding); err != nil {
		t.Fatal(err)
	}
	if calls := len(provider.all(compacting)); calls != 1 {
		t.Fatalf("before edit: %d native requests, want one rejection", calls)
	}
	binding.Provider.Revision++
	addExchange(t, f, "Edited revision")
	if err := compactNow(t, f, x, binding); err != nil {
		t.Fatal(err)
	}
	_, checkpoint := latestCheckpoint(t, f)
	if calls := len(provider.all(compacting)); calls != 2 || checkpoint.Native == nil {
		t.Fatalf("after edit: %d native requests, native checkpoint = %v", calls, checkpoint.Native != nil)
	}
}
