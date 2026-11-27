package terminal

import (
	"errors"
	"testing"
	"testing/synctest"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestWorkspaceTerminalRequiresSandbox(t *testing.T) {
	t.Parallel()
	service := New(t.Context(), protocol.NodeID{1}, nil)
	t.Cleanup(func() { _ = service.Close() })
	launch := Launch{
		ID: protocol.TerminalID{1}, Root: t.TempDir(), Access: protocol.ProcessAccessWorkspace,
		Command: []string{"fixture-not-started"}, Viewport: protocol.TerminalViewport{Columns: 80, Rows: 24},
	}
	_, err := service.Start(t.Context(), launch)
	var refusal *protocol.Fault
	if !errors.As(err, &refusal) || refusal.Code != protocol.ErrorCodeNotConfigured {
		t.Fatalf("workspace launch without a sandbox = %v", err)
	}
	denied := fault(protocol.ErrorCodeNotConfigured, "fixture sandbox is unavailable")
	launch.Sandbox = func(string, []string, []string) ([]string, []string, func(), error) {
		return nil, nil, nil, denied
	}
	if _, err := service.Start(t.Context(), launch); !errors.Is(err, denied) {
		t.Fatalf("workspace launch refused by sandbox = %v", err)
	}
}

func TestTerminalPollIntervalBackoff(t *testing.T) {
	if got := terminalPollInterval(0); got != terminalActiveInterval {
		t.Fatalf("active local interval = %v, want %v", got, terminalActiveInterval)
	}
	if got := terminalPollInterval(30); got != terminalWarmInterval {
		t.Fatalf("warm interval = %v, want %v", got, terminalWarmInterval)
	}
	if got := terminalPollInterval(45); got != terminalCoolInterval {
		t.Fatalf("cool interval = %v, want %v", got, terminalCoolInterval)
	}
	if got := terminalPollInterval(60); got != terminalLocalIdle {
		t.Fatalf("local idle interval = %v, want %v", got, terminalLocalIdle)
	}
}

func TestSubscriptionWakeResetsBackoff(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		service := New(t.Context(), protocol.NodeID{1}, nil)
		t.Cleanup(func() { _ = service.Close() })
		title := "initial"
		h := &handle{
			service: service,
			streams: map[*subscription]struct{}{},
			info: protocol.TerminalInfo{
				ID:     protocol.TerminalID{2},
				Status: protocol.TerminalStatus{Kind: "running"},
				Title:  &title,
			},
			viewport: protocol.TerminalViewport{Columns: 80, Rows: 24},
		}
		stream := &subscription{
			handle:     h,
			closed:     make(chan struct{}),
			finished:   make(chan struct{}),
			wake:       make(chan struct{}, 1),
			emptyTicks: 100, // already backed off to terminalLocalIdle
			last:       h.info,
		}
		h.streams[stream] = struct{}{}
		t.Cleanup(func() { _ = stream.Close() })

		ctx := t.Context()
		done := make(chan protocol.Update, 1)
		errs := make(chan error, 1)
		go func() {
			update, err := stream.Next(ctx)
			if err != nil {
				errs <- err
				return
			}
			done <- update
		}()

		synctest.Wait()
		updated := "updated"
		h.mu.Lock()
		h.info.Title = &updated
		h.wakeStreamsLocked()
		h.mu.Unlock()
		synctest.Wait()

		select {
		case err := <-errs:
			t.Fatalf("Next failed before wake: %v", err)
		case update := <-done:
			frame, ok := update.Data.(protocol.TerminalFrame)
			if !ok || frame.Info.Title == nil || *frame.Info.Title != updated {
				t.Fatalf("unexpected wake update: %+v", update)
			}
		default:
			t.Fatal("wake signal did not unblock backed-off subscription immediately")
		}
	})
}
