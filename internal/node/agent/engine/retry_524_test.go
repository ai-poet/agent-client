package engine

import (
	"context"
	"fmt"
	"reflect"
	"strings"
	"sync/atomic"
	"testing"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestHTTP524RecoveryDoesNotReplayTools(t *testing.T) {
	const diagnostic = "origin timed out (HTTP 524)"
	failure := &fantasy.ProviderError{StatusCode: 524, Message: diagnostic}
	for _, recovers := range []bool{true, false} {
		t.Run(fmt.Sprintf("recovers_%t", recovers), func(t *testing.T) {
			var executed atomic.Int32
			probe := objectTool("probe", true, true, func(context.Context, canonical.ToolRequest) ToolResult {
				executed.Add(1)
				return ToolResult{Value: map[string]bool{"ok": true}}
			})
			m := &scriptedModel{scripts: []script{response(call("committed", "probe", `{}`), finish())}}
			failures := 2
			if recovers {
				failures = 1
			}
			for range failures {
				m.scripts = append(m.scripts, response(text("Discarded attempt"), call("discarded", "probe", `{}`), fantasy.StreamPart{
					Type:  fantasy.StreamPartTypeError,
					Error: failure,
				}))
			}
			if recovers {
				m.scripts = append(m.scripts, response(text("Final answer"), finish()))
			}
			f := newFixture(t, m, probe)
			f.start()
			turn := f.submit("Task")
			status := protocol.ConversationStatusFailed
			if recovers {
				status = protocol.ConversationStatusCompleted
			}
			select {
			case wait := <-f.r.clock.(*manualClock).waits:
				if executed.Load() != 1 {
					t.Fatalf("failed attempt executed tools or replayed the completed one: %d", executed.Load())
				}
				close(wait.advance)
			case <-f.ctx.Done():
				t.Fatal("the model transport failure did not enter bounded recovery")
			}
			run := f.waitRun(turn, status)
			if executed.Load() != 1 || m.count() != 3 {
				t.Fatalf("recovery repeated a completed step: tools=%d requests=%d", executed.Load(), m.count())
			}
			if !recovers && (run.Error == nil || run.Error.Message != diagnostic || run.Error.Details == nil || run.Error.Details.Kind != "model_recovery") {
				t.Fatalf("unresolved model failure was marked successful or lost its cause: %+v", run.Error)
			}
			m.mu.Lock()
			for _, request := range m.calls[2:] {
				if !reflect.DeepEqual(request.Prompt, m.calls[1].Prompt) {
					t.Error("recovery changed the request's completed tool history")
				}
			}
			m.mu.Unlock()
			requests, responses, retries, terminalErrors := 0, 0, 0, 0
			state := f.state()
			for _, record := range state.Records {
				message := record.Message()
				if message == nil {
					continue
				}
				if execution := record.Metadata.Execution; execution != nil && execution.Retry != nil {
					retries++
					if retry := execution.Retry; retry.Limit == nil || *retry.Limit != 1 || retry.Status == nil || int(*retry.Status) != failure.StatusCode || retry.Code == nil || *retry.Code != string(model.Unavailable) || retry.Error == nil || *retry.Error != diagnostic {
						t.Fatalf("retry lost the provider error or actual HTTP status: %+v", retry)
					}
					if message.Usage != nil && !reflect.DeepEqual(*message.Usage, canonical.Usage{}) {
						t.Fatalf("settlement recovery invented usage: %+v", message.Usage)
					}
					if execution.Observation == nil {
						t.Fatal("recovery lost its failed attempt observation")
					}
					attempts := execution.Observation.Attempts
					if len(attempts) != 1 || attempts[0].Outcome != protocol.ConversationObservationOutcomeFailed || attempts[0].Usage != nil {
						t.Fatalf("the failed attempt became a success or known usage: %+v", attempts)
					}
				}
				if message.Error != nil {
					terminalErrors++
					if recovers || message.Error.Code == nil || *message.Error.Code != string(model.Unavailable) || !message.Error.RetryExhausted || message.Error.Message != diagnostic {
						t.Fatalf("terminal upstream failure was hidden: %+v", message.Error)
					}
				}
				for _, block := range message.Content {
					switch value := block.Data.(type) {
					case *canonical.ToolRequest:
						requests++
						if value.ID != "committed" {
							t.Fatal("an unsuccessful stream persisted a tool request")
						}
					case *canonical.ToolResponse:
						responses++
					case *canonical.TextContent:
						if recovers && strings.Contains(value.Text, "Discarded") {
							t.Fatal("recovery kept discarded provisional text")
						}
					}
				}
			}
			wantErrors := 1
			if recovers {
				wantErrors = 0
				mustHaveText(t, state.Records, "Final answer")
			}
			if requests != 1 || responses != 1 || retries != 1 || terminalErrors != wantErrors {
				t.Fatalf("canonical recovery changed effects or terminal status: requests=%d responses=%d retries=%d errors=%d", requests, responses, retries, terminalErrors)
			}
		})
	}
}
