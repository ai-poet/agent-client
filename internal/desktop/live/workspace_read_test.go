package live

import (
	"bytes"
	"log/slog"
	"slices"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestWorkspaceReadFailureClassification(t *testing.T) {
	fault := func(code protocol.ErrorCode, message string) error {
		return &link.FaultError{Fault: protocol.Fault{Code: code, Message: message}}
	}
	cases := []struct {
		key  string
		err  error
		want string
	}{
		{"file/main.go", fault(protocol.ErrorCodeNotFound, "missing"), ""},
		{"directory/src", fault(protocol.ErrorCodeNotFound, "missing"), ""},
		{"git-status", fault(protocol.ErrorCodeNotFound, "missing"), "workspace_action_failed"},
		{"git-status", fault(protocol.ErrorCodeBusy, "Git read deadline exceeded"), "live_read_busy"},
		{"file/main.go", fault(protocol.ErrorCodeBusy, "file read deadline exceeded"), "live_read_busy"},
		{"file/secret.txt", fault(protocol.ErrorCodePermissionDenied, "denied"), "workspace_read_permission_failed"},
		{"directory/", fault(protocol.ErrorCodePermissionDenied, "denied"), "workspace_read_permission_failed"},
		{"git-status", fault(protocol.ErrorCodePermissionDenied, "denied"), "workspace_read_permission_failed"},
		{"file/main.go", fault(protocol.ErrorCodeUnavailable, "resource host is offline"), "resource_offline_action"},
		{"file/main.go", fault(protocol.ErrorCodeUnavailable, "Link connection was lost"), "workspace_action_failed"},
		{"git-status", fault(protocol.ErrorCodeInvalidRequest, "bad repo"), "workspace_action_failed"},
		{"git-status", fault(protocol.ErrorCodeNotConfigured, "git missing"), "workspace_action_failed"},
	}
	for _, tc := range cases {
		got := workspaceReadFailureKey(tc.key, tc.err)
		if got != tc.want {
			t.Errorf("workspaceReadFailureKey(%q, %v) = %q, want %q", tc.key, tc.err, got, tc.want)
		}
		if got != "" {
			for _, lang := range i18n.Languages {
				if !slices.Contains(i18n.Keys(lang), got) {
					t.Errorf("%s catalog missing key %q", lang, got)
				}
			}
		}
	}
}

func TestWorkspaceReadBackoffToastAndLogging(t *testing.T) {
	newHarness := func(t *testing.T, respond func(protocol.Request) link.Response) (*workspaceProject, *terminalTabsTestNode, func(func(*ui.Context)), func() []ui.Toast) {
		t.Helper()
		root, source := terminalTabsFixture(t)
		root.initWorkspace()
		project := protocol.ProjectID{1}
		tree := protocol.Worktree{ID: protocol.WorktreeID{1}, Project: &project, Path: "/Users/alice/private-repo", Main: true}
		p := root.workspaceFor(tree)
		source.respond = respond

		var (
			shown []ui.Toast
			act   func(*ui.Context)
		)
		tester := ui.NewTester(func(c *ui.Context) {
			if act != nil {
				act(c)
				act = nil
			}
			root.mu.Lock()
			queue := root.queue
			root.queue = nil
			root.mu.Unlock()
			for _, post := range queue {
				post(c)
			}
			ui.ToastViewportBase(c, func(_ ui.Element, toasts []ui.Toast) {
				shown = append(shown[:0], toasts...)
			})
		}, 640, 400)
		tester.SetLocale("en")
		run := func(action func(*ui.Context)) {
			act = action
			tester.Frame()
			synctest.Wait()
			tester.Frame()
		}
		return p, source, run, func() []ui.Toast { return append([]ui.Toast(nil), shown...) }
	}

	t.Run("not_found suppresses toast for file and directory but keeps toast for other keys", func(t *testing.T) {
		synctest.Test(t, func(t *testing.T) {
			var logged bytes.Buffer
			previous := slog.Default()
			slog.SetDefault(slog.New(slog.NewTextHandler(&logged, &slog.HandlerOptions{Level: slog.LevelDebug})))
			defer slog.SetDefault(previous)

			p, _, run, toasts := newHarness(t, func(protocol.Request) link.Response {
				return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeNotFound, Message: "missing /Users/alice/private-repo/secret-content"}}
			})
			run(func(*ui.Context) { p.loadFile("gone.go") })
			run(func(*ui.Context) { p.readDirectory("missing-dir", true) })
			if got := toasts(); len(got) != 0 {
				t.Fatalf("file/directory not_found emitted toasts: %+v", got)
			}
			if !p.entryIsMissing("gone.go") || !p.entryIsMissing("missing-dir") {
				t.Fatalf("missing entries were not recorded: %v", p.missingEntries)
			}
			run(func(*ui.Context) { p.readGit() })
			got := toasts()
			if len(got) != 1 || got[0].Title != i18n.In("en", "workspace_action_failed") {
				t.Fatalf("git-status not_found toasts = %+v", got)
			}
			logText := logged.String()
			for _, want := range []string{`msg="workspace read failed"`, `key=file/gone.go`, `command=read_file`, `code=not_found`, `key=directory/missing-dir`, `command=list_directory`, `key=git-status`, `command=inspect_git`} {
				if !strings.Contains(logText, want) {
					t.Errorf("log missing %s:\n%s", want, logText)
				}
			}
			if strings.Contains(logText, "secret-content") || strings.Contains(logText, "/Users/alice") {
				t.Errorf("log leaked fault message or host path:\n%s", logText)
			}
		})
	})

	t.Run("error codes map to expected toasts", func(t *testing.T) {
		for _, tc := range []struct {
			name    string
			fault   protocol.Fault
			wantKey string
		}{
			{"busy", protocol.Fault{Code: protocol.ErrorCodeBusy, Message: "timeout"}, "live_read_busy"},
			{"permission denied", protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "eacces"}, "workspace_read_permission_failed"},
			{"offline host", protocol.Fault{Code: protocol.ErrorCodeUnavailable, Message: "resource host is offline"}, "resource_offline_action"},
			{"conflict", protocol.Fault{Code: protocol.ErrorCodeConflict, Message: "changed"}, "workspace_changed"},
			{"outcome unknown", protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "unknown"}, "workspace_action_unknown"},
			{"fallback", protocol.Fault{Code: protocol.ErrorCodeInternal, Message: "boom"}, "workspace_action_failed"},
		} {
			t.Run(tc.name, func(t *testing.T) {
				synctest.Test(t, func(t *testing.T) {
					p, _, run, toasts := newHarness(t, func(protocol.Request) link.Response {
						f := tc.fault
						return link.Response{Fault: &f}
					})
					run(func(*ui.Context) { p.readGit() })
					got := toasts()
					want := i18n.In("en", tc.wantKey)
					if len(got) != 1 || got[0].Title != want {
						t.Fatalf("toasts = %+v, want 1 toast %q", got, want)
					}
				})
			})
		}
	})

	t.Run("backoff throttles repeated retries and deduplicates toasts until success clears state", func(t *testing.T) {
		synctest.Test(t, func(t *testing.T) {
			shouldFail := true
			p, source, run, toasts := newHarness(t, func(request protocol.Request) link.Response {
				if shouldFail {
					return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "denied"}}
				}
				switch request.Command.Kind {
				case "inspect_git":
					return link.Response{Output: protocol.Output{Kind: "git_status", Data: protocol.GitStatus{Kind: protocol.RepositoryKindDirectory}}}
				case "list_directory":
					return link.Response{Output: protocol.Output{Kind: "directory", Data: protocol.Directory{}}}
				}
				return link.Response{Output: protocol.Output{Kind: "ok"}}
			})

			run(func(*ui.Context) { p.readGit() })
			if got := len(source.sent("inspect_git")); got != 1 {
				t.Fatalf("initial inspect_git count = %d, want 1", got)
			}
			if !p.failedReads["git-status"] || p.failedAt["git-status"].IsZero() {
				t.Fatalf("failure state not recorded: failedReads=%v failedAt=%v", p.failedReads, p.failedAt)
			}

			// Within the 5s backoff window, readGit clears failedReads but failedAt suppresses resending.
			time.Sleep(workspaceReadBackoff - time.Second)
			run(func(*ui.Context) { p.readGit() })
			if got := len(source.sent("inspect_git")); got != 1 {
				t.Fatalf("inspect_git re-sent during backoff: count = %d, want 1", got)
			}

			// Directory refresh within backoff does not resend or stick in Loading state.
			run(func(*ui.Context) { p.refreshFiles() })
			if got := len(source.sent("list_directory")); got != 1 {
				t.Fatalf("initial list_directory count = %d, want 1", got)
			}
			run(func(*ui.Context) { p.refreshFiles() })
			if got := len(source.sent("list_directory")); got != 1 {
				t.Fatalf("list_directory re-sent during backoff: count = %d, want 1", got)
			}
			if state := p.directoryStates[""]; state.Loading || !state.Failed {
				t.Fatalf("backed-off directory state = %+v, want Loading=false Failed=true", state)
			}

			// Identical permission_denied toast from git-status and list_directory collapses into one.
			if got := toasts(); len(got) != 1 || got[0].Title != i18n.In("en", "workspace_read_permission_failed") {
				t.Fatalf("repeated same-copy failures stacked toasts: %+v", got)
			}

			// Once the backoff window elapses, the next read retries; success clears failedReads and failedAt.
			shouldFail = false
			time.Sleep(workspaceReadBackoff)
			run(func(*ui.Context) { p.readGit() })
			if got := len(source.sent("inspect_git")); got != 2 {
				t.Fatalf("inspect_git did not retry after backoff: count = %d, want 2", got)
			}
			if p.failedReads["git-status"] || !p.failedAt["git-status"].IsZero() {
				t.Fatalf("successful read did not clear failure state: failedReads=%v failedAt=%v", p.failedReads, p.failedAt)
			}
		})
	})

	t.Run("a persistent failure is reported once across watch retries", func(t *testing.T) {
		synctest.Test(t, func(t *testing.T) {
			shouldFail := true
			p, source, run, toasts := newHarness(t, func(request protocol.Request) link.Response {
				if shouldFail {
					return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeInternal, Message: "boom"}}
				}
				return link.Response{Output: protocol.Output{Kind: "git_status", Data: protocol.GitStatus{Kind: protocol.RepositoryKindDirectory}}}
			})
			failed := i18n.In("en", "workspace_action_failed")
			run(func(*ui.Context) { p.readGit() })
			if got := toasts(); len(got) != 1 || got[0].Title != failed {
				t.Fatalf("first failure toasts = %+v", got)
			}
			// The toast has expired and the backoff has elapsed: a watch refresh
			// retries the read, and the unchanged cause stays quiet.
			expire := func() {
				time.Sleep(workspaceReadBackoff + 10*time.Second)
				run(func(*ui.Context) {})
				if got := toasts(); len(got) != 0 {
					t.Fatalf("toast did not expire: %+v", got)
				}
			}
			for retry := range 3 {
				expire()
				run(func(*ui.Context) { p.readGit() })
				if got := len(source.sent("inspect_git")); got != retry+2 {
					t.Fatalf("watch refresh %d sent inspect_git %d times", retry+1, got)
				}
				if got := toasts(); len(got) != 0 {
					t.Fatalf("watch refresh %d repeated the toast: %+v", retry+1, got)
				}
			}
			// An explicit refresh reports the failure again.
			expire()
			run(func(*ui.Context) { p.refresh() })
			if got := toasts(); len(got) != 1 || got[0].Title != failed {
				t.Fatalf("explicit refresh toasts = %+v", got)
			}
			// A success ends the streak; the next failure is new.
			expire()
			shouldFail = false
			run(func(*ui.Context) { p.readGit() })
			shouldFail = true
			expire()
			run(func(*ui.Context) { p.readGit() })
			if got := toasts(); len(got) != 1 || got[0].Title != failed {
				t.Fatalf("failure after success toasts = %+v", got)
			}
		})
	})
}
