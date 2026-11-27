package node

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	pathpkg "path"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/node/admission"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/engine"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/operations"
	"github.com/ai-poet/cheaprouter-harness/internal/node/files"
	"github.com/ai-poet/cheaprouter-harness/internal/node/process"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

const planRunCommandDescription = "Run a foreground workspace command in an enabled, available read-only OS sandbox. No workspace writes, network, unrestricted access, background commands, services, or PTY. Otherwise use dedicated read-only tools."

type workspaceToolCall struct {
	request               protocol.Request
	prepared              *files.Prepared
	approval              protocol.ApprovalID
	commandControl        *process.ControlTarget
	patchInput, patchRoot string
	fileScope             externalFileScope
}

func (node *Node) workspaceTools(ctx context.Context, inv *store.AgentInvocation) ([]engine.Tool, error) {
	if _, err := node.workspaceRoot(ctx, inv.Run.Worktree, false); err != nil {
		return nil, err
	}
	catalog, err := operations.CaptureCatalog(ctx, node.profile.Ownership.Path, inv.Turn.Plugins)
	if err != nil {
		return nil, err
	}
	var tools []engine.Tool
	for _, declaration := range catalog.Tools {
		if !availableOperationTool(inv, declaration) || !slices.Contains(inv.Turn.Plugins, declaration.Owner) || knowledgeOperation(declaration.Operation) || storageOperation(declaration.Operation) || operations.ScopedOperation(declaration.Operation) {
			continue
		}
		// A connection conversation without an assistant offers only tools that name a context.
		if inv.Turn.Config.Resource != nil && inv.Turn.Config.Assistant == nil && len(declaration.Contexts) == 0 &&
			!strings.HasPrefix(string(declaration.Operation), "artifacts.") {
			continue
		}
		if declaration.Operation == protocol.ToolOperationWriteArtifact && inv.Session.Delegation != nil {
			continue
		}
		var schema map[string]any
		if err = json.Unmarshal(declaration.Parameters, &schema); err != nil {
			return nil, err
		}
		planning := inv.Turn.Config.Mode == protocol.WorkModePlan
		if planning && declaration.Operation == protocol.ToolOperationRunCommand {
			declaration.Description = planRunCommandDescription
		}
		var mu sync.Mutex
		calls := map[string]*workspaceToolCall{}
		tool := engine.Tool{Definition: fantasy.FunctionTool{Name: declaration.Name, Description: declaration.Description, InputSchema: schema}, SchemaJSON: declaration.Parameters, ReadOnly: declaration.ReadOnly, Concurrent: declaration.Concurrent, Deferred: !declaration.Initial, Approval: !declaration.ReadOnly, Binding: &store.ApprovalBinding{Owner: declaration.Owner, Tool: declaration.Name, Operation: &declaration.Operation}}
		observed := workspaceRevisions{worktreeID: inv.Run.Worktree}
		fullFiles := inv.Turn.Config.Permission == protocol.PermissionFull && externalFileOperation(declaration.Operation)
		if fullFiles {
			observed.worktree = inv.Worktree.Path
		}
		if observesRevisions(declaration.Operation) {
			tool.ObserveHistory = observed.update
		}
		tool.Prepare = func(ctx context.Context, call canonical.ToolRequest) error {
			if planning {
				// Worktree status and diffs can execute repository-configured
				// filters in native Git, outside the command sandbox.
				switch declaration.Operation {
				case protocol.ToolOperationInspectGit, protocol.ToolOperationReadGitDiff, protocol.ToolOperationListGitChanges, protocol.ToolOperationReadGitChange:
					return &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "native Git worktree inspection can run external filters; use a read-only sandbox command"}
				}
				if !declaration.ReadOnly && declaration.Operation != protocol.ToolOperationRunCommand && declaration.Operation != protocol.ToolOperationWriteArtifact {
					return &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "planning mode permits read-only tools"}
				}
			}
			args := call.Arguments
			worktree := filepath.Clean(inv.Worktree.Path)
			var patchInput string
			var scope externalFileScope
			if fullFiles {
				worktree = resolvedPath(worktree)
				canonical := func(path string) (string, error) { return localPath(worktree, canonicalPath(path)), nil }
				var err error
				if args, err = fileArgPaths(declaration.Operation, args, canonical); err != nil {
					return err
				}
				paths := collectFileArgPaths(declaration.Operation, args)
				scope.root = externalRoot(worktree, paths)
				if scope.root != "" {
					for _, path := range paths {
						if !filepath.IsAbs(path) {
							scope.worktree = worktree
							break
						}
					}
				}
			}
			arguments, err := observed.resolve(declaration.Operation, args)
			if err != nil {
				return err
			}
			root := worktree
			if scope.root != "" {
				root = scope.root
				if arguments, err = fileArgPaths(declaration.Operation, arguments, func(path string) (string, error) { return rootPath(worktree, root, path) }); err != nil {
					return err
				}
			}
			sandbox := protocol.DefaultSandboxSettings()
			if declaration.Operation == protocol.ToolOperationRunCommand {
				// Each command follows the setting as it is now, not as it
				// was when the turn started.
				if err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
					var err error
					sandbox, err = store.ReadSandboxSettings(ctx, db)
					return err
				}); err != nil {
					return err
				}
			}
			command, err := operations.Command(declaration.Operation, arguments, inv.Run.Worktree, inv.Session.ID, inv.Run.Turn, inv.Worktree.Project, planning || sandbox.Enabled && inv.Turn.Config.Permission != protocol.PermissionFull)
			if err == nil && planning && declaration.Operation == protocol.ToolOperationRunCommand {
				err = store.CheckPlanRunCommand(*command.Data.(*protocol.CommandRunCommandPayload), process.SandboxSupported(), sandbox.Enabled)
			}
			if declaration.Operation == protocol.ToolOperationApplyChanges {
				var patch struct {
					Input string `json:"input"`
				}
				if err = json.Unmarshal(arguments, &patch); err != nil {
					return err
				}
				patchInput = patch.Input
				revisions := observed.paths
				if scope.root != "" {
					revisions = rootRevisions(worktree, root, revisions)
				}
				changes, e := files.ResolvePatchWithRevisions(ctx, root, patch.Input, revisions, core.RestrictedFiles)
				if e != nil {
					return e
				}
				command = protocol.Command{Kind: "apply_changes", Data: &protocol.CommandApplyChangesPayload{Worktree: inv.Run.Worktree, Changes: changes}}
				err = nil
			}
			if err != nil {
				return err
			}
			if target, ok := commandWorktree(command); ok && target != inv.Run.Worktree {
				allowed := false
				if inv.Worktree.Project != nil {
					err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
						capture, e := store.ReadWorkspaceGitCapture(ctx, db, target)
						if e != nil {
							return e
						}
						allowed = capture.Project.ID == *inv.Worktree.Project
						return nil
					})
					if err != nil {
						return err
					}
				}
				if !allowed {
					return &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "tool worktree is outside the captured project"}
				}
			}
			request := protocol.NewRequest(node.ID(), command)
			request.Plugin = &protocol.PluginContext{Turn: &inv.Run.Turn, Package: declaration.Owner, Worktree: &inv.Run.Worktree, Session: &inv.Session.ID, Surface: protocol.PluginDesktopSurface{Kind: "workspace"}}
			current := &workspaceToolCall{request: request, fileScope: scope, patchInput: patchInput, patchRoot: root}
			if fileTextMutation(command.Kind) {
				current.prepared, err = files.Prepare(ctx, root, command, core.RestrictedFiles)
				if err != nil {
					return err
				}
			}
			if files.SupportsMutation(command.Kind) {
				target := root
				if scope.worktree != "" {
					target = inv.Worktree.Path
				}
				if err = node.protectWorkspaceMutation(target, target, command, current.prepared, scope); err != nil {
					return err
				}
			}
			mu.Lock()
			calls[call.ID] = current
			mu.Unlock()
			return nil
		}
		tool.ConfigureApproval = func(ctx context.Context, db *sql.Conn, call canonical.ToolRequest, value protocol.ConversationApproval) (protocol.ConversationApproval, error) {
			mu.Lock()
			current := calls[call.ID]
			mu.Unlock()
			var changes *protocol.ChangePreview
			if current.prepared != nil {
				changes = &current.prepared.Preview
			}
			switch input := current.request.Command.Data.(type) {
			case *protocol.CommandStopCommandPayload:
				if input.Session == inv.Session.ID {
					current.commandControl = node.processes.SandboxedControl(input.Session, input.ID)
				}
			case *protocol.CommandWriteCommandPayload:
				if input.Session == inv.Session.ID {
					current.commandControl = node.processes.SandboxedControl(input.Session, input.ID)
				}
			}
			if current.commandControl != nil {
				value.Access = new(protocol.ProcessAccessWorkspace)
			}
			return store.ConfigureWorkspaceApproval(ctx, db, inv.Run.Turn, value, declaration.Operation, current.request.Command, changes)
		}
		tool.Approved = func(call canonical.ToolRequest, approval protocol.ApprovalID, request protocol.RequestID) {
			mu.Lock()
			defer mu.Unlock()
			current := calls[call.ID]
			current.approval = approval
			current.request.ID = request
		}
		tool.Execute = func(ctx context.Context, call canonical.ToolRequest) engine.ToolResult {
			mu.Lock()
			current := calls[call.ID]
			delete(calls, call.ID)
			mu.Unlock()
			if current == nil {
				return workspaceToolFault(errors.New("native tool was not prepared"))
			}
			if current.patchInput != "" && inv.Turn.Config.Permission == protocol.PermissionFull && current.prepared.Rejected == nil {
				// This fork serializes workspace effects at the root. Check the complete
				// prepared set before admission; Apply checks each precondition again.
				release := node.locks.roots(current.patchRoot, inv.Worktree.Path)
				fresh, err := files.RepreparePatch(ctx, current.patchRoot, current.patchInput, current.prepared)
				release()
				if err != nil {
					return workspaceToolFault(err)
				}
				target := current.patchRoot
				if current.fileScope.worktree != "" {
					target = inv.Worktree.Path
				}
				if err := node.protectWorkspaceMutation(target, target, fresh.Command, fresh, current.fileScope); err != nil {
					return workspaceToolFault(err)
				}
				current.prepared, current.request.Command = fresh, fresh.Command
			}
			request := current.request
			scope := current.fileScope
			if scope.root != "" && scope.worktree == "" {
				output, err := node.externalFileIO(ctx, inv.Caller, request, current.prepared, scope.root)
				if err != nil {
					return workspaceToolFault(err)
				}
				value := node.workspaceToolValue(ctx, request, scope.result(output), false, planning)
				return engine.ToolResult{Value: value, ReturnedError: workspaceOutputFailed(output), ReturnedFailure: workspaceOutputFailure(output)}
			}
			if !declaration.ReadOnly {
				body, err := json.Marshal(request)
				if err != nil {
					return workspaceToolFault(err)
				}
				unlock := node.agent.LockPublication()
				err = node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
					if err := store.ApprovedWorkspaceRequest(ctx, db, inv.Run.Turn, current.approval, request); err != nil {
						return err
					}
					previous, err := admission.Admit(ctx, db, inv.Caller, request, body)
					if err != nil {
						return err
					}
					if previous != nil {
						return &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "tool effect was already admitted and will not replay"}
					}
					if current.prepared != nil && current.prepared.Rejected == nil {
						return store.CaptureFileCheckpointsRequest(ctx, db, inv.Run.Turn, current.approval, request, scope.checkpoints(current.prepared))
					}
					return nil
				})
				unlock()
				if err != nil {
					return workspaceToolFault(err)
				}
			}
			effect := &workspaceCompletion{commandControl: current.commandControl, fileScope: scope}
			defer effect.close()
			output, effectErr := node.workspaceIO(ctx, inv.Caller, request, current.prepared, effect)
			if !declaration.ReadOnly {
				output, effectErr = node.recordWorkspaceEffect(context.WithoutCancel(ctx), inv.Caller, request, output, effectErr, effect)
			}
			if effectErr != nil {
				return workspaceToolFault(effectErr)
			}
			if declaration.Operation == protocol.ToolOperationReadCommand {
				if snapshot, ok := output.Data.(protocol.ProcessSnapshot); ok {
					unlock := node.agent.LockPublication()
					err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
						events, err := store.ConsumeCommandBackground(ctx, db, inv.Run.Turn, snapshot, time.Now())
						if err != nil {
							return err
						}
						_, err = store.RecordEvents(ctx, db, node.ID(), events)
						return err
					})
					if err == nil {
						node.publish()
						node.agent.RefreshCommitted(inv.Session.ID)
					}
					unlock()
					if err != nil {
						return workspaceToolFault(err)
					}
					node.agent.Committed()
				}
			}
			value := node.workspaceToolValue(ctx, request, output, effect.timedOut, planning)
			var readMeta *map[string]json.RawMessage
			if page, ok := output.Data.(protocol.FileLines); ok && len(page.Lines) > 0 && page.Revision != "" {
				raw, _ := json.Marshal(map[string]string{page.Path: page.Revision})
				readMeta = &map[string]json.RawMessage{canonical.FileReadsKey(inv.Run.Worktree): raw}
			}
			return engine.ToolResult{Value: value, Meta: readMeta, ReturnedError: workspaceOutputFailed(output), ReturnedFailure: workspaceOutputFailure(output)}
		}
		tools = append(tools, tool)
	}
	indexed, err := node.knowledgeTools(inv, catalog)
	if err != nil {
		return nil, err
	}
	tools = append(tools, indexed...)
	stored, err := node.storageTools(inv, catalog)
	if err != nil {
		return nil, err
	}
	tools = append(tools, stored...)
	scoped, err := node.scopedTools(inv, catalog)
	if err != nil {
		return nil, err
	}
	tools = append(tools, scoped...)
	browsers, err := node.browserTools(ctx, inv)
	if err != nil {
		return nil, err
	}
	managedBrowsers, err := node.externalBrowserTools(ctx, inv)
	if err != nil {
		return nil, err
	}
	media, err := node.mediaTools(inv)
	if err != nil {
		return nil, err
	}
	managed, err := node.agentTools(inv)
	if err != nil {
		return nil, err
	}
	bound, err := node.pluginTools(ctx, inv, append(append(tools, browsers...), managedBrowsers...))
	return append(append(bound, media...), managed...), err
}
func workspaceToolFault(err error) engine.ToolResult {
	return engine.ToolResult{Failure: nodeFault(err).(*protocol.Fault)}
}
func workspaceOutputFailed(output protocol.Output) bool { return workspaceOutputFailure(output) != nil }
func workspaceOutputFailure(output protocol.Output) *protocol.ToolFailure {
	var outcome *protocol.ProcessOutcome
	switch value := output.Data.(type) {
	case protocol.ProcessCompletion:
		outcome = &value.Outcome
	case protocol.ProcessSnapshot:
		switch value.Info.Status.Kind {
		case "finished":
			raw, _ := json.Marshal(value.Info.Status.Data)
			var decoded protocol.ProcessOutcome
			if json.Unmarshal(raw, &decoded) == nil {
				outcome = &decoded
			}
		case "failed":
			return &protocol.ToolFailure{Origin: protocol.ToolFailureOriginRuntime}
		}
	case protocol.FileChanges:
		if value.Status != protocol.ChangesStatusApplied {
			return &protocol.ToolFailure{Origin: protocol.ToolFailureOriginTool}
		}
	}
	if outcome == nil {
		return nil
	}
	failure := &protocol.ToolFailure{Origin: protocol.ToolFailureOriginRuntime}
	switch outcome.Kind {
	case "exited":
		raw, _ := json.Marshal(outcome.Data)
		var code int32
		if json.Unmarshal(raw, &code) == nil && code == 0 {
			return nil
		}
		failure.Origin = protocol.ToolFailureOriginTool
	case "cancelled":
		code := protocol.ErrorCodeCancelled
		failure.Code = &code
	case "unknown":
		code := protocol.ErrorCodeOutcomeUnknown
		failure.Code = &code
	}
	return failure
}
func (node *Node) workspaceToolValue(ctx context.Context, request protocol.Request, output protocol.Output, timedOut, planning bool) any {
	if value, shaped := codeToolValue(output); shaped {
		return value
	}
	if output.Kind != "command_result" && output.Kind != "command_output" && output.Kind != "command_started" {
		return output
	}
	raw, err := json.Marshal(output)
	if err != nil {
		return output
	}
	var value map[string]any
	if json.Unmarshal(raw, &value) != nil {
		return output
	}
	id := request.ID
	switch input := request.Command.Data.(type) {
	case *protocol.CommandReadCommandPayload:
		id = input.ID
	case *protocol.CommandStopCommandPayload:
		id = input.ID
	}
	if output.Kind == "command_result" || output.Kind == "command_output" {
		value["sailry_command"] = map[string]any{"id": id}
		var session protocol.SessionID
		if request.Plugin != nil && request.Plugin.Session != nil {
			session = *request.Plugin.Session
		}
		if snapshot, ok := output.Data.(protocol.ProcessSnapshot); ok {
			session = snapshot.Info.Session
			id = snapshot.Info.ID
		}
		if preview, err := node.processes.Preview(session, id, output.Kind == "command_output"); err == nil {
			value["sailry_command"] = preview
		}
	}
	if input, ok := request.Command.Data.(*protocol.CommandRunCommandPayload); ok && !input.Background && timedOut && output.Kind == "command_result" {
		if data, ok := value["data"].(map[string]any); ok {
			data["notice"] = "Background command capacity was full, so the command was stopped at its timeout."
		}
	}
	if input, ok := request.Command.Data.(*protocol.CommandRunCommandPayload); ok && output.Kind == "command_started" && !input.Background {
		if data, ok := value["data"].(map[string]any); ok {
			notice := "The user sent a new message while this command was running. The command was moved to the background and is still running; use the command read and stop tools with this id to check it. Its completion is reported automatically."
			if timedOut {
				notice = "The command reached timeout_ms and is still running in the background. Use read_command with this id (wait_ms, until) to follow it, or stop_command to end it. Its completion is reported automatically."
			}
			data["notice"] = notice
		}
	}
	if workspaceOutputFailed(output) {
		workspace := false
		text := ""
		if input, ok := request.Command.Data.(*protocol.CommandRunCommandPayload); ok {
			workspace = input.Access == protocol.ProcessAccessWorkspace
		}
		switch result := output.Data.(type) {
		case protocol.ProcessCompletion:
			text = result.Stdout.Text + "\n" + result.Stderr.Text
		case protocol.ProcessSnapshot:
			workspace = result.Info.Access == protocol.ProcessAccessWorkspace
			text = result.Stdout.Text + "\n" + result.Stderr.Text
		}
		if workspace {
			lower := strings.ToLower(text)
			for _, diagnostic := range []string{"operation not permitted", "permission denied", "sandbox", "dyld:", "dyld[", "could not resolve host", "network is unreachable"} {
				if strings.Contains(lower, diagnostic) {
					if data, ok := value["data"].(map[string]any); ok {
						if planning {
							data["sandbox_notice"] = "This command ran in a read-only workspace sandbox with no network. Unrestricted access is unavailable; use dedicated read-only tools instead."
						} else {
							data["sandbox_notice"] = "This command ran in the workspace sandbox: network access is disabled, writes are limited to the worktree and temporary directories, and existing Git metadata and managed child worktrees are protected. macOS also blocks new .git paths. Git writes, including git init, git add and git commit, require access=\"unrestricted\" under the current permission policy. If sandbox restrictions caused the failure, inspect any partial changes before requesting unrestricted access to retry. Unrestricted access does not override operating-system permissions."
						}
					}
					break
				}
			}
		}
	}
	return value
}

// workspaceRevisions follows the canonical history visible to this turn. Receipts
// from a successful write are observations of its new revision, like reads.
type workspaceRevisions struct {
	worktreeID  protocol.WorktreeID
	invalidated map[string]struct{}
	paths       map[string]string
	count       int
	last        string
	worktree    string
}

func (r *workspaceRevisions) update(records []canonical.Record) {
	if r.paths == nil || r.count > len(records) || r.count > 0 && records[r.count-1].ID != r.last {
		r.paths = map[string]string{}
		r.invalidated = nil
		r.count = 0
	}
	for _, record := range records[r.count:] {
		if record.Kind == "background" {
			if rt := record.Metadata.Runtime; rt != nil && rt.FileChanges != nil {
				if changes := rt.FileChanges[r.worktreeID.String()]; len(changes) != 0 {
					r.observeChildChanges(changes, r.pathKey, combinesChildChanges(record.Background()))
				}
			}
			continue
		}
		message := record.Message()
		if message == nil || record.Provenance.Author != "tool" {
			continue
		}
		for _, block := range message.Content {
			response, ok := block.Data.(*canonical.ToolResponse)
			if !ok {
				if v, ok := block.Data.(canonical.ToolResponse); ok {
					response = &v
				} else {
					continue
				}
			}
			r.observe(response, r.pathKey)
		}
	}
	r.count = len(records)
	if r.count > 0 {
		r.last = records[r.count-1].ID
	}
}

// Batched background delivery flattens child receipts into one path map. A
// value can already have overwritten a different revision for the same path.
func combinesChildChanges(background *canonical.Background) bool {
	if background == nil {
		return false
	}
	seenChild := false
	for _, result := range background.Results {
		if result.Notice.Kind.Kind == "agent" {
			if seenChild {
				return true
			}
			seenChild = true
		}
	}
	return false
}

func (r *workspaceRevisions) observeChildChanges(files map[string]string, key func(string) string, combined bool) {
	for path, rev := range files {
		k := key(path)
		if k == "" {
			continue
		}
		if rev == "" {
			r.invalidateTree(k)
			continue
		}
		if combined {
			r.invalidate(k)
			continue
		}
		// Delivery order is not write order. Conflicting evidence invalidates
		// the observation; a duplicate cannot revive it without a direct read
		// or write. This also handles differently spelled aliases in old maps.
		if known, ok := r.paths[k]; ok {
			if known != rev {
				r.invalidate(k)
			}
			continue
		}
		if !r.isInvalidated(k) {
			r.paths[k] = rev
		}
	}
}

func (r *workspaceRevisions) isInvalidated(path string) bool {
	if len(r.invalidated) == 0 {
		return false
	}
	for {
		if _, ok := r.invalidated[path]; ok {
			return true
		}
		i := strings.LastIndexByte(path, '/')
		if i <= 0 {
			return false
		}
		path = path[:i]
	}
}

func (r *workspaceRevisions) observe(response *canonical.ToolResponse, key func(string) string) {
	if response.Status == canonical.ToolSuccess && response.Meta != nil {
		if raw := (*response.Meta)[canonical.FileReadsKey(r.worktreeID)]; raw != nil {
			var reads map[string]string
			if json.Unmarshal(raw, &reads) == nil {
				for path, revision := range reads {
					r.observeRevision(key(path), revision)
				}
			}
		}
		if raw := (*response.Meta)[canonical.FileChangesKey(r.worktreeID)]; raw != nil {
			var files map[string]string
			if json.Unmarshal(raw, &files) == nil {
				r.observeChildChanges(files, key, false)
			}
		}
	}

	var value struct {
		Kind string
		Data struct {
			Path, Revision, From, To string
			Files                    []struct {
				Path, Destination, Revision string
				Kind                        protocol.ChangeKind
				Status                      protocol.ChangeStatus
			}
		}
	}
	if json.Unmarshal(response.Result.Value, &value) != nil {
		return
	}
	data := value.Data
	if value.Kind == "file_changes" {
		for _, file := range data.Files {
			file.Path, file.Destination = key(file.Path), key(file.Destination)
			switch file.Status {
			case protocol.ChangeStatusUnknown:
				r.invalidate(file.Path)
				r.invalidate(file.Destination)
			case protocol.ChangeStatusApplied:
				switch file.Kind {
				case protocol.ChangeKindDeleted:
					r.invalidate(file.Path)
				case protocol.ChangeKindRenamed:
					r.move(file.Path, file.Destination)
				default:
					r.observeRevision(file.Path, file.Revision)
				}
			}
		}
		return
	}
	if response.Status != canonical.ToolSuccess {
		return
	}
	switch value.Kind {
	case "file_content", "file_lines", "file_written", "notebook_page":
		if data.Revision != "" {
			r.observeRevision(key(data.Path), data.Revision)
		}
	case "entry_trashed":
		r.invalidateTree(key(data.Path))
	case "entry_moved":
		// Generic entry operations may move directories across roots.
		r.invalidateTree(key(data.From))
		r.invalidateTree(key(data.To))
	}
}

func observesRevisions(operation protocol.ToolOperation) bool {
	switch operation {
	case protocol.ToolOperationApplyChanges, protocol.ToolOperationWriteFile, protocol.ToolOperationEditFile,
		protocol.ToolOperationDeleteFile, protocol.ToolOperationMoveFile, protocol.ToolOperationEditNotebook:
		return true
	}
	return false
}

// resolve fills an omitted expected_revision of a single-file mutation with
// the revision this conversation last observed, so the admitted command
// carries a concrete revision and a later external change still conflicts.
// write_file never defaults to replacing: an omitted revision creates, so its
// approval prompt, which reads the model's arguments, names the real effect.
// Other mutations require a prior read.
func (r *workspaceRevisions) resolve(operation protocol.ToolOperation, arguments json.RawMessage) (json.RawMessage, error) {
	if operation == protocol.ToolOperationApplyChanges || !observesRevisions(operation) {
		return arguments, nil
	}
	var values map[string]json.RawMessage
	if json.Unmarshal(arguments, &values) != nil || values == nil {
		// operations.Command reports malformed arguments.
		return arguments, nil
	}
	if _, ok := values["expected_revision"]; ok {
		return arguments, nil
	}
	var path string
	if json.Unmarshal(values["path"], &path) != nil {
		return arguments, nil
	}
	revision, ok := r.paths[path]
	if !ok && path != "" {
		revision, ok = r.paths[r.pathKey(path)]
	}
	switch {
	case operation == protocol.ToolOperationWriteFile:
		values["expected_revision"] = json.RawMessage("null")
	case ok && revision != "":
		values["expected_revision"], _ = json.Marshal(revision)
	default:
		return nil, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: path + " has no current observed revision; read it or pass expected_revision"}
	}
	return json.Marshal(values)
}

func (r *workspaceRevisions) pathKey(path string) string {
	if filepath.IsAbs(path) {
		path = filepath.Clean(path)
		if r.worktree != "" {
			path = localPath(r.worktree, path)
		}
		return filepath.ToSlash(path)
	}
	if path == "" {
		return ""
	}
	return pathpkg.Clean(filepath.ToSlash(path))
}

func (r *workspaceRevisions) observeRevision(path, revision string) {
	if path == "" {
		return
	}
	if revision == "" {
		r.invalidate(path)
		return
	}
	r.paths[path] = revision
	delete(r.invalidated, path)
}

func (r *workspaceRevisions) invalidate(path string) {
	if path == "" {
		return
	}
	delete(r.paths, path)
	if r.invalidated == nil {
		r.invalidated = map[string]struct{}{}
	}
	r.invalidated[path] = struct{}{}
}

func (r *workspaceRevisions) invalidateTree(path string) {
	if path == "" {
		return
	}
	r.invalidate(path)
	prefix := path + "/"
	for known := range r.paths {
		if strings.HasPrefix(known, prefix) {
			delete(r.paths, known)
		}
	}
}

func (r *workspaceRevisions) move(from, to string) {
	if from != to {
		r.observeRevision(to, r.paths[from])
		r.invalidate(from)
	}
}
