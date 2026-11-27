package engine

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"time"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
	"lukechampine.com/blake3"
)

// Tool is one captured declaration and executor. Concurrency and authority are
// properties of the captured registration, never arguments supplied by a model.
type Tool struct {
	Definition   fantasy.FunctionTool
	SchemaJSON   json.RawMessage
	MCPAuthority [32]byte
	ReadOnly     bool
	Concurrent   bool
	// FilePaths names the targets of the native executor, never of a script that rewrites its arguments.
	FilePaths   func(canonical.ToolRequest) []string
	NativeQueue bool
	// QueueArguments resolves a package's native arguments once, before
	// scheduling. PrepareQueued captures revisions later, at admission.
	QueueArguments  func(context.Context, canonical.ToolRequest) (canonical.ToolRequest, error)
	PrepareQueued   func(context.Context, canonical.ToolRequest) error
	ReleasePrepared func(canonical.ToolRequest)

	Deferred   bool
	Delegation bool
	Approval   bool
	// Confirm reports whether one prepared call of a tool without Approval
	// asks first: a read-only tool with an optional project write.
	Confirm           func(canonical.ToolRequest) bool
	Binding           *store.ApprovalBinding
	Execute           func(context.Context, canonical.ToolRequest) ToolResult
	Prepare           func(context.Context, canonical.ToolRequest) error
	ObserveHistory    func([]canonical.Record)
	ConfigureApproval func(context.Context, *sql.Conn, canonical.ToolRequest, protocol.ConversationApproval) (protocol.ConversationApproval, error)
	Approved          func(canonical.ToolRequest, protocol.ApprovalID, protocol.RequestID)
	Close             func() error
	Instruction       string
	ContextParts      []string
	BeforeModel       func(context.Context, []canonical.Record) (bool, error)
	BindDelegation    func(Tool) Tool
	// Guidance makes BindDelegation receive guide_agent instead of spawn_agent.
	Guidance bool
	// Hosted marks a package tool whose script runs with neither a bound
	// operation nor a flow, within the package's declared actions. Flow lists
	// the operations a flow tool may run. A child without a tool list keeps
	// such tools by these declarations.
	Hosted bool
	Flow   []protocol.ToolOperation
	// Exposed is the name an MCP tool is offered under when no other tool
	// owns it. A package tool is offered under its declared name.
	Exposed string
	// Display is how the turn's records show a call of a tool that no package
	// declaration describes, such as one an MCP server lists.
	Display *protocol.ToolDisplay
	// SearchLabels are the captured display labels and translations used only
	// for discovery. They do not enlarge the model's tool declarations.
	SearchLabels []string
	schema       *jsonschema.Schema
	// ExecutePrepared runs a call with arguments a package prepared. The call
	// keeps the identity the model sent; the arguments are what executes.
	ExecutePrepared func(context.Context, canonical.ToolRequest, json.RawMessage) ToolResult
}
type ToolResult struct {
	Collect         *collectedChild
	Meta            *map[string]json.RawMessage
	ReturnedFailure *protocol.ToolFailure
	ReturnedError   bool
	Value           any
	Format          canonical.ToolFormat
	Error           bool
	Failure         *protocol.Fault
	Context         *canonical.Context
	// Attachments are stored assets shown with the result and never sent to
	// the model.
	Attachments  []canonical.Attachment
	modelFailure model.FailureKind
}

// WithModelFailure retains the classification of a direct model request for
// tool diagnostics, without changing the tool response kept in history.
func (result ToolResult) WithModelFailure(failure *model.Failure) ToolResult {
	if failure != nil && !failure.RequestObserved() {
		result.modelFailure = failure.Kind
	}
	return result
}

// A retry reconstructs executors from its own turn; only declarations with the
// same binding, schema, approval behavior and MCP authority may be reused.
func delegatedToolDigest(tool Tool) ([32]byte, error) {
	body, err := json.Marshal([]any{tool.Binding, tool.Definition.InputSchema, tool.MCPAuthority, tool.Flow, tool.ReadOnly, tool.Hosted, tool.Approval, tool.Confirm != nil})
	if err != nil {
		return [32]byte{}, err
	}
	return blake3.Sum256(body), nil
}

func inheritableChildTool(tool Tool) bool {
	if tool.Delegation || tool.BindDelegation != nil {
		return false
	}
	if tool.Binding == nil && slices.Contains(catalogToolNames, tool.Definition.Name) {
		return false
	}
	if slices.ContainsFunc(declaredOperations(tool), func(op protocol.ToolOperation) bool {
		switch op {
		case protocol.ToolOperationListRoles, protocol.ToolOperationSaveRole, protocol.ToolOperationRemoveRole, protocol.ToolOperationApplyRoles:
			return true
		}
		return false
	}) {
		return false
	}
	return tool.Binding != nil || !slices.Contains([]string{"tool_search", "ask_user", "set_session_title", "list_subagents"}, tool.Definition.Name)
}

func encodeDelegatedTools(tools []Tool) ([]byte, error) {
	allowed := make(map[string][32]byte, len(tools))
	for _, tool := range tools {
		if !inheritableChildTool(tool) {
			continue
		}
		name := tool.Definition.Name
		if _, ok := allowed[name]; ok {
			continue
		}
		digest, err := delegatedToolDigest(tool)
		if err != nil {
			return nil, err
		}
		allowed[name] = digest
	}
	return json.Marshal(allowed)
}

func (r *Runtime) captureTools(x *execution) error {
	x.tools = append([]Tool(nil), r.tools...)
	if r.toolsForTurn != nil {
		scoped, err := r.toolsForTurn(x.ctx, x.inv)
		if err != nil {
			return err
		}
		x.tools = append(x.tools, scoped...)
	}
	builtins := r.builtins(x)
	if x.inv.Session.Delegation != nil {
		// The title and the question belong to the conversation the user
		// reads. A child has neither, so a role cannot select them.
		builtins = slices.DeleteFunc(builtins, func(tool Tool) bool {
			return tool.Definition.Name == "set_session_title" || tool.Definition.Name == "ask_user"
		})
	}
	x.tools = append(builtins, x.tools...)
	for index := range x.tools {
		if bind := x.tools[index].BindDelegation; bind != nil {
			name := "spawn_agent"
			if x.tools[index].Guidance {
				name = "guide_agent"
			}
			if delegate := findTool(builtins, name); delegate != nil {
				x.tools[index] = bind(*delegate)
			}
		}
	}
	x.tools = slices.DeleteFunc(x.tools, func(tool Tool) bool { return tool.BindDelegation != nil && !tool.Delegation })
	for _, tool := range x.tools {
		if tool.Close != nil {
			x.toolCleanup = append(x.toolCleanup, tool.Close)
		}
	}
	if x.inv.Session.Delegation != nil && *x.inv.Session.Delegation != nil {
		if x.inv.RetryTools != nil {
			var allowed map[string][32]byte
			if err := json.Unmarshal(x.inv.RetryTools, &allowed); err != nil {
				return err
			}
			x.tools = slices.DeleteFunc(x.tools, func(tool Tool) bool {
				want, ok := allowed[tool.Definition.Name]
				got, err := delegatedToolDigest(tool)
				return !ok || err != nil || got != want
			})
		}
		if x.childRole != nil && x.childRole.Tools != nil {
			if len(missingRoleTools(*x.childRole, x.inv.Turn.Config.Mode, modelSupportsTools(x.inv.Provider, x.inv.Turn.Config.Model), x.tools)) != 0 {
				return &protocol.Fault{Code: protocol.ErrorCodeNotConfigured, Message: "role references an unavailable tool"}
			}
		}
		x.tools = slices.DeleteFunc(x.tools, func(tool Tool) bool { return !inheritableChildTool(tool) || !roleAllowsTool(x.childRole, tool) })
	}
	if err := r.deferTools(x); err != nil {
		return err
	}
	if err := r.applyAssistant(x); err != nil {
		return err
	}
	if spawn := findTool(x.tools, "spawn_agent"); spawn != nil {
		spawn.Definition.Description += "\n" + r.roleRoster(x)
	}
	if search := r.builtinSearch(x); search != nil {
		x.tools = append(x.tools, *search)
	}
	for i := range x.tools {
		tool := &x.tools[i]
		compiler := jsonschema.NewCompiler()
		compiler.UseLoader(jsonschema.SchemeURLLoader{})
		location := "urn:sailry:tool:" + tool.Definition.Name
		if err := compiler.AddResource(location, tool.Definition.InputSchema); err != nil {
			return err
		}
		schema, err := compiler.Compile(location)
		if err != nil {
			return err
		}
		tool.schema = schema
	}
	return nil
}

func (r *Runtime) closeTools(x *execution) error {
	cleanup := x.toolCleanup
	x.toolCleanup = nil
	var failures []error
	panicked := false
	for _, close := range cleanup {
		failures = append(failures, func() (err error) {
			defer func() {
				if value := recover(); value != nil {
					panicked = true
					logPanic(x, "tool_cleanup", "", value)
					err = &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "tool resource cleanup failed unexpectedly"}
				}
			}()
			return close()
		}())
	}
	if errors.Join(failures...) != nil {
		fault := &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "tool resource cleanup could not be confirmed"}
		if panicked {
			return &observedTurnPanic{fault}
		}
		return fault
	}
	return nil
}

// deferTools keeps the everyday worktree capabilities on the first request
// and leaves every other captured tool discoverable, as the source selection
// does. A child whose role lists its tools defers nothing. Until here
// Deferred carries the declaration: a package tool that
// is not deferred declared itself initial.
func (r *Runtime) deferTools(x *execution) error {
	child := x.inv.Session.Delegation != nil
	if child && x.childRole != nil && x.childRole.Tools != nil {
		for index := range x.tools {
			x.tools[index].Deferred = false
		}
		return nil
	}
	selected := ""
	if x.resources != nil {
		mode, err := x.resources.SelectedMode(x.inv.Turn.Config)
		if err != nil {
			return err
		}
		if mode != nil {
			selected = mode.Package.Name
		}
	}
	role := selectedRole(x.inv.Message) != nil
	for index := range x.tools {
		tool := &x.tools[index]
		switch {
		case tool.Delegation && role:
			tool.Deferred = false
		case tool.Binding == nil || tool.Binding.Owner.Name == "":
			// The Node's own tools state their rule where they are built.
			// The session history tools are discoverable.
			if slices.Contains([]string{"list_sessions", "read_session", "search_session"}, tool.Definition.Name) {
				tool.Deferred = true
			}
		default:
			// A declared initial tool starts visible only to the main agent.
			tool.Deferred = !(tool.Binding.Owner.Name == selected || visibleTool(*tool) || !tool.Deferred && !child)
		}
	}
	return nil
}

// declaredOperations lists the operations a package tool declares: its bound
// operation and the operations of its flow.
func declaredOperations(tool Tool) []protocol.ToolOperation {
	declared := tool.Flow
	if tool.Binding != nil && tool.Binding.Operation != nil {
		declared = append([]protocol.ToolOperation{*tool.Binding.Operation}, declared...)
	}
	return declared
}

// visibleTool reports whether every operation a tool declares starts visible.
func visibleTool(tool Tool) bool {
	declared := declaredOperations(tool)
	return len(declared) != 0 && !slices.ContainsFunc(declared, func(operation protocol.ToolOperation) bool { return !visibleOperation(operation) })
}

// visibleOperation names the operations offered on the first request.
func visibleOperation(operation protocol.ToolOperation) bool {
	switch operation {
	case protocol.ToolOperationReadFile, protocol.ToolOperationReadFileLines, protocol.ToolOperationListDirectory, protocol.ToolOperationFindFiles, protocol.ToolOperationSearchFiles,
		protocol.ToolOperationWriteFile, protocol.ToolOperationEditFile, protocol.ToolOperationApplyChanges,
		protocol.ToolOperationSearchCode, protocol.ToolOperationOutlineCode, protocol.ToolOperationGraphCode,
		protocol.ToolOperationInspectGit, protocol.ToolOperationReadGitDiff,
		protocol.ToolOperationRunCommand, protocol.ToolOperationReadCommand, protocol.ToolOperationStopCommand,
		protocol.ToolOperationFetchWeb, protocol.ToolOperationSearchWeb,
		protocol.ToolOperationProgress, protocol.ToolOperationListRoles, protocol.ToolOperationListArtifactRevisions:
		return true
	}
	return false
}

// deferredContext lists the names of the tools a turn starts without, so the
// model knows what discovery can load. The list is fixed for the turn.
func (r *Runtime) deferredContext(x *execution, state store.AgentState) string {
	if x.deferredContext == nil {
		names := r.exposure(x, state)
		var deferred []string
		for _, tool := range x.tools {
			if tool.Deferred && !slices.Contains(state.LoadedTools, tool.Definition.Name) {
				deferred = append(deferred, names.name(tool.Definition.Name))
			}
		}
		slices.Sort(deferred)
		text := ""
		if len(deferred) != 0 {
			text = "Deferred tools\n" + strings.Join(deferred, ", ")
		}
		x.deferredContext = &text
	}
	return *x.deferredContext
}

func (r *Runtime) declarations(x *execution, binding model.Binding, state store.AgentState) []fantasy.FunctionTool {
	var values []fantasy.FunctionTool
	if !capturedModel(binding).Tools {
		return values
	}
	// The request renames these canonical declarations at the model boundary.
	r.exposure(x, state)
	for _, tool := range x.tools {
		if !tool.Deferred || slices.Contains(state.LoadedTools, tool.Definition.Name) {
			values = append(values, tool.Definition)
		}
	}
	return values
}
func findTool(tools []Tool, name string) *Tool {
	for i := range tools {
		if tools[i].Definition.Name == name {
			return &tools[i]
		}
	}
	return nil
}

// Package-owned native commands reach the workspace launcher through this
// approval binding. A tool name or a hosted script alone grants no exception.
func isPlanRunCommand(tool *Tool) bool {
	return tool != nil && tool.Approval && tool.ConfigureApproval != nil && tool.Prepare != nil &&
		tool.Binding != nil && tool.Binding.Operation != nil && *tool.Binding.Operation == protocol.ToolOperationRunCommand
}

func (r *Runtime) runTools(x *execution, state store.AgentState, calls []canonical.ToolRequest) error {
	// Discovery answers with the names this turn offers its tools under.
	r.exposure(x, state)
	results := make([]ToolResult, len(calls))
	executable := make([]bool, len(calls))
	startedCalls := make([]bool, len(calls))
	asks := make([]bool, len(calls))
	var marks []canonical.Record
	var historyLimit int
	x.observedMu.Lock()
	if x.observed.Tools == nil {
		x.observed.Tools = map[string]int{}
	}
	for _, call := range calls {
		x.observed.Tools[call.Name]++
	}
	x.observedMu.Unlock()
	inputs := slices.Clone(calls)
	deferPrepare := func(tool *Tool) bool {
		if tool.Confirm != nil && !tool.Approval {
			return false
		}
		return tool.FilePaths != nil || tool.ObserveHistory != nil || tool.Binding != nil && tool.Binding.Operation != nil && strings.HasPrefix(string(*tool.Binding.Operation), "files.")
	}
	prepare := func(i int, tool *Tool) error {
		if tool.PrepareQueued != nil {
			return tool.PrepareQueued(x.ctx, inputs[i])
		}
		if tool.Prepare != nil {
			return tool.Prepare(x.ctx, inputs[i])
		}
		return nil
	}
	// Mark every allowed call before requesting the first approval.
	for i, call := range calls {
		tool := findTool(x.tools, call.Name)
		if call.Error != nil {
			results[i] = ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: call.Error.Message}}
			continue
		}
		// A captured deferred tool called by name runs under its own approval:
		// the saved request already loaded it, as in the Rust catalog.
		if tool == nil {
			results[i] = ToolResult{Error: true, Value: map[string]any{"content": []any{map[string]any{"type": "text", "text": "This tool call was not handled."}}, "isError": true}}
			continue
		}
		// Only a parent planning turn can delegate, ask or record an artifact.
		child := x.inv.Session.Delegation != nil
		planning := x.inv.Turn.Config.Mode == protocol.WorkModePlan
		readOnlyRole := child && x.childRole != nil && x.childRole.ReadOnly
		parentException := !child && (tool.Delegation || call.Name == "ask_user" ||
			tool.Binding != nil && tool.Binding.Operation != nil && *tool.Binding.Operation == protocol.ToolOperationWriteArtifact)
		if (planning || readOnlyRole) && !tool.ReadOnly && !parentException && !(planning && isPlanRunCommand(tool)) {
			results[i] = ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "planning mode permits read-only tools"}}
			continue
		}
		var arguments any
		if err := json.Unmarshal(call.Arguments, &arguments); err != nil {
			results[i] = invalidTool("invalid tool arguments")
			continue
		}
		if err := tool.schema.Validate(arguments); err != nil {
			if validation, ok := errors.AsType[*jsonschema.ValidationError](err); ok {
				orderValidationErrors(validation)
			}
			results[i] = invalidTool(fmt.Sprintf("invalid tool arguments: %s", err))
			continue
		}
		if tool.ReleasePrepared != nil {
			defer tool.ReleasePrepared(call)
		}
		if tool.QueueArguments != nil {
			input, err := tool.QueueArguments(x.ctx, call)
			if err != nil {
				results[i] = toolFault(err)
				continue
			}
			inputs[i] = input
		}
		if deferPrepare(tool) {
			if tool.ObserveHistory != nil {
				historyLimit = i
			}
			asks[i] = tool.Approval
			if !asks[i] {
				executable[i] = true
				marks = append(marks, r.executableRecord(x, call, true))
			}
			continue
		}
		if tool.ObserveHistory != nil {
			tool.ObserveHistory(state.Records)
		}
		if tool.Prepare != nil {
			if err := prepare(i, tool); err != nil {
				results[i] = toolFault(err)
				continue
			}
		}
		if asks[i] = tool.Approval || tool.Confirm != nil && tool.Confirm(call); !asks[i] {
			executable[i] = true
			marks = append(marks, r.executableRecord(x, call, true))
		}
	}
	if err := r.save(x.ctx, x.inv.Session.ID, marks...); err != nil {
		return err
	}
	// Earlier writes may finish out of order. Preparation sees completed
	// predecessors in the same order as their eventual canonical receipts.
	receipts := make([]canonical.Record, historyLimit)
	completed := make([]bool, historyLimit)
	recordCompletion := func(i int) {
		if i >= historyLimit {
			return
		}
		completed[i] = true
		if results[i].Value == nil && results[i].Failure == nil {
			return
		}
		rec := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.User, responseBlock(uint64(i), calls[i], results[i])), r.clock.Now())
		rec.ID = "batch:" + calls[i].ID
		rec.Provenance.Author = "tool"
		receipts[i] = rec
	}
	prefix := 0
	history := state.Records[:len(state.Records):len(state.Records)]
	historyBefore := func(i int) []canonical.Record {
		// An unadmitted call is incomplete, so the prefix cannot pass i
		// even when an independent later call was admitted first.
		for prefix < i && completed[prefix] {
			if receipts[prefix].ID != "" {
				history = append(history, receipts[prefix])
			}
			prefix++
		}
		visible := history[:len(history):len(history)]
		for j := prefix + 1; j < i; j++ {
			if receipts[j].ID != "" {
				visible = append(visible, receipts[j])
			}
		}
		return visible
	}
	admit := func(i int) error {
		call := calls[i]
		tool := findTool(x.tools, call.Name)
		if !asks[i] && !executable[i] || results[i].Value != nil || results[i].Failure != nil {
			return nil
		}
		if deferPrepare(tool) {
			if tool.ObserveHistory != nil {
				tool.ObserveHistory(historyBefore(i))
			}
			if tool.Prepare != nil {
				if err := prepare(i, tool); err != nil {
					executable[i] = false
					results[i] = toolFault(err)
					return nil
				}
			}
		}
		if !asks[i] {
			return nil
		}
		approved, err := r.approval(x, call, tool)
		var denied *protocol.Fault
		if x.inv.Turn.Config.Mode == protocol.WorkModePlan && isPlanRunCommand(tool) && errors.As(err, &denied) && denied.Code == protocol.ErrorCodePermissionDenied {
			results[i] = toolFault(err)
			return nil
		}
		if err != nil {
			return err
		}
		executable[i] = approved
		if !approved {
			results[i] = ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodePermissionDenied, Message: "Tool call was denied"}}
		}
		if err = r.save(x.ctx, x.inv.Session.ID, r.executableRecord(x, call, approved)); err != nil {
			return err
		}
		return nil
	}
	execute := func(i int) {
		if !executable[i] {
			return
		}
		metric, started := r.telemetry.Capture(), time.Now()
		defer func() { observeTool(metric, x.ctx, x.tools, calls[i].Name, results[i], started) }()
		// A panicking tool fails its own call; the turn and the Node continue.
		containPanic := func() {
			if value := recover(); value != nil {
				logPanic(x, "tool", calls[i].ID, value)
				results[i] = ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "Tool call failed unexpectedly"}}
			}
		}
		defer containPanic()
		if err := x.hooks.Before(x.ctx, calls[i].Name); err != nil {
			if x.ctx.Err() == nil {
				results[i] = toolFault(err)
			}
			return
		}
		if x.ctx.Err() != nil {
			return
		}
		tool := findTool(x.tools, calls[i].Name)
		defer func() {
			x.hooks.After(x.ctx, calls[i].Name, results[i].Failure == nil && !results[i].Error && !results[i].ReturnedError)
		}()
		defer containPanic()
		defer x.timeline.tool(calls[i], tool.Delegation)()
		startedCalls[i] = true
		results[i] = tool.Execute(x.ctx, calls[i])
	}
	concurrent := func(i int) bool {
		tool := findTool(x.tools, calls[i].Name)
		return !asks[i] && (!executable[i] || tool != nil && tool.ReadOnly && tool.Concurrent && !x.hooks.WrapsTools())
	}
	access := make([]toolAccess, len(calls))
	for i, call := range inputs {
		if (asks[i] || executable[i]) && !x.hooks.WrapsTools() {
			access[i] = parallelToolAccess(findTool(x.tools, call.Name), call)
		}
	}
	var batchErr error
	for start := 0; start < len(calls) && x.ctx.Err() == nil; {
		end := start + 1
		reads, mixed := concurrent(start), access[start].parallel
		for end < len(calls) {
			reads = reads && concurrent(end)
			mixed = mixed && access[end].parallel
			if !reads && !mixed {
				break
			}
			end++
		}
		batchErr = runQueuedTools(x.ctx, start, access[start:end], admit, execute, recordCompletion)
		if batchErr != nil {
			break
		}
		start = end
	}
	blocks := make([]canonical.Block, 0, len(calls))
	var contextDelta canonical.Context
	for i, call := range calls {
		if startedCalls[i] && (results[i].Failure != nil || results[i].ReturnedFailure != nil) {
			recordToolFailure(x.ctx, findTool(x.tools, call.Name), results[i])
		}
		if (executable[i] || batchErr != nil || x.ctx.Err() != nil) && results[i].Value == nil && results[i].Failure == nil && (!results[i].Error || x.ctx.Err() != nil) {
			results[i] = interruptedTool(startedCalls[i])
		}
		blocks = append(blocks, responseBlock(uint64(i), call, results[i]))
		if delta := results[i].Context; delta != nil {
			if delta.Title != nil {
				contextDelta.Title = delta.Title
			}
			if delta.Policy != nil {
				contextDelta.Policy = delta.Policy
			}
			if delta.LoadedTools != nil {
				if contextDelta.LoadedTools == nil {
					names := []string{}
					contextDelta.LoadedTools = &names
				}
				for _, name := range *delta.LoadedTools {
					if !slices.Contains(*contextDelta.LoadedTools, name) {
						*contextDelta.LoadedTools = append(*contextDelta.LoadedTools, name)
					}
				}
			}
		}
	}
	if contextDelta.LoadedTools != nil && state.Context.Policy != nil {
		policy := *state.Context.Policy
		if contextDelta.Policy != nil {
			policy = *contextDelta.Policy
		}
		if addition := capabilityInstructions(x, policy, *contextDelta.LoadedTools); addition != "" {
			policy += "\n\n" + addition
			contextDelta.Policy = &policy
		}
	}
	record := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.User, blocks...), r.clock.Now())
	record.Provenance.Author = "tool"
	for i, result := range results {
		if result.ReturnedError {
			if record.Metadata.Presentation == nil {
				record.Metadata.Presentation = &canonical.Presentation{}
			}
			record.Metadata.Presentation.Failures = append(record.Metadata.Presentation.Failures, canonical.ToolFailure{Index: uint64(i), ID: calls[i].ID, Name: calls[i].Name, Failure: returnedToolFailure(result)})
		}
	}
	if contextDelta.Title != nil || contextDelta.LoadedTools != nil || contextDelta.Policy != nil {
		record.Metadata.Context = &contextDelta
	}
	if err := r.save(context.WithoutCancel(x.ctx), x.inv.Session.ID, record); err != nil {
		return err
	}
	for _, result := range results {
		if result.Collect != nil {
			if err := r.consumeChild(x, result.Collect.session, &result.Collect.turn); err != nil {
				return err
			}
		}
	}
	return batchErr
}

// Report only evidence from an executed native tool. Preparation and hook
// refusals, interrupted calls, and package or MCP results are not that evidence.
func recordToolFailure(step context.Context, tool *Tool, result ToolResult) {
	if step.Err() != nil || tool == nil || tool.Binding != nil && tool.Binding.Owner.Name != "" || tool.MCPAuthority != ([32]byte{}) {
		return
	}
	var code protocol.ErrorCode
	var cause string
	if fault := result.Failure; fault != nil {
		if fault.ReportOwned {
			return
		}
		switch fault.Code {
		case protocol.ErrorCodeInternal:
			code = fault.Code
		case protocol.ErrorCodeUnavailable:
			switch result.modelFailure {
			case model.Network, model.Unavailable, model.InvalidResponse:
				code, cause = fault.Code, "model_"+string(result.modelFailure)
			}
		}
	} else if failure := result.ReturnedFailure; result.ReturnedError && failure != nil && failure.Origin == protocol.ToolFailureOriginRuntime &&
		failure.Code != nil && *failure.Code == protocol.ErrorCodeOutcomeUnknown && tool.Binding != nil && tool.Binding.Operation != nil &&
		*tool.Binding.Operation == protocol.ToolOperationRunCommand {
		// run_command returns this outcome only for a foreground completion.
		// read_command and stop_command can replay a background snapshot.
		code, cause = *failure.Code, "foreground_command"
	}
	if code != "" {
		logging.Emit(slog.LevelError, slog.String("area", "agent"), slog.String("operation", "tool_execution"),
			slog.String("phase", logging.Failed), slog.String("code", string(code)), slog.String("cause_code", cause))
	}
}

func (r *Runtime) executableRecord(x *execution, call canonical.ToolRequest, allowed bool) canonical.Record {
	patch, _ := json.Marshal(map[string]bool{canonical.ExecutableKey: allowed})
	record := canonical.NewRecord(x.inv.Run.Turn, "operation", canonical.Operation{Kind: "tool_meta", Data: &canonical.ToolMeta{Call: call.ID, Patch: patch}}, r.clock.Now())
	record.Provenance.Author = "assistant"
	return record
}
func responseBlock(index uint64, call canonical.ToolRequest, result ToolResult) canonical.Block {
	value := result.Value
	status := canonical.ToolSuccess
	format := result.Format
	if result.ReturnedError {
		status = canonical.ToolErrorStatus
	}
	if result.Error {
		status = canonical.ToolErrorStatus
		format = canonical.MCP
	}
	if result.Failure != nil {
		status = canonical.ToolErrorStatus
		format = canonical.MCPError
		value = map[string]any{"code": -32000, "message": result.Failure.Message, "data": result.Failure}
	}
	if value == nil {
		value = map[string]any{}
	}
	raw, _ := json.Marshal(value)
	var normalized any
	if json.Unmarshal(raw, &normalized) == nil {
		raw, _ = json.Marshal(normalized)
	}
	sum := blake3.Sum256(raw)
	return canonical.Block{Index: index, Kind: "tool_response", Data: &canonical.ToolResponse{ID: call.ID, Name: call.Name, Status: status, Format: format, Result: canonical.Payload{Value: raw}, Revision: fmt.Sprintf("%x", sum), Provider: call.Provider, Meta: result.Meta, Attachments: result.Attachments}}
}

func interruptedTool(mayHaveStarted bool) ToolResult {
	if !mayHaveStarted {
		return ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodeCancelled, Message: "Tool call did not start before the turn ended"}}
	}
	return ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodeOutcomeUnknown, Message: "Tool call was interrupted before completing; inspect the original operation before retrying"}}
}

// settleCalls records an interrupted result for every call the stopped or
// failed turn left unanswered, as the source finalization does.
func (r *Runtime) settleCalls(ctx context.Context, x *execution) error {
	state, err := r.load(ctx, x.inv.Session.ID)
	if err != nil {
		return err
	}
	admitted := make(map[string]bool)
	calls := pendingCalls(state.Records, admitted)
	if len(calls) == 0 {
		return nil
	}
	blocks := make([]canonical.Block, 0, len(calls))
	for _, call := range calls {
		blocks = append(blocks, responseBlock(uint64(len(blocks)), call, interruptedTool(admitted[call.ID])))
	}
	record := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.User, blocks...), r.clock.Now())
	record.Provenance.Author = "tool"
	return r.save(ctx, x.inv.Session.ID, record)
}

// logPanic records a recovered panic. A log field is one bounded line, so the
// stack is reduced to its innermost frames.
func logPanic(x *execution, operation, call string, value any) {
	var frames []string
	counters := make([]uintptr, 32)
	trace := runtime.CallersFrames(counters[:runtime.Callers(2, counters)])
	for {
		frame, more := trace.Next()
		if strings.HasPrefix(frame.Function, "runtime.") {
			frames = frames[:0]
		} else if len(frames) < 6 {
			frames = append(frames, frame.Function[strings.LastIndexByte(frame.Function, '/')+1:]+":"+strconv.Itoa(frame.Line))
		}
		if !more {
			break
		}
	}
	logging.Emit(slog.LevelError, slog.String("area", "agent"), slog.String("phase", logging.Failed), slog.String("operation", operation),
		slog.String("session", x.inv.Session.ID.String()), slog.String("turn", x.inv.Run.Turn.String()), slog.String("call", call),
		slog.String("exception", fmt.Sprint(value)), slog.String("detail", strings.Join(frames, " < ")))
}

func invalidTool(message string) ToolResult {
	return ToolResult{Failure: &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: message}}
}

func orderValidationErrors(err *jsonschema.ValidationError) {
	for _, cause := range err.Causes {
		orderValidationErrors(cause)
	}
	switch detail := err.ErrorKind.(type) {
	case *kind.AdditionalProperties:
		slices.Sort(detail.Properties)
	case *kind.Schema, *kind.Group, *kind.Reference, *kind.PropertyNames:
		// These groups can collect causes in map order. Combinators keep their declared branch order.
		slices.SortFunc(err.Causes, func(a, b *jsonschema.ValidationError) int {
			if order := slices.Compare(a.InstanceLocation, b.InstanceLocation); order != 0 {
				return order
			}
			return strings.Compare(a.Error(), b.Error())
		})
	}
}

func jsonTool(name, description, schema string, readOnly, concurrent bool, execute func(context.Context, canonical.ToolRequest) ToolResult) Tool {
	var input map[string]any
	if err := json.Unmarshal([]byte(schema), &input); err != nil {
		panic(err)
	}
	return Tool{Definition: fantasy.FunctionTool{Name: name, Description: description, InputSchema: input}, SchemaJSON: json.RawMessage(schema), ReadOnly: readOnly, Concurrent: concurrent, Execute: execute}
}
func returnedToolFailure(result ToolResult) protocol.ToolFailure {
	if result.ReturnedFailure != nil {
		return *result.ReturnedFailure
	}
	return protocol.ToolFailure{Origin: protocol.ToolFailureOriginTool}
}
