package engine

import (
	"context"
	"testing"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestSlowFirstRetryIsImmediatelyVisible(t *testing.T) {
	t.Parallel()
	m := &scriptedModel{}
	f := newFixture(t, m)
	clock := f.r.clock.(*manualClock)
	m.scripts = []script{
		func(ctx context.Context, call fantasy.Call, yield func(fantasy.StreamPart) bool) {
			// Simulate a 72-second slow upstream failure (e.g. HTTP/2 PROTOCOL_ERROR or 300s stall).
			clock.mu.Lock()
			clock.now = clock.now.Add(72 * time.Second)
			clock.mu.Unlock()
			yield(fantasy.StreamPart{Type: fantasy.StreamPartTypeError, Error: &model.Failure{Kind: model.Network, Message: "stream error: PROTOCOL_ERROR"}})
		},
		response(text("Recovered cleanly"), finish()),
	}
	f.start()
	turn := f.submit("Long request")

	select {
	case wait := <-clock.waits:
		var retries []protocol.ConversationRecoveryRetry
		for _, record := range f.state().Records {
			if record.Metadata.Execution != nil && record.Metadata.Execution.Retry != nil {
				retries = append(retries, *record.Metadata.Execution.Retry)
			}
		}
		if len(retries) != 1 {
			t.Fatalf("got %d retry records, want 1", len(retries))
		}
		if retries[0].Attempt != 1 || retries[0].Limit == nil || *retries[0].Limit != model.MainRetries {
			t.Fatalf("slow first retry stayed hidden or misnumbered: %+v", retries[0])
		}
		close(wait.advance)
	case <-f.ctx.Done():
		t.Fatal("timed out waiting for slow retry")
	}

	run := f.waitRun(turn, protocol.ConversationStatusCompleted)
	if run.Status != protocol.ConversationStatusCompleted {
		t.Fatalf("turn status = %s, want completed", run.Status)
	}
}
