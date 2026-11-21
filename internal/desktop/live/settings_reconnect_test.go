package live

import (
	"context"
	"slices"
	"testing"
	"testing/synctest"
	"time"

	"github.com/ai-poet/agent-client/internal/client"
	"github.com/ai-poet/agent-client/internal/desktop/shell"
	"github.com/ai-poet/agent-client/internal/desktop/theme"
	"github.com/ai-poet/agent-client/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type settingsReconnectSource struct {
	conversationTestTransport
	node *settingsReconnectStream
}

type settingsReconnectStream struct {
	conversationTestSubscription
	lost chan struct{}
}

func (s *settingsReconnectStream) Next(ctx context.Context) (protocol.Update, error) {
	select {
	case update := <-s.updates:
		return update, nil
	case <-s.lost:
		return protocol.Update{}, &protocol.Fault{Code: protocol.ErrorCodeUnavailable, Message: "fixture disconnected"}
	case <-s.closed:
		return protocol.Update{}, context.Canceled
	case <-ctx.Done():
		return protocol.Update{}, ctx.Err()
	}
}

func (s *settingsReconnectSource) Subscribe(ctx context.Context, topic protocol.Topic) (link.Subscription, error) {
	if topic.Kind != "node" {
		return s.conversationTestTransport.Subscribe(ctx, topic)
	}
	stream := &settingsReconnectStream{
		conversationTestSubscription: conversationTestSubscription{updates: make(chan protocol.Update, 1), closed: make(chan struct{})},
		lost:                         make(chan struct{}),
	}
	stream.updates <- protocol.Update{Kind: "snapshot", Data: s.snapshot}
	s.node = stream
	return stream, nil
}

func newSettingsReconnectRuntime(t *testing.T, source *settingsReconnectSource) (*Runtime, func(), func() func()) {
	t.Helper()
	controller := client.New(source)
	if err := controller.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = controller.Close() })
	data := &viewmodel.Data{Workspace: &viewmodel.Workspace{}}
	for _, session := range source.snapshot.Sessions {
		data.Unassigned = append(data.Unassigned, &viewmodel.Conversation{ID: session.ID.String(), Node: source.Target().String()})
	}
	r := &Runtime{ctx: t.Context(), controller: controller, App: shell.NewLive(data)}
	r.workspaceSettings.sandboxKnown = true
	var retry func()
	// Small enough to keep MyGo's external raster workers out of synctest.
	tester := ui.NewTesterWith(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		r.mu.Lock()
		queue := r.queue
		r.queue = nil
		r.mu.Unlock()
		for _, finish := range queue {
			finish(c)
		}
		ui.ToastViewportBase(c, func(_ ui.Element, toasts []ui.Toast) {
			for _, toast := range toasts {
				if toast.OnAction != nil {
					retry = toast.OnAction
				}
			}
		})
	}, 1, 1, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	return r, func() {
		r.workers.Wait()
		tester.Frame()
	}, func() func() { return retry }
}

func TestWorkspaceHooksReconnect(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		snapshot := protocol.DefaultSnapshot()
		snapshot.Node = protocol.NodeID{1}
		project := protocol.NewProjectID()
		for i := range 2 {
			worktree, session := protocol.WorktreeID{byte(i + 1)}, protocol.SessionID{byte(i + 1)}
			snapshot.Worktrees = append(snapshot.Worktrees, protocol.Worktree{ID: worktree, Project: &project, Path: "/fixture/hooks"})
			snapshot.Sessions = append(snapshot.Sessions, protocol.Session{ID: session, Worktree: worktree, Project: &project})
		}
		revision := uint64(3)
		wrongWorktree, failRead := false, true
		var saveID protocol.RequestID
		var inspected []protocol.RequestID
		var kinds []string
		source := &settingsReconnectSource{conversationTestTransport: conversationTestTransport{snapshot: snapshot}}
		source.handle = func(request protocol.Request) (link.Response, bool) {
			kinds = append(kinds, request.Command.Kind)
			output := func(worktree protocol.WorktreeID, enabled bool) protocol.Output {
				return protocol.Output{Kind: "hook_settings", Data: protocol.HooksState{Worktree: worktree, Settings: protocol.HooksSettings{Revision: revision, Enabled: enabled}}}
			}
			switch request.Command.Kind {
			case "read_hooks":
				if failRead {
					return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeInternal, Message: "fixture read failure"}}, true
				}
				input, _ := detail[protocol.CommandReadHooksPayload](request.Command.Data)
				if wrongWorktree {
					input.Worktree = snapshot.Worktrees[1].ID
				}
				return link.Response{Output: output(input.Worktree, false)}, true
			case "save_hooks":
				input, _ := detail[protocol.CommandSaveHooksPayload](request.Command.Data)
				if input.Worktree != snapshot.Worktrees[0].ID || input.ExpectedRevision != revision || !input.Settings.Enabled {
					return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "wrong hook save scope, revision or draft"}}, true
				}
				saveID = request.ID
				return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "fixture completion lost"}}, true
			case "inspect_request":
				query, _ := detail[protocol.CommandInspectRequestPayload](request.Command.Data)
				inspected = append(inspected, query.ID)
				outcome := protocol.RequestOutcome{Kind: "admitted"}
				if len(inspected) == 2 {
					revision++
					saved := output(snapshot.Worktrees[0].ID, true)
					outcome = protocol.RequestOutcome{Kind: "completed", Data: protocol.Result[protocol.Output, protocol.Fault]{Ok: &saved}}
				}
				return link.Response{Output: protocol.Output{Kind: "request_outcome", Data: protocol.OutputRequestOutcomePayload{ID: query.ID, Outcome: outcome}}}, true
			}
			return link.Response{}, false
		}
		r, drain, retry := newSettingsReconnectRuntime(t, source)
		m := r.App.Settings
		selectSession := func(i int) {
			r.App.Sidebar.Selected = snapshot.Sessions[i].ID.String()
			r.workspaceSettingsFrame()
		}
		selectSession(0)
		drain()
		selectSession(0)
		drain()
		if m.SaveHooks != nil || !slices.Equal(kinds, []string{"read_hooks"}) {
			t.Fatalf("failed read left save available or retried per frame: %v", kinds)
		}
		failRead, wrongWorktree = false, true
		initial := r.controller.ObservationGeneration()
		close(source.node.lost)
		synctest.Wait()
		time.Sleep(time.Second)
		synctest.Wait()
		if r.controller.ObservationGeneration() == 0 || r.controller.ObservationGeneration() == initial {
			t.Fatal("fixture did not reconnect")
		}
		// The UI never rendered an offline frame.
		selectSession(0)
		drain()
		if m.SaveHooks != nil || m.Hooks.Revision != 0 || len(kinds) != 2 {
			t.Fatalf("same worktree was not reread after reconnect, or accepted another worktree: %v, %+v", kinds, m.Hooks)
		}
		wrongWorktree = false
		if err := r.controller.Refresh(t.Context()); err != nil {
			t.Fatal(err)
		}
		selectSession(0)
		r.workers.Wait() // hold completion in the queue until after switching worktrees
		selectSession(1)
		if m.SaveHooks != nil {
			t.Fatal("switched worktree became writable before its own read")
		}
		drain()
		first := r.workspaceHooksModel(snapshot.Worktrees[0].ID)
		if first.Revision != 0 || m.Hooks.Revision != 3 || m.SaveHooks == nil {
			t.Fatalf("stale callback was accepted or the new worktree was not loaded: %+v, %+v", first.Stored, m.Hooks)
		}
		staleSave := m.SaveHooks
		selectSession(0)
		staleSave()
		drain()
		if m.SaveHooks == nil || kinds[len(kinds)-1] != "read_hooks" {
			t.Fatalf("stale save changed the newly selected worktree: %v", kinds)
		}
		m.Hooks.Draft.Enabled = true
		m.SaveHooks()
		drain()
		again := retry()
		if m.SaveHooks != nil || !m.Hooks.Draft.Enabled || again == nil {
			t.Fatal("unresolved save lost the draft, allowed a fresh save, or did not offer recovery")
		}
		selectSession(1)
		again()
		drain()
		if len(inspected) != 1 {
			t.Fatalf("Retry inspected while another worktree was selected: %v", inspected)
		}
		selectSession(0)
		drain()
		again()
		drain()
		if saveID == (protocol.RequestID{}) || !slices.Equal(inspected, []protocol.RequestID{saveID, saveID}) || m.Hooks.Revision != 4 || m.SaveHooks == nil {
			t.Fatalf("Retry did not settle the original write: %v, %+v", kinds, m.Hooks)
		}
		saves := 0
		for _, kind := range kinds {
			if kind == "save_hooks" {
				saves++
			}
		}
		if saves != 1 {
			t.Fatalf("Retry performed a second write: %v", kinds)
		}
	})
}

func TestMediaSettingsReconnectPair(t *testing.T) {
	for _, failure := range []string{"assignments", "options", "malformed options"} {
		t.Run(failure, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				snapshot := protocol.DefaultSnapshot()
				snapshot.Node, snapshot.MediaSettings.Revision = protocol.NodeID{1}, 3
				read, listed := 0, 0
				reject := true
				source := &settingsReconnectSource{conversationTestTransport: conversationTestTransport{snapshot: snapshot}}
				source.handle = func(request protocol.Request) (link.Response, bool) {
					fault := link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeInternal, Message: "fixture read failure"}}
					switch request.Command.Kind {
					case "read_media_settings":
						read++
						if reject && failure == "assignments" {
							return fault, true
						}
						return link.Response{Output: protocol.Output{Kind: "media_settings", Data: snapshot.MediaSettings}}, true
					case "list_media_models":
						listed++
						if reject && failure == "options" {
							return fault, true
						}
						kind := "media_models"
						if reject && failure == "malformed options" {
							kind = "unrelated"
						}
						return link.Response{Output: protocol.Output{Kind: kind, Data: []protocol.MediaCandidate{{Provider: protocol.ProviderID{1}, Model: "vision"}}}}, true
					}
					return link.Response{}, false
				}
				r, drain, _ := newSettingsReconnectRuntime(t, source)
				follow := func() { r.followMedia(r.snapshot()); drain() }
				follow()
				follow()
				wantListed := 1
				if failure == "assignments" {
					wantListed = 0
				}
				m := r.App.Settings
				if read != 1 || listed != wantListed || m.Media.Revision != 0 || len(m.MediaOptions) != 0 || r.media.accepted != (mediaReadKey{}) {
					t.Fatalf("invalid pair was accepted or retried per frame: reads=%d lists=%d media=%+v", read, listed, m.Media)
				}
				reject = false
				if failure == "assignments" {
					close(source.node.lost)
					synctest.Wait()
					time.Sleep(time.Second)
					synctest.Wait()
				} else if err := r.controller.Refresh(t.Context()); err != nil {
					t.Fatal(err)
				}
				follow()
				follow()
				if read != 2 || listed != wantListed+1 || m.Media.Revision != 3 || len(m.MediaOptions) != 1 || r.media.accepted != r.media.wanted {
					t.Fatalf("same revision was not recovered: reads=%d lists=%d media=%+v", read, listed, m.Media)
				}
			})
		})
	}
}
