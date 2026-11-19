package live

import (
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/shell"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func TestWorkspaceHookSettings(t *testing.T) {
	t.Run("terminal settings projection", checkTerminalSettingsHelpers)
	t.Run("backup settings projection", checkBackupSettingsHelpers)

	t.Run("knowledge collections projection", checkKnowledgeViews)

	t.Run("system settings projection", checkSystemHelpers)
	t.Run("feature usage", testFeatureUsage)
	t.Run("terminal tabs prune", testTerminalTabsPrune)
	t.Run("terminal tabs create", testTerminalTabsCreate)
	t.Run("terminal tabs close", testTerminalTabsClose)
	t.Run("terminal tabs close pending", testTerminalTabsClosePending)

	t.Run("browser address", testBrowserAddress)
	t.Run("browser capture", testBrowserCapture)
	t.Run("browser call deadline", testBrowserDeadline)
	t.Run("project hooks stay remote while sandbox remains local", func(t *testing.T) {
		owners := make([]*Runtime, 2)
		transports := make([]*conversationTestTransport, 2)
		data := &viewmodel.Data{Workspace: &viewmodel.Workspace{}}
		app := shell.NewLive(data)
		for index := range owners {
			node, project, worktree, session := protocol.NodeID{byte(index + 1)}, protocol.NewProjectID(), protocol.NewWorktreeID(), protocol.NewSessionID()
			snapshot := protocol.DefaultSnapshot()
			snapshot.Node = node
			snapshot.SandboxSettingsRevision = uint64(index + 3)
			snapshot.Worktrees = []protocol.Worktree{{ID: worktree, Project: &project, Path: "/fixture/owner"}}
			snapshot.Sessions = []protocol.Session{{ID: session, Project: &project, Worktree: worktree}}
			transports[index] = &conversationTestTransport{snapshot: snapshot}
			transports[index].handle = func(request protocol.Request) (link.Response, bool) {
				switch request.Command.Kind {
				case "read_hooks":
					input, _ := detail[protocol.CommandReadHooksPayload](request.Command.Data)
					return link.Response{Output: protocol.Output{Kind: "hook_settings", Data: protocol.HooksState{Worktree: input.Worktree, Settings: protocol.HooksSettings{Revision: uint64(index + 7)}, Sandboxed: true}}}, true
				case "save_hooks":
					input, _ := detail[protocol.CommandSaveHooksPayload](request.Command.Data)
					input.Settings.Revision++
					return link.Response{Output: protocol.Output{Kind: "hook_settings", Data: protocol.HooksState{Worktree: input.Worktree, Settings: input.Settings, Sandboxed: true}}}, true
				case "save_sandbox_settings":
					input, _ := detail[protocol.SandboxSettings](request.Command.Data)
					input.Revision++
					return link.Response{Output: protocol.Output{Kind: "sandbox_settings", Data: input}}, true
				}
				return link.Response{}, false
			}
			controller := client.New(transports[index])
			if err := controller.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			defer controller.Close()
			owners[index] = &Runtime{
				ctx: t.Context(), controller: controller, App: app,
				busy: map[string]bool{}, uncertain: map[string]protocol.Request{},
				workspaceSettings: workspaceSettingsState{
					worktree: worktree, sandboxKnown: true, sandboxRevision: snapshot.SandboxSettingsRevision,
					sandboxModel: settings.Stored[bool]{Revision: uint64(index + 3)},
				},
			}
			data.Projects = append(data.Projects, &viewmodel.Project{ID: project.String(), Host: node.String(), Conversations: []*viewmodel.Conversation{{ID: session.String(), Node: node.String()}}})
		}
		local, remote := owners[0], owners[1]
		remote.parent = local
		local.bindings = map[string]*Runtime{remote.controller.Target().String(): remote}
		// Initial binding captures the sandbox state already loaded in the window.
		app.Settings.Sandbox = local.workspaceSettings.sandboxModel
		local.bindWorkspaceSettings()
		settle := settingsQueueTester(local)
		localSession, remoteSession := transports[0].snapshot.Sessions[0].ID.String(), transports[1].snapshot.Sessions[0].ID.String()
		// Project selection owns hooks independently of the monitored device.
		localHost, remoteHost := local.controller.Target().String(), remote.controller.Target().String()
		app.Host = localHost
		local.OpenSession(remoteSession)
		local.workspaceSettingsFrame()
		settle(remote)
		if label, ok := app.Settings.HooksContext(); !ok || !strings.Contains(label, remoteHost) || app.Settings.Hooks.Revision != 8 || app.Settings.Sandbox.Revision != 3 {
			t.Fatal("remote project hooks or local sandbox lost their owner")
		}
		app.Host = remoteHost
		local.workspaceSettingsFrame()
		if app.Settings.Sandbox.Revision != 3 || app.Settings.Hooks.Revision != 8 {
			t.Fatal("monitor changed settings ownership")
		}

		app.Settings.Hooks.Draft.Enabled = true
		app.Settings.SaveHooks()
		app.Settings.Sandbox.Draft = true
		app.Settings.SaveSandbox()
		app.Host = localHost
		local.OpenSession(localSession)
		local.workspaceSettingsFrame()
		settle(local, remote)
		if app.Settings.Hooks.Revision != 7 || app.Settings.Hooks.Draft.Enabled || app.Settings.Sandbox.Revision != 4 || !app.Settings.Sandbox.Draft {
			t.Fatal("remote hook completion overwrote local hooks or sandbox completion")
		}
		transports[1].mu.Lock()
		requests := append([]protocol.Request(nil), transports[1].requests...)
		transports[1].mu.Unlock()
		if len(requests) != 2 || requests[0].Command.Kind != "read_hooks" || requests[1].Command.Kind != "save_hooks" {
			t.Fatalf("remote hooks dispatched %+v", requests)
		}
		for _, request := range requests {
			if request.Target != remote.controller.Target() {
				t.Fatal("remote settings save escaped its execution Node")
			}
			if request.Command.Kind == "save_hooks" {
				input, valid := detail[protocol.CommandSaveHooksPayload](request.Command.Data)
				if !valid || input.Worktree != remote.workspaceSettings.worktree || input.ExpectedRevision != 8 || !input.Settings.Enabled {
					t.Fatal("remote hook save lost its captured worktree, revision or draft")
				}
			}
		}
		transports[0].mu.Lock()
		localRequests := append([]protocol.Request(nil), transports[0].requests...)
		transports[0].mu.Unlock()
		if len(localRequests) != 2 || localRequests[0].Command.Kind != "save_sandbox_settings" && localRequests[1].Command.Kind != "save_sandbox_settings" ||
			localRequests[0].Target != local.controller.Target() || localRequests[1].Target != local.controller.Target() {
			t.Fatalf("sandbox save escaped local Node: %+v", localRequests)
		}
		if controller, ok := local.SessionClient(remoteSession); !ok || controller != remote.controller {
			t.Fatal("export lost the remote session owner after local navigation")
		}
		app.Host = remoteHost
		local.OpenSession(remoteSession)
		local.workspaceSettingsFrame()
		settle(remote)
		if !app.Settings.Hooks.Draft.Enabled || !app.Settings.Sandbox.Draft || app.Settings.Hooks.Pending || app.Settings.Sandbox.Pending {
			t.Fatal("navigation lost the retained remote hook or local sandbox draft")
		}
	})
	for _, admitted := range []bool{false, true} {
		name := "snapshot worktree"
		if admitted {
			name = "newly admitted worktree"
		}
		t.Run(name, func(t *testing.T) {
			node, project, worktree := protocol.NodeID{1}, protocol.NewProjectID(), protocol.NewWorktreeID()
			tree := protocol.Worktree{ID: worktree, Project: &project, Path: "/fixture/hooks", Main: true}
			snapshot := protocol.DefaultSnapshot()
			snapshot.Node = node
			if !admitted {
				snapshot.Worktrees = []protocol.Worktree{tree}
			}
			source := &conversationTestTransport{snapshot: snapshot}
			source.handle = func(request protocol.Request) (link.Response, bool) {
				if request.Command.Kind != "read_hooks" {
					return link.Response{}, false
				}
				return link.Response{Output: protocol.Output{Kind: "hook_settings", Data: protocol.HooksState{Worktree: worktree, Settings: protocol.HooksSettings{Revision: 7}, Sandboxed: true}}}, true
			}
			controller := client.New(source)
			if err := controller.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			defer controller.Close()
			const draftID = "draft/hooks"
			data := &viewmodel.Data{
				Workspace: &viewmodel.Workspace{Hosts: []viewmodel.Host{{ID: node.String(), Local: true}}},
				Projects:  []*viewmodel.Project{{ID: project.String(), Host: node.String(), Conversations: []*viewmodel.Conversation{{ID: draftID, Node: node.String()}}}},
			}
			r := &Runtime{ctx: t.Context(), controller: controller, App: shell.NewLive(data), workspace: &workspaceState{admitted: map[protocol.WorktreeID]protocol.Worktree{}}}
			if admitted {
				r.workspace.admitted[worktree] = tree
			}
			r.App.Sidebar.Selected = draftID
			r.App.Composer(draftID).Worktree = tree.Path
			r.workspaceSettings.sandboxKnown = true
			r.workspaceSettingsFrame()
			settingsQueueTester(r)(r)
			if r.workspaceSettings.worktree != worktree || r.App.Settings.Hooks.Revision != 7 || !r.hooksSandboxed() {
				t.Fatal("project draft lost its captured hook owner or settings")
			}
			label, ok := r.hookContext()
			if !ok || !strings.Contains(label, node.String()) || !strings.Contains(label, tree.Path) || !strings.Contains(label, worktree.String()) {
				t.Fatalf("hook consent scope = %q, available=%v", label, ok)
			}
			delete(r.workspace.admitted, worktree)
			r.App.Data.Projects = nil
			r.App.Data.Unassigned = []*viewmodel.Conversation{{ID: "draft/unassigned", Node: node.String()}}
			r.App.Sidebar.Selected = "draft/unassigned"
			r.workspaceSettingsFrame()
			if r.workspaceSettings.worktree != (protocol.WorktreeID{}) || r.hooksSandboxed() || r.App.Settings.Hooks.Revision != 0 {
				t.Fatal("unassigned draft retained project hook authority")
			}
			if _, ok := r.hookContext(); ok {
				t.Fatal("unassigned draft exposed project hook consent")
			}
		})
	}
}

func settingsQueueTester(root *Runtime) func(...*Runtime) {
	tester := ui.NewTester(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		root.mu.Lock()
		queue := root.queue
		root.queue = nil
		root.mu.Unlock()
		for _, complete := range queue {
			complete(c)
		}
	}, 640, 480)
	return func(workers ...*Runtime) {
		for _, owner := range workers {
			owner.workers.Wait()
		}
		tester.Frame()
	}
}
