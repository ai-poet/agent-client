package engine

import (
	"net/http"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNativeCompactionStopsWithoutTranscript(t *testing.T) {
	for _, tc := range []struct {
		name   string
		status int
		body   string
		kind   model.FailureKind
		calls  int
	}{
		{"quota", 402, `{"error":{"code":"insufficient_balance","message":"Insufficient balance"}}`, model.Quota, 2},
		{"invalid key", 401, `{"error":{"type":"authentication_error","message":"Invalid key"}}`, model.Authentication, 2},
		{"unknown key is not an unknown feature", 400, `{"error":{"type":"invalid_api_key","message":"Unknown API key"}}`, model.Authentication, 2},
		{"rate limit", 429, `{"error":{"type":"rate_limit_error","message":"Rate limited"}}`, model.RateLimited, model.MainRetries + 1},
		{"overload", 529, `{"error":{"type":"overloaded_error","message":"Overloaded"}}`, model.Unavailable, model.MainRetries + 1},
		{"forbidden", 403, `{"error":{"type":"permission_error","message":"Forbidden"}}`, model.Authentication, model.MainRetries + 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			f, x, binding, provider := nativeFixture(t, protocol.ConversationModelApiAnthropic, func(w http.ResponseWriter, req wireRequest) {
				if compacting(req) {
					writeJSON(w, tc.status, tc.body)
				} else {
					writeSSE(w, anthropicText(nativeFixtureHandoff))
				}
			})
			clock := f.r.clock.(*manualClock)
			stop := make(chan struct{})
			defer close(stop)
			go func() {
				for {
					select {
					case wait := <-clock.waits:
						close(wait.advance)
					case <-stop:
						return
					}
				}
			}()
			err := compactNow(t, f, x, binding)
			failure := model.Classify(err)
			if failure == nil || failure.Kind != tc.kind || failure.Exhausted != (tc.calls == model.MainRetries+1) {
				t.Fatalf("native failure = %+v; want %s after %d attempts", failure, tc.kind, tc.calls)
			}
			if calls := len(provider.all(compacting)); calls != tc.calls {
				t.Errorf("native attempts = %d, want %d", calls, tc.calls)
			}
			if summaries := provider.all(func(req wireRequest) bool { return !compacting(req) }); len(summaries) != 0 {
				t.Fatalf("native failure started %d transcript requests", len(summaries))
			}
		})
	}
}
