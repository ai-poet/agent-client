package live

import (
	"reflect"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type workspaceSettingsState struct {
	worktree        protocol.WorktreeID
	sandboxed       bool
	sandboxRevision uint64
	hookModels      map[protocol.WorktreeID]*workspaceHookModel
	hookBinding     uint64 // changes when this owner's selected worktree changes or is rebound
	hookObservation uint64
	hookAttempted   bool
	hookReady       bool
	sandboxModel    settings.Stored[bool]
	sandboxKnown    bool
}

type workspaceHookModel struct {
	settings.Stored[settings.HookSettings]
	uncertain *protocol.Request
}

func (r *Runtime) initWorkspaceSettings() {
	state := &r.workspaceSettings
	if state.hookModels != nil {
		return
	}
	state.hookModels = map[protocol.WorktreeID]*workspaceHookModel{}
	if r.parent == nil {
		state.sandboxModel = r.App.Settings.Sandbox
	}
}
func (r *Runtime) workspaceHooksModel(id protocol.WorktreeID) *workspaceHookModel {
	r.initWorkspaceSettings()
	if model := r.workspaceSettings.hookModels[id]; model != nil {
		return model
	}
	model := &workspaceHookModel{}
	r.workspaceSettings.hookModels[id] = model
	return model
}
func (r *Runtime) captureWorkspaceSettings() {
	r.initWorkspaceSettings()
	if r.parent == nil {
		r.workspaceSettings.sandboxModel = r.App.Settings.Sandbox
	}
	if r.localRuntime().workspaceSettingsOwner != r {
		return
	}
	if id := r.workspaceSettings.worktree; id != (protocol.WorktreeID{}) {
		r.workspaceHooksModel(id).Draft = r.App.Settings.Hooks.Draft
	}
}
func (r *Runtime) publishWorkspaceSettings() {
	if r.parent == nil {
		r.App.Settings.Sandbox = r.workspaceSettings.sandboxModel
	}
	if r.localRuntime().workspaceSettingsOwner != r {
		return
	}
	m, state := r.App.Settings, &r.workspaceSettings
	m.Hooks = settings.Stored[settings.HookSettings]{}
	m.SaveHooks = nil
	if id := state.worktree; id != (protocol.WorktreeID{}) {
		model := r.workspaceHooksModel(id)
		m.Hooks = model.Stored
		if state.hookReady && !model.Pending && model.uncertain == nil {
			binding, observation := state.hookBinding, state.hookObservation
			if r.workspaceHookCurrent(id, binding, observation) {
				m.SaveHooks = func() {
					if r.workspaceHookCurrent(id, binding, observation) && state.hookReady && !model.Pending && model.uncertain == nil {
						r.saveWorkspaceHooks(id, model)
					}
				}
			}
		}
	}
}

func (r *Runtime) invalidateWorkspaceHooks() {
	state := &r.workspaceSettings
	state.hookBinding++
	state.hookAttempted, state.hookReady, state.sandboxed = false, false, false
}

func (r *Runtime) workspaceHookCurrent(id protocol.WorktreeID, binding, observation uint64) bool {
	state := &r.workspaceSettings
	return r.localRuntime().workspaceSettingsOwner == r && id == state.worktree &&
		binding == state.hookBinding && observation != 0 && observation == r.controller.ObservationGeneration() &&
		(r.ctx == nil || r.ctx.Err() == nil)
}

func (r *Runtime) bindWorkspaceSettings() {
	root := r.localRuntime()
	r.initWorkspaceSettings()
	root.workspaceSettingsOwner = r
	r.invalidateWorkspaceHooks()
	root.App.Settings.SaveSandbox = root.saveSandbox
	root.App.Settings.HooksContext = r.hookContext
	root.App.Settings.HooksSandboxed = r.hooksSandboxed
	r.publishWorkspaceSettings()
}

// workspaceSettingsFrame keeps sandbox configuration local and hooks on the
// selected project's execution Node. Hook editors and in-flight saves retain
// their captured owner and worktree while navigation changes.
func (r *Runtime) workspaceSettingsFrame() {
	root := r.localRuntime()
	root.captureWorkspaceSettings()
	root.App.Settings.SaveSandbox = root.saveSandbox
	state := &root.workspaceSettings
	if (!state.sandboxKnown || root.snapshot().SandboxSettingsRevision > state.sandboxRevision) && !state.sandboxModel.Pending {
		root.readSandbox()
	}
	owner := root.selectedRuntime()
	previous := root.workspaceSettingsOwner
	if previous != nil {
		if previous != root {
			previous.captureWorkspaceSettings()
		}
		if root.App.Settings.HooksEditing() {
			owner = previous
		}
	}
	if owner == nil {
		root.workspaceSettingsOwner = nil
		m := root.App.Settings
		m.SaveHooks, m.HooksSandboxed = nil, nil
		m.HooksContext = func() (string, bool) { return "", false }
		m.Hooks = settings.Stored[settings.HookSettings]{}
	} else {
		if previous != owner {
			owner.bindWorkspaceSettings()
		}
		owner.refreshWorkspaceSettings()
	}
	root.publishWorkspaceSettings()
}

func (r *Runtime) refreshWorkspaceSettings() {
	if r.ctx != nil && r.ctx.Err() != nil {
		return
	}
	r.initWorkspaceSettings()
	state := &r.workspaceSettings
	// The hooks are those of the conversation shown, when it runs on this
	// Node; a conversation on another host has its hooks on that host.
	var id protocol.WorktreeID
	if r.localRuntime().selectedRuntime() == r {
		session, ok := r.session(r.App.Sidebar.Selected)
		if !ok {
			session, ok = r.draftWorkspaceSession(r.App.Sidebar.Selected)
		}
		if ok && session.Project != nil {
			id = session.Worktree
		}
	}
	if id != state.worktree && !r.App.Settings.HooksEditing() {
		state.worktree = id
		r.invalidateWorkspaceHooks()
	}
	observation := r.controller.ObservationGeneration()
	if observation != state.hookObservation {
		state.hookObservation = observation
		r.invalidateWorkspaceHooks()
	}
	if observation != 0 && state.worktree != (protocol.WorktreeID{}) && !state.hookAttempted {
		r.readWorkspaceHooks(state.worktree)
	}
	r.publishWorkspaceSettings()
}
func (r *Runtime) hookContext() (string, bool) {
	id := r.workspaceSettings.worktree
	if id == (protocol.WorktreeID{}) {
		return "", false
	}
	var tree protocol.Worktree
	found := false
	for _, candidate := range r.snapshot().Worktrees {
		if candidate.ID == id {
			tree, found = candidate, true
			break
		}
	}
	if !found && r.workspace != nil {
		tree, found = r.workspace.admitted[id]
	}
	if !found || tree.Project == nil {
		return "", false
	}
	label := r.controller.Target().String() + " · " + tree.Path + " · " + tree.ID.String()
	if r.workspace != nil {
		if project := r.workspace.projects[id]; project != nil && project.status.Branch != nil {
			label += " · " + *project.status.Branch
		}
	}
	return label, true
}
func (r *Runtime) hooksSandboxed() bool {
	return r.workspaceSettings.sandboxed
}
func hookDraft(value protocol.HooksSettings) settings.HookSettings {
	draft := settings.HookSettings{Enabled: value.Enabled, Scripts: []viewmodel.Hook{}}
	add := func(id string, phase viewmodel.HookPhase, script *protocol.HooksScript) {
		if script != nil {
			draft.Scripts = append(draft.Scripts, viewmodel.Hook{ID: id, Name: script.Name, Command: script.Command, Phase: phase, TimeoutMS: int(script.TimeoutMs), Enabled: script.Enabled})
		}
	}
	add("before_tool", viewmodel.BeforeTool, value.BeforeTool)
	add("after_tool", viewmodel.AfterTool, value.AfterTool)
	add("turn_end", viewmodel.TurnEnd, value.TurnEnd)
	for _, entry := range value.Additional {
		phase := viewmodel.BeforeTool
		switch entry.Phase {
		case protocol.HooksPhaseAfterTool:
			phase = viewmodel.AfterTool
		case protocol.HooksPhaseTurnEnd:
			phase = viewmodel.TurnEnd
		}
		add(entry.ID, phase, &entry.Script)
	}
	return draft
}
func (r *Runtime) readWorkspaceHooks(id protocol.WorktreeID) {
	model := r.workspaceHooksModel(id)
	if model.Pending {
		return
	}
	state := &r.workspaceSettings
	binding, observation := state.hookBinding, state.hookObservation
	state.hookAttempted = true
	model.Pending = true
	if !r.async(func() {
		out, err := r.controller.Do(r.ctx, protocol.Command{Kind: "read_hooks", Data: protocol.CommandReadHooksPayload{Worktree: id}})
		r.enqueue(func(c *ui.Context) {
			model.Pending = false
			if !r.workspaceHookCurrent(id, binding, observation) {
				return
			}
			defer r.publishWorkspaceSettings()
			if err != nil {
				r.reportKeys(c, err, "hooks_failed", "hooks_conflict", "hooks_unknown")
				return
			}
			value, ok := detail[protocol.HooksState](out.Data)
			if !ok || out.Kind != "hook_settings" || value.Worktree != id || value.Settings.Revision < model.Revision {
				settings.Report(c, kit.Failed, workspaceHookMessages)
				return
			}
			draft := hookDraft(value.Settings)
			if reflect.DeepEqual(model.Draft, model.Saved) {
				model.Draft = draft
			}
			model.Saved, model.Revision = draft, value.Settings.Revision
			state.sandboxed, state.hookReady = value.Sandboxed, true
			if request := model.uncertain; request != nil {
				// Refresh or returning to this worktree makes recovery available
				// even if the original action toast has expired.
				settings.ReportRetry(c, kit.Uncertain, workspaceHookMessages, r.retryWorkspaceHooks(id, model, request))
			}
		})
	}) {
		model.Pending = false
	}
}

var workspaceHookMessages = settings.Messages{Done: "hooks_saved", Failed: "hooks_failed", Conflict: "hooks_conflict", Uncertain: "hooks_unknown"}

func (r *Runtime) saveWorkspaceHooks(id protocol.WorktreeID, m *workspaceHookModel) {
	r.captureWorkspaceSettings()
	value := protocol.DefaultHooksSettings()
	value.Revision = m.Revision
	value.Enabled = m.Draft.Enabled
	for _, entry := range m.Draft.Scripts {
		phase := protocol.HooksPhaseBeforeTool
		switch entry.Phase {
		case viewmodel.AfterTool:
			phase = protocol.HooksPhaseAfterTool
		case viewmodel.TurnEnd:
			phase = protocol.HooksPhaseTurnEnd
		}
		script := protocol.HooksScript{Name: entry.Name, Enabled: entry.Enabled, Command: entry.Command, TimeoutMs: uint64(entry.TimeoutMS)}
		switch entry.ID {
		case "before_tool":
			value.BeforeTool = &script
		case "after_tool":
			value.AfterTool = &script
		case "turn_end":
			value.TurnEnd = &script
		default:
			value.Additional = append(value.Additional, protocol.HooksEntry{ID: entry.ID, Phase: phase, Script: script})
		}
	}
	request := r.controller.Prepare(protocol.Command{Kind: "save_hooks", Data: protocol.CommandSaveHooksPayload{Worktree: id, Settings: value, ExpectedRevision: m.Revision}})
	r.settleWorkspaceHooks(id, m, &request, false)
}

func (r *Runtime) retryWorkspaceHooks(id protocol.WorktreeID, model *workspaceHookModel, request *protocol.Request) func() {
	return func() {
		state := &r.workspaceSettings
		// The original write can be inspected after reconnect, but not while
		// a different binding is displayed or after that write was settled.
		if r.workspaceHookCurrent(id, state.hookBinding, state.hookObservation) && !model.Pending && model.uncertain == request {
			r.settleWorkspaceHooks(id, model, request, true)
		}
	}
}

func (r *Runtime) settleWorkspaceHooks(id protocol.WorktreeID, m *workspaceHookModel, request *protocol.Request, inspect bool) {
	state := &r.workspaceSettings
	binding, observation := state.hookBinding, r.controller.ObservationGeneration()
	m.Pending = true
	r.publishWorkspaceSettings()
	if !r.async(func() {
		var out protocol.Output
		var err error
		if inspect {
			var result client.Resolution
			result, err = r.controller.Resolve(r.ctx, *request)
			if err != nil {
				// A failed query does not settle the original write.
				result = client.Resolution{State: client.RequestPending}
			}
			out, err = result.Result()
		} else {
			out, err = r.controller.Settle(r.ctx, *request)
		}
		r.enqueue(func(c *ui.Context) {
			m.Pending = false
			m.uncertain = nil
			outcome := kit.Done
			var again func()
			value, valid := detail[protocol.HooksState](out.Data)
			switch {
			case client.Retained(err):
				outcome = kit.Uncertain
				m.uncertain = request
				again = r.retryWorkspaceHooks(id, m, request)
			case faultCode(err) == protocol.ErrorCodeOutcomeUnknown:
				outcome = kit.Uncertain
			case faultCode(err) == protocol.ErrorCodeConflict || faultCode(err) == protocol.ErrorCodeRevisionConflict:
				outcome = kit.Conflict
			case err != nil || !valid || out.Kind != "hook_settings" || value.Worktree != id:
				outcome = kit.Failed
			}
			settings.ReportRetry(c, outcome, workspaceHookMessages, again)
			if !r.workspaceHookCurrent(id, binding, observation) {
				return
			}
			if outcome == kit.Done && value.Settings.Revision >= m.Revision {
				m.Saved, m.Draft, m.Revision = hookDraft(value.Settings), hookDraft(value.Settings), value.Settings.Revision
				state.sandboxed = value.Sandboxed
			} else if err == nil || outcome == kit.Conflict || outcome == kit.Uncertain && m.uncertain == nil {
				// Rebase the retained draft once the current revision can be read.
				r.invalidateWorkspaceHooks()
			}
			r.publishWorkspaceSettings()
		})
	}) {
		m.Pending = false
		r.publishWorkspaceSettings()
	}
}
func (r *Runtime) readSandbox() {
	r.initWorkspaceSettings()
	m := &r.workspaceSettings.sandboxModel
	if m.Pending {
		return
	}
	m.Pending = true
	r.workspaceSettings.sandboxRevision = r.snapshot().SandboxSettingsRevision
	r.workspaceSettings.sandboxKnown = true
	r.publishWorkspaceSettings()
	r.execute(protocol.Command{Kind: "read_sandbox_settings"}, func(c *ui.Context, out protocol.Output, err error) {
		m.Pending = false
		defer r.publishWorkspaceSettings()
		if err != nil {
			r.reportKeys(c, err, "sandbox_settings_failed", "sandbox_settings_conflict", "sandbox_settings_unknown")
			return
		}
		value, ok := detail[protocol.SandboxSettings](out.Data)
		if !ok {
			return
		}
		if m.Draft == m.Saved {
			m.Draft = value.Enabled
		}
		m.Saved, m.Revision = value.Enabled, value.Revision
		r.workspaceSettings.sandboxRevision = value.Revision
	})
}
func (r *Runtime) saveSandbox() {
	if r.ctx != nil && r.ctx.Err() != nil {
		r.Report(fileActionFault(protocol.ErrorCodeUnavailable, "the sandbox execution host is unavailable"))
		return
	}
	r.captureWorkspaceSettings()
	m := &r.workspaceSettings.sandboxModel
	if m.Pending {
		return
	}
	m.Pending = true
	r.publishWorkspaceSettings()
	r.saveSettingRetry(protocol.Command{Kind: "save_sandbox_settings", Data: protocol.SandboxSettings{Revision: m.Revision, Enabled: m.Draft}}, settings.Messages{Done: "sandbox_settings_saved", Failed: "sandbox_settings_failed", Conflict: "sandbox_settings_conflict", Uncertain: "sandbox_settings_unknown"}, nil, r.saveSandbox, func(out protocol.Output, ok bool) {
		m.Pending = false
		defer r.publishWorkspaceSettings()
		if !ok {
			return
		}
		value, valid := detail[protocol.SandboxSettings](out.Data)
		if !valid {
			return
		}
		m.Saved, m.Revision = value.Enabled, value.Revision
		r.workspaceSettings.sandboxRevision = value.Revision
	})
}
