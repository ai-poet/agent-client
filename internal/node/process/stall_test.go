package process

import (
	"context"
	"testing"
	"testing/synctest"
	"time"

	"github.com/ai-poet/agent-client/internal/protocol"
)

func TestStallMonitor(t *testing.T) {
	t.Parallel()
	type fixture struct {
		service *Service
		stalls  chan Stalled
		output  chan string
		launch  Launch
	}
	start := func(t *testing.T) fixture {
		value := fixture{stalls: make(chan Stalled, 8), output: make(chan string)}
		service, err := New(Options{Profile: t.TempDir(),
			Stalled: func(_ context.Context, stalled Stalled) error { value.stalls <- stalled; return nil },
			Run: func(ctx context.Context, _ Launch, emit func(protocol.ProcessOutputOutputStream, []byte), _ <-chan []byte) (protocol.ProcessCompletion, error) {
				for {
					select {
					case text, open := <-value.output:
						if !open {
							return protocol.ProcessCompletion{Outcome: protocol.ProcessOutcome{Kind: "exited", Data: int32(0)}}, nil
						}
						emit(protocol.ProcessOutputOutputStreamStdout, []byte(text))
					case <-ctx.Done():
						return protocol.ProcessCompletion{Outcome: protocol.ProcessOutcome{Kind: "cancelled"}}, nil
					}
				}
			}})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = service.Close() })
		value.service = service
		value.launch = Launch{ID: protocol.NewRequestID(), Session: protocol.NewSessionID(), Turn: protocol.NewTurnID(), Root: t.TempDir(), Command: "fixture", Access: protocol.ProcessAccessWorkspace, TimeoutMs: 1, FollowUp: true}
		return value
	}
	advance := func(d time.Duration) {
		time.Sleep(d)
		synctest.Wait()
	}
	stalled := func(t *testing.T, value fixture) Stalled {
		t.Helper()
		select {
		case report := <-value.stalls:
			return report
		default:
			t.Fatal("silent command was not reported")
			return Stalled{}
		}
	}
	run := func(name string, f func(*testing.T)) {
		t.Run(name, func(t *testing.T) { synctest.Test(t, f) })
	}
	run("only the first silent period of a follow-up command is reported", func(t *testing.T) {
		value := start(t)
		info, err := value.service.Start(value.launch)
		if err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		advance(10 * time.Minute)
		value.output <- "still working\n"
		synctest.Wait()
		advance(StallWindow - time.Second)
		if len(value.stalls) != 0 {
			t.Fatal("output did not restart the silent window")
		}
		advance(time.Second)
		first := stalled(t, value)
		if first.Snapshot.Info.ID != info.ID || first.Snapshot.Info.Session != info.Session || first.Snapshot.Info.Status.Kind != "running" ||
			first.Snapshot.Stdout.Text != "still working\n" || first.SilenceMs != uint64(StallWindow.Milliseconds()) ||
			first.ElapsedMs != uint64((10*time.Minute+StallWindow).Milliseconds()) {
			t.Fatal("unexpected first stall", first)
		}
		value.output <- "resumed\n"
		synctest.Wait()
		advance(2 * StallWindow)
		if len(value.stalls) != 0 {
			t.Fatal("a follow-up stall was reported again after output resumed")
		}
		if snapshot, err := value.service.Read(info.Session, info.ID); err != nil || snapshot.Info.Status.Kind != "running" {
			t.Fatal("a stall changed the command", snapshot, err)
		}
	})
	run("a finished command is never reported", func(t *testing.T) {
		value := start(t)
		info, err := value.service.Start(value.launch)
		if err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		close(value.output)
		if outcome, err := value.service.Wait(context.Background(), info.Session, info.ID, 30000, "missing"); err != nil || outcome != protocol.ProcessWaitExited {
			t.Fatalf("wait = %q, %v; want exit", outcome, err)
		}
		advance(2 * StallWindow)
		if err := value.service.Close(); err != nil {
			t.Fatal(err)
		}
		if len(value.stalls) != 0 {
			t.Fatal("finished command was reported as stalled")
		}
	})
	run("a promoted command is watched from its promotion", func(t *testing.T) {
		value := start(t)
		promotion := make(chan struct{})
		close(promotion)
		output, err := value.service.ExecutePromotable(context.Background(), value.launch, promotion)
		if err != nil || output.Kind != "command_started" {
			t.Fatal(output, err)
		}
		synctest.Wait()
		advance(StallWindow)
		if report := stalled(t, value); report.Snapshot.Info.ID != value.launch.ID {
			t.Fatal("unexpected promoted stall", report)
		}
	})
}

func TestCutOutputKeepsOutcome(t *testing.T) {
	t.Parallel()
	service, err := New(Options{Profile: t.TempDir(), Run: func(_ context.Context, _ Launch, emit func(protocol.ProcessOutputOutputStream, []byte), _ <-chan []byte) (protocol.ProcessCompletion, error) {
		emit(protocol.ProcessOutputOutputStreamStdout, []byte("partial"))
		// The runner stopped reading stdout before it closed.
		return protocol.ProcessCompletion{Outcome: protocol.ProcessOutcome{Kind: "exited", Data: int32(0)}, Stdout: protocol.ProcessCapture{Truncated: true}}, nil
	}})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = service.Close() })
	result, err := service.Execute(context.Background(), Launch{ID: protocol.NewRequestID(), Session: protocol.NewSessionID(), Turn: protocol.NewTurnID(), Root: t.TempDir(), Command: "fixture", Access: protocol.ProcessAccessWorkspace, TimeoutMs: 1000})
	if err != nil || result.Outcome.Kind != "exited" || result.Stdout.Text != "partial" || !result.Stdout.Truncated || result.Stderr.Truncated {
		t.Fatal("cut output changed the outcome or lost its mark", result, err)
	}
}
