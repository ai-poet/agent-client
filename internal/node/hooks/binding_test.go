package hooks

import (
	"context"
	"errors"
	"log/slog"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestHooksCapturedAtAdmission(t *testing.T) {
	settings := protocol.DefaultHooksSettings()
	settings.Enabled = true
	settings.BeforeTool = &protocol.HooksScript{Command: "captured", Enabled: true, TimeoutMs: 30000}
	settings.Additional = []protocol.HooksEntry{{ID: "second", Phase: protocol.HooksPhaseBeforeTool, Script: protocol.HooksScript{Command: "second", Enabled: true, TimeoutMs: 100}}}
	var calls []string
	binding := New(Capture{Settings: settings, Admitted: true, Claim: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64) error {
		calls = append(calls, "claim")
		return nil
	}, Finish: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64, protocol.ProcessOutcome, error) error {
		calls = append(calls, "finish")
		return nil
	}, Launch: func(_ context.Context, command string, timeout uint64, env map[string]string) (protocol.ProcessCompletion, error) {
		calls = append(calls, command)
		if env["SAILRY_HOOK_TOOL"] != "read_file" {
			t.Fatal(env)
		}
		if command == "captured" && timeout != 30000 {
			t.Fatal(timeout)
		}
		return protocol.ProcessCompletion{Outcome: protocol.ProcessOutcome{Kind: "exited", Data: int32(0)}}, nil
	}})
	settings.BeforeTool.Command = "changed"
	settings.Additional[0].Script.Command = "changed too"
	if err := binding.Before(context.Background(), "read_file"); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(calls, []string{"claim", "captured", "second", "finish"}) {
		t.Fatal(calls)
	}
}

func TestUncertainHookClaimNeverLaunches(t *testing.T) {
	settings := protocol.DefaultHooksSettings()
	settings.Enabled = true
	settings.BeforeTool = &protocol.HooksScript{Command: "effect", Enabled: true, TimeoutMs: 10}
	launched := false
	binding := New(Capture{Settings: settings, Admitted: true, Claim: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64) error {
		return &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "already admitted"}
	}, Finish: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64, protocol.ProcessOutcome, error) error {
		return nil
	}, Launch: func(context.Context, string, uint64, map[string]string) (protocol.ProcessCompletion, error) {
		launched = true
		return protocol.ProcessCompletion{}, nil
	}})
	var fault *protocol.Fault
	if err := binding.Before(context.Background(), "write_file"); !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeOutcomeUnknown || launched {
		t.Fatal(err, launched)
	}
}

func TestHookReportsOnlyUnexpectedTerminalFailures(t *testing.T) {
	closeLog, err := logging.Install(filepath.Join(t.TempDir(), "profile"), "host")
	if err != nil {
		t.Fatal(err)
	}
	defer closeLog()
	var reports []logging.Fields
	logging.SetReport(func(level slog.Level, fields logging.Fields) {
		if level == slog.LevelError && fields["area"] == "hooks" {
			reports = append(reports, fields)
		}
	})
	defer logging.SetReport(nil)
	unknown := protocol.ErrorCodeOutcomeUnknown
	for _, test := range []struct {
		name     string
		phase    protocol.HooksPhase
		panics   bool
		claimed  error
		finished error
		outcome  protocol.ProcessOutcome
		launch   error
		code     protocol.ErrorCode
		report   bool
	}{
		{name: "before tool panic", panics: true, code: unknown, report: true},
		{name: "after tool panic is returned to turn settlement", phase: protocol.HooksPhaseAfterTool, panics: true, code: unknown},
		{name: "turn end receipt failure is returned to turn settlement", phase: protocol.HooksPhaseTurnEnd, finished: errors.New("private receipt detail"), code: unknown},
		{name: "unknown process outcome", outcome: protocol.ProcessOutcome{Kind: "unknown", Data: "private process detail"}, code: unknown, report: true},
		{name: "completion could not be recorded", finished: errors.New("private receipt detail"), code: unknown, report: true},
		{name: "duplicate claim is not replayed", claimed: &protocol.Fault{Code: unknown, Message: "already admitted"}, code: unknown},
		{name: "script veto", outcome: protocol.ProcessOutcome{Kind: "exited", Data: int32(1)}, code: protocol.ErrorCodePermissionDenied},
		{name: "active cancellation", outcome: protocol.ProcessOutcome{Kind: "cancelled"}, code: protocol.ErrorCodePermissionDenied},
	} {
		t.Run(test.name, func(t *testing.T) {
			reports = nil
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			script := &protocol.HooksScript{Command: "fixture", Enabled: true, TimeoutMs: 100}
			settings := protocol.HooksSettings{Enabled: true}
			switch test.phase {
			case protocol.HooksPhaseAfterTool:
				settings.AfterTool = script
			case protocol.HooksPhaseTurnEnd:
				settings.TurnEnd = script
			default:
				settings.BeforeTool = script
			}
			turn := protocol.NewTurnID()
			binding := New(Capture{Turn: turn, Settings: settings, Admitted: true,
				Claim: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64) error { return test.claimed },
				Finish: func(ctx context.Context, _ protocol.TurnID, _ protocol.HooksPhase, _ uint64, _ protocol.ProcessOutcome, _ error) error {
					if ctx.Err() != nil {
						t.Fatal("hook receipt must settle independently of cancellation")
					}
					return test.finished
				},
				Launch: func(context.Context, string, uint64, map[string]string) (protocol.ProcessCompletion, error) {
					if test.panics {
						panic("private panic detail")
					}
					outcome := test.outcome
					if outcome.Kind == "" {
						outcome = protocol.ProcessOutcome{Kind: "exited", Data: int32(0)}
					} else if outcome.Kind == "cancelled" {
						cancel()
					}
					return protocol.ProcessCompletion{Outcome: outcome}, test.launch
				},
			})
			var err error
			switch test.phase {
			case protocol.HooksPhaseAfterTool:
				binding.After(ctx, "read_file", true)
				err = binding.Complete(ctx, "completed")
			case protocol.HooksPhaseTurnEnd:
				err = binding.Complete(ctx, "completed")
			default:
				err = binding.Before(ctx, "read_file")
			}
			var fault *protocol.Fault
			if !errors.As(err, &fault) || fault.Code != test.code {
				t.Fatalf("hook fault = %v, want %s", err, test.code)
			}
			if !test.report {
				if len(reports) != 0 {
					t.Fatalf("unexpected terminal report: %v", reports)
				}
				return
			}
			if len(reports) != 1 || reports[0]["operation"] != string(protocol.HooksPhaseBeforeTool) || reports[0]["turn"] != turn.String() ||
				reports[0]["phase"] != logging.Failed || reports[0]["code"] != string(test.code) || reports[0]["exception"] != nil || reports[0]["detail"] != nil {
				t.Fatalf("terminal reports = %v", reports)
			}
		})
	}
}

func TestTurnEndHooks(t *testing.T) {
	settings := protocol.DefaultHooksSettings()
	settings.Enabled = true
	settings.AfterTool = &protocol.HooksScript{Command: "after", Enabled: true, TimeoutMs: 10}
	settings.TurnEnd = &protocol.HooksScript{Command: "end", Enabled: true, TimeoutMs: 10}
	type launch struct{ command, event, status, tool string }
	bind := func(stop context.Context, fail string) (*Binding, *[]launch) {
		var launches []launch
		binding := New(Capture{Settings: settings, Admitted: true, Stop: stop, Claim: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64) error {
			return nil
		}, Finish: func(context.Context, protocol.TurnID, protocol.HooksPhase, uint64, protocol.ProcessOutcome, error) error {
			return nil
		}, Launch: func(_ context.Context, command string, _ uint64, env map[string]string) (protocol.ProcessCompletion, error) {
			launches = append(launches, launch{command, env["SAILRY_HOOK_EVENT"], env["SAILRY_HOOK_STATUS"], env["SAILRY_HOOK_TOOL"]})
			code := int32(0)
			if command == fail {
				code = 1
			}
			return protocol.ProcessCompletion{Outcome: protocol.ProcessOutcome{Kind: "exited", Data: code}}, nil
		}})
		return binding, &launches
	}
	t.Run("a finished turn reports completed", func(t *testing.T) {
		binding, launches := bind(context.Background(), "")
		binding.After(context.Background(), "write_file", true)
		if err := binding.Complete(context.Background(), "completed"); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(*launches, []launch{{"after", "after_tool", "succeeded", "write_file"}, {"end", "turn_end", "completed", ""}}) {
			t.Fatal(*launches)
		}
	})
	t.Run("a failed turn reports failed", func(t *testing.T) {
		binding, launches := bind(context.Background(), "")
		if err := binding.Complete(context.Background(), "failed"); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(*launches, []launch{{"end", "turn_end", "failed", ""}}) {
			t.Fatal(*launches)
		}
	})
	t.Run("an After Tool failure fails the turn", func(t *testing.T) {
		binding, launches := bind(context.Background(), "after")
		binding.After(context.Background(), "write_file", false)
		var fault *protocol.Fault
		if err := binding.Complete(context.Background(), "completed"); !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeUnavailable {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(*launches, []launch{{"after", "after_tool", "failed", "write_file"}, {"end", "turn_end", "failed", ""}}) {
			t.Fatal(*launches)
		}
	})
	t.Run("a stopped turn runs no script", func(t *testing.T) {
		stop, cancel := context.WithCancel(context.Background())
		binding, launches := bind(stop, "")
		cancel()
		// The engine completes with a context detached from the stopped turn.
		if err := binding.Complete(context.WithoutCancel(stop), "failed"); err != nil {
			t.Fatal(err)
		}
		binding.After(context.WithoutCancel(stop), "write_file", true)
		if len(*launches) != 0 {
			t.Fatal(*launches)
		}
	})
	t.Run("a turn that was not admitted has no binding", func(t *testing.T) {
		if New(Capture{Settings: settings}) != nil {
			t.Fatal("hooks bound a turn that was not admitted")
		}
		var binding *Binding
		if binding.WrapsTools() || binding.Before(context.Background(), "write_file") != nil || binding.Complete(context.Background(), "completed") != nil {
			t.Fatal("an absent binding ran")
		}
	})
}
