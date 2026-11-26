// Adapted from Goose ee8500d5 and 14741e9a (Apache-2.0).
// See ../model/LICENSE.goose.
package engine

import (
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"slices"
	"strings"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	handoff "github.com/ai-poet/cheaprouter-harness/internal/node/agent/compact"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/model"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/output"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/prompt"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/tokens"
	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func capturedModel(binding model.Binding) protocol.ConversationModel {
	selected, _ := model.ResolvedModel(binding)
	return selected
}

func contextTokens(records []canonical.Record) uint64 {
	start := 0
	for i, record := range records {
		if record.Kind == "compaction" {
			start = i
		}
	}
	for i := len(records) - 1; i >= start; i-- {
		message := records[i].MutableMessage()
		if message != nil && message.Usage != nil {
			if !compactionUsage(records[i]) && message.Usage.Total != nil {
				used := *message.Usage.Total
				if message.Usage.Input != nil && message.Usage.Output != nil && message.Usage.Reasoning != nil &&
					!hasSignedReasoning(message) && *message.Usage.Reasoning <= *message.Usage.Output {
					// Reasoning tokens that are not replayed back into the next request do not consume the next prompt window.
					used = *message.Usage.Input + (*message.Usage.Output - *message.Usage.Reasoning)
				}
				return used + pendingTokens(records[i+1:])
			}
			// Only the latest observation can describe the current context. An
			// incomplete report or summary usage cannot reuse an older total.
			break
		}
	}
	// Retained messages can precede the checkpoint in storage. Their old
	// measurements describe the pre-compaction request, but their content
	// still belongs to the new context alongside the latest summary.
	return pendingTokens(records)
}

func hasSignedReasoning(message *canonical.Message) bool {
	for _, block := range message.Content {
		switch block.Kind {
		case "redacted_thinking":
			return true
		case "thinking":
			if thinking, ok := block.Data.(*canonical.Thinking); ok && thinking.Signature != nil && *thinking.Signature != "" {
				return true
			}
			if thinking, ok := block.Data.(canonical.Thinking); ok && thinking.Signature != nil && *thinking.Signature != "" {
				return true
			}
		}
	}
	return false
}
func pendingTokens(records []canonical.Record) uint64 {
	var cache output.Cache
	var count uint64
	for _, record := range cache.Project(handoff.Pending(records)) {
		count += tokens.Chat("", []canonical.Record{record})
	}
	return count
}
func compactionUsage(record canonical.Record) bool {
	return record.Kind == "compaction" || record.Metadata.Execution != nil && record.Metadata.Execution.Purpose != nil && *record.Metadata.Execution.Purpose == canonical.PurposeCompaction
}

// shouldCompact follows the model's window and compaction budget on every
// route. An agent program's prompt bound limits only a whole-history replay;
// one that exceeds it fails as a context overflow, which compaction recovers.
func shouldCompact(binding model.Binding, state store.AgentState, percent uint8) bool {
	last := lastMessage(state.Records)
	if last == nil || last.Role == canonical.Assistant {
		return false
	}
	selected, _ := model.ResolvedModel(binding)
	limit := uint64(selected.Context)
	budget := store.AgentCompactionBudget(selected, percent)
	if budget == nil || budget.TriggerTokens == 0 || budget.TriggerTokens >= limit {
		return false
	}
	count := contextTokens(state.Records)
	return count > budget.TriggerTokens && automaticFailures(binding, state.Records) < pauseAfter
}

// pauseAfter is how many automatic compactions in a row may fail before the
// threshold stops starting one: each costs a summary request and its retries.
const pauseAfter = 3

// automaticFailures counts the automatic compactions that failed in a row for
// this provider and model. A compaction that succeeded, manual or automatic,
// starts the count again, and so does a request another provider or model
// answered. The count is read from the records, so it survives a restart.
func automaticFailures(binding model.Binding, records []canonical.Record) int {
	count := 0
	for _, record := range records {
		if record.Kind == "compaction" {
			count = 0
			continue
		}
		execution := record.Metadata.Execution
		if execution != nil && execution.Source != nil && execution.Media == nil && (execution.Purpose == nil || *execution.Purpose != canonical.PurposeSearch) &&
			(execution.Source.ID != binding.Provider.ID || execution.Source.Model != binding.Config.Model) {
			count = 0
			continue
		}
		if maintenance := record.Metadata.Compaction; maintenance != nil && maintenance.Failure != nil && maintenance.Failure.Source == protocol.ConversationCompactionSourceAutomatic {
			count++
		}
	}
	return count
}

// compactionFailure is the conversation's record of a compaction that failed.
// Clients show it as their own localized notice with the cause.
func (r *Runtime) compactionFailure(x *execution, binding model.Binding, earlier []canonical.Record, manual bool, stage protocol.ConversationCompactionStage, reason protocol.ConversationCompactionReason, attempts []model.Attempt, failure error) canonical.Record {
	detail := protocol.DefaultConversationCompactionFailure()
	detail.Source, detail.Stage, detail.Reason = protocol.ConversationCompactionSourceAutomatic, stage, reason
	if manual {
		detail.Source = protocol.ConversationCompactionSourceManual
	}
	if len(attempts) > 0 {
		detail.Status = attempts[len(attempts)-1].Status
	}
	var cause string
	var fault *protocol.Fault
	if errors.As(failure, &fault) {
		cause = fault.Message
	} else {
		cause = model.Classify(failure).Message
	}
	detail.Cause = &cause
	record := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.Assistant), r.clock.Now())
	record.Provenance.Author = "system"
	runtime := canonical.DefaultRuntime()
	runtime.AgentVisible = false
	record.Metadata.Runtime = &runtime
	// The failure names its provider and model, which the pause counts by. The
	// one that reaches the limit says the pause once.
	scope := model.Scope(binding)
	record.Metadata.Execution = &canonical.Execution{Source: &scope}
	detail.Paused = !manual && automaticFailures(binding, earlier) == pauseAfter-1
	record.Metadata.Compaction = &canonical.Maintenance{Failure: &detail}
	return record
}
func (r *Runtime) compact(x *execution, binding model.Binding, route model.Route, state store.AgentState, manual bool) error {
	selected, _ := model.ResolvedModel(binding)
	// The compacted history reaches the model whole, so its handoff and
	// retained tail are sized for the window a replay can fill.
	window := route.ReplayWindow(selected.Context)
	keepRecent := true
	if last := lastMessage(state.Records); last != nil && last.Error != nil && last.Error.Code != nil && *last.Error.Code == "context_overflow" {
		keepRecent = false
	}
	selection, useful := handoff.Select(state.Records, uint64(window), keepRecent)
	if manual && !useful {
		// An explicit request can summarize a short conversation whose entire
		// history would otherwise fit in the retained tail.
		selection, useful = handoff.Select(state.Records, uint64(window), false)
	}
	if !useful {
		return nil
	}
	return r.summarize(x, binding, route, state, compactionPlan{selection: selection, manual: manual, window: window, native: keepRecent && !route.Session && route.Native != "" && (!manual || strings.TrimSpace(x.inv.Message.Text) == "")})
}

type compactionPlan struct {
	selection handoff.Selection
	manual    bool
	window    uint32
	native    bool
	carry     *canonical.NativeCompaction
}

// handoffFailure means the summary failed without changing the checkpoint.
// A threshold is advisory: the next normal request can still use the history.
type handoffFailure struct{ error }

func (e handoffFailure) Unwrap() error { return e.error }

func (r *Runtime) summarize(x *execution, binding model.Binding, route model.Route, state store.AgentState, plan compactionPlan) error {
	selection, manual, window := plan.selection, plan.manual, plan.window
	selected := capturedModel(binding)
	summaryID := protocol.NewRequestID().String()
	hideCompacting := func() {
		if !manual {
			r.feed.Publish(x.inv.Session.ID, protocol.ConversationChange{
				Kind: "hidden",
				Data: protocol.ConversationChangeHiddenPayload{ID: summaryID, Turn: x.inv.Run.Turn},
			})
		}
	}
	if !manual {
		progress, _ := json.Marshal(map[string]any{"type": "compacting", "started_ms": r.clock.Now().UnixMilli()})
		_ = r.feed.Delta(x.inv.Session.ID, protocol.ConversationDraft{ID: summaryID, Turn: x.inv.Run.Turn, Author: "sailry", Parts: []protocol.ConversationPart{{Kind: "resource", Data: json.RawMessage(progress)}}})
	}
	var err error
	binding, err = r.credential(x, binding)
	if err != nil {
		hideCompacting()
		return err
	}
	summaryBinding := model.Summary(binding)
	if route.Session {
		// A transcript summary runs in an agent session of its own, so it
		// neither replaces nor resumes the session that holds the history.
		summaryBinding.Session = protocol.SessionID{}
	}
	output := min(int64(model.SummaryOutputLimit(selected)), int64(handoff.Output(uint64(window))))
	requestText := handoff.Request
	if manual && strings.TrimSpace(x.inv.Message.Text) != "" {
		requestText += "\n\nUser focus for the handoff:\n" + x.inv.Message.Text
	}
	var attempts []model.Attempt
	// A compaction that ends without a summary leaves its billed attempts and
	// one record saying where and why it stopped; a stopped turn is no failure.
	failed := func(stage protocol.ConversationCompactionStage, reason protocol.ConversationCompactionReason, failure error) error {
		hideCompacting()
		var records []canonical.Record
		for _, attempt := range attempts {
			records = append(records, r.observationRecord(x, summaryBinding, canonical.PurposeCompaction, attempt))
		}
		if x.ctx.Err() == nil {
			records = append(records, r.compactionFailure(x, summaryBinding, state.Records, manual, stage, reason, attempts, failure))
		}
		if e := r.save(context.WithoutCancel(x.ctx), x.inv.Session.ID, records...); e != nil {
			return e
		}
		return handoffFailure{failure}
	}
	var observationErr error
	observeAttempt := func(source model.Binding, attempt model.Attempt) error {
		x.timeline.attempt(attempt)
		if attempt.RetryDelay != nil {
			observationErr = r.save(context.WithoutCancel(x.ctx), x.inv.Session.ID, r.retryRecord(x, source, canonical.PurposeCompaction, attempt))
			return observationErr
		}
		attempts = append(attempts, attempt)
		return nil
	}
	if plan.native {
		compacted, ok, err := r.nativeCompaction(x, binding, route, state, selection, func(a model.Attempt) error {
			return observeAttempt(binding, a)
		})
		if observationErr != nil {
			hideCompacting()
			return observationErr
		}
		if ok {
			return r.saveCompaction(x, binding, route, state, plan, summaryID, compacted.Summary, nil, &compacted.Native, attempts[len(attempts)-1].Usage, attempts)
		}
		var records []canonical.Record
		for _, a := range attempts {
			records = append(records, r.observationRecord(x, binding, canonical.PurposeCompaction, a))
		}
		if e := r.save(context.WithoutCancel(x.ctx), x.inv.Session.ID, records...); e != nil {
			hideCompacting()
			return e
		}
		attempts = nil
		if err != nil {
			hideCompacting()
			return err
		}
	}
	if previous, ok := selection.Prefix[0].Data.(*canonical.Compaction); ok && !previous.Readable() && previous.Native != nil {
		// A transcript can't summarize encrypted items. Expand the original
		// records up to this plan's boundary, not the retained tail.
		readable, err := readableSelection(state.Records, selection.Boundary.ID)
		if err != nil {
			return failed(protocol.ConversationCompactionStageBudget, protocol.ConversationCompactionReasonNoFittingPrefix, err)
		}
		selection.Prefix = readable.Prefix
	}
	languageModel, err := r.model(x.ctx, summaryBinding)
	if err != nil {
		return failed(protocol.ConversationCompactionStageRequest, protocol.ConversationCompactionReasonRequestFailed, err)
	}
	invalidRetry := requestText + "\n\n" + handoff.RetryInvalidOutput
	emptyRetry := requestText + "\n\n" + handoff.RetryEmptyOutput
	systemBytes := model.SummarySystemBytes(languageModel, requestText)
	system, err := handoff.Prepare(selection.Prefix, uint64(window), uint64(output), requestText, systemBytes, invalidRetry, emptyRetry)
	if err != nil {
		return failed(protocol.ConversationCompactionStageBudget, protocol.ConversationCompactionReasonNoFittingPrefix, err)
	}
	requestSummary := func(promptText string) (string, bool, canonical.Usage, error) {
		request := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.User, canonical.Text(0, promptText)), r.clock.Now())
		var text strings.Builder
		limited, textBlock := false, false
		var usage canonical.Usage
		observe := func(attempt model.Attempt) error {
			usage = attempt.Usage
			return observeAttempt(summaryBinding, attempt)
		}
		collect := func(part fantasy.StreamPart) bool {
			switch part.Type {
			case fantasy.StreamPartTypeTextDelta:
				if !textBlock && text.Len() > 0 {
					text.WriteByte('\n')
				}
				text.WriteString(part.Delta)
				textBlock = true
			case fantasy.StreamPartTypeReasoningDelta, fantasy.StreamPartTypeReasoningEnd, fantasy.StreamPartTypeToolCall:
				textBlock = false
			case fantasy.StreamPartTypeFinish:
				switch strings.ToLower(string(part.FinishReason)) {
				case "length", "max_tokens", "max_output_tokens":
					limited = true
				}
			}
			return x.ctx.Err() == nil
		}
		run := func(sys string) error {
			text.Reset()
			limited, textBlock = false, false
			call, err := model.BuildCall(summaryBinding, model.Request{System: sys, Records: []canonical.Record{request}, MaxOutputTokens: &output, Delegated: x.inv.Session.Delegation != nil})
			if err != nil {
				return err
			}
			r.scopeTrace(&call, languageModel, x, summaryBinding, canonical.PurposeCompaction)
			return model.Run(x.ctx, languageModel, call, model.SummaryPurpose, r.clock, observe, collect, nil)
		}
		err := run(system)
		const maxCompactionShrinks = 4
		for shrink := 0; shrink < maxCompactionShrinks && err != nil && x.ctx.Err() == nil && model.Classify(err).Kind == model.ContextOverflow; shrink++ {
			// The model's actual input limit can be much smaller than the captured
			// window. Shrink repeatedly until it fits or reaches the minimum budget.
			smaller, ok := handoff.PrepareSmaller(selection.Prefix, uint64(window), uint64(output), promptText, system, invalidRetry, emptyRetry)
			if !ok || smaller == system {
				break
			}
			system = smaller
			err = run(system)
		}
		return text.String(), limited, usage, err
	}
	failRequest := func(err error) error {
		reason := protocol.ConversationCompactionReasonRequestFailed
		switch model.Classify(err).Kind {
		case model.RateLimited, model.Quota:
			reason = protocol.ConversationCompactionReasonRateLimited
		case model.Authentication, model.NotConfigured:
			reason = protocol.ConversationCompactionReasonAuthorizationFailed
		case model.ContextOverflow:
			reason = protocol.ConversationCompactionReasonContextOverflow
		case model.InvalidRequest, model.Unsupported:
			reason = protocol.ConversationCompactionReasonServiceRejected
		case model.InvalidResponse:
			reason = protocol.ConversationCompactionReasonIncompleteResponse
		}
		return failed(protocol.ConversationCompactionStageRequest, reason, err)
	}
	capLimit := handoff.Cap(uint64(window), uint64(output))
	var summary handoff.Summary
	var usage canonical.Usage
	promptText := requestText
	for round := range 2 {
		rawText, limited, roundUsage, err := requestSummary(promptText)
		if observationErr != nil {
			hideCompacting()
			return observationErr
		}
		if err != nil {
			return failRequest(err)
		}
		usage = roundUsage
		if err := x.ctx.Err(); err != nil {
			return failRequest(err)
		}
		if limited {
			reason := protocol.ConversationCompactionReasonTruncatedSummary
			return failed(protocol.ConversationCompactionStageSummary, reason, compactionSummaryFault(reason))
		}
		parsed, parseErr := handoff.Parse(rawText, capLimit)
		if parseErr == nil {
			summary = parsed
			break
		}
		reason := protocol.ConversationCompactionReasonUnexpectedOutput
		retryPrompt := invalidRetry
		if strings.TrimSpace(rawText) == "" {
			reason = protocol.ConversationCompactionReasonEmptySummary
			retryPrompt = emptyRetry
		}
		if round == 0 {
			promptText = retryPrompt
			continue
		}
		return failed(protocol.ConversationCompactionStageSummary, reason, compactionSummaryFault(reason))
	}
	return r.saveCompaction(x, binding, route, state, plan, summaryID, summary.Render(), compactedFiles(summary), plan.carry, usage, attempts)
}

func (r *Runtime) saveCompaction(x *execution, binding model.Binding, route model.Route, state store.AgentState, plan compactionPlan, summaryID, rendered string, files []canonical.CompactedFile, native *canonical.NativeCompaction, usage canonical.Usage, attempts []model.Attempt) error {
	selection, manual := plan.selection, plan.manual
	boundary := selection.Boundary
	message := canonical.NewMessage(canonical.User)
	if rendered != "" {
		message = canonical.NewMessage(canonical.User, canonical.Text(0, rendered))
	}
	message.Model = &binding.Config.Model
	message.Usage = &usage
	// The last summary attempt is billed on the summary; earlier attempts retain
	// their independent observations.
	compacted := canonical.Compaction{Boundary: boundary.ID, Summary: message, Start: state.Records[0].Created, End: boundary.Created, Files: files, Native: native}
	record := canonical.NewRecord(x.inv.Run.Turn, "compaction", &compacted, r.clock.Now())
	record.ID = summaryID
	record.Provenance.Author = "system"
	runtime := canonical.DefaultRuntime()
	runtime.UserVisible = false
	record.Metadata.Runtime = &runtime
	tailState := state
	tailState.Records = selection.Tail
	loaded := retainedTools(tailState)
	record.Metadata.Context = &canonical.Context{LoadedTools: &loaded}
	var records []canonical.Record
	if len(attempts) > 0 {
		for _, attempt := range attempts[:len(attempts)-1] {
			records = append(records, r.observationRecord(x, binding, canonical.PurposeCompaction, attempt))
		}
		last := r.observationRecord(x, binding, canonical.PurposeCompaction, attempts[len(attempts)-1])
		record.Metadata.Execution = last.Metadata.Execution
	}
	records = append(records, record)
	if len(selection.Tail) > 0 || plan.carry != nil {
		// The original tail remains after Boundary. Projection puts the new
		// summary before it, so no copied messages or synthetic reply is needed.
		return r.compactSession(x, binding, route, r.save(x.ctx, x.inv.Session.ID, records...))
	}
	// The continuation is a runtime notice, not an assistant statement or user
	// input. Automatic compaction interrupts work in progress, so it always
	// asks the model to keep working; only a user-requested compaction ends at
	// a natural conversation boundary.
	continuation := "Context was compacted. The preceding summary covers the earlier conversation. Do not mention the summary or compaction. Continue calling tools as necessary to complete the task."
	if manual {
		continuation = "Context was compacted at the user's request. The preceding summary covers the earlier conversation. Do not mention the summary or compaction. Continue from the summarized context."
	}
	continuation = "<sailry-runtime-continuation>" + continuation + "</sailry-runtime-continuation>"
	var preserved *canonical.Record
	var preservedInput *canonical.Message
	retainMedia := true
	if !manual {
		visible := handoff.Visible(state.Records)
		summaries := map[string]bool{}
		for _, record := range state.Records {
			if record.Kind == "compaction" {
				summaries[record.ID] = true
			}
		}
		for i := len(visible) - 1; i >= 0; i-- {
			if summaries[visible[i].ID] || compactionUsage(visible[i]) || visible[i].Metadata.Runtime != nil && visible[i].Metadata.Runtime.TurnContext {
				continue
			}
			candidate, _, _ := visible[i].ModelContent()
			if candidate != nil && candidate.Role == canonical.Assistant && candidate.Error == nil {
				retainMedia = false
			}
			if candidate != nil && candidate.Role == canonical.User && !containsTool(candidate) {
				for _, block := range candidate.Content {
					switch block.Kind {
					case "text", "image", "document", "file":
						preserved, preservedInput = &visible[i], candidate
					}
					if preserved != nil {
						break
					}
				}
				if preserved != nil {
					break
				}
			}
		}
	}
	continued := canonical.NewRecord(x.inv.Run.Turn, "message", canonical.NewMessage(canonical.User, canonical.Text(0, continuation)), r.clock.Now())
	continued.Provenance.Author = "assistant"
	notice := runtime
	notice.TurnContext = true
	continued.Metadata.Runtime = &notice
	records = append(records, continued)
	if preserved != nil {
		var blocks []canonical.Block
		for _, block := range preservedInput.Content {
			switch block.Kind {
			case "text":
			case "image", "document", "file":
				if !retainMedia {
					continue
				}
			default:
				continue
			}
			block.Index = uint64(len(blocks))
			blocks = append(blocks, block)
		}
		// Pending attachments keep their original turn binding, including when
		// guidance joined this run. Already processed media stays in history.
		carry := canonical.NewRecord(preserved.Turn, "message", canonical.NewMessage(canonical.User, blocks...), r.clock.Now())
		carry.Provenance.Author = "assistant"
		carry.Metadata.Runtime = &runtime
		records = append(records, carry)
	}
	return r.compactSession(x, binding, route, r.save(x.ctx, x.inv.Session.ID, records...))
}

func compactedFiles(summary handoff.Summary) []canonical.CompactedFile {
	var files []canonical.CompactedFile
	for _, file := range summary.Files {
		description := file.Summary
		if file.KeyCode != nil {
			description += "\n" + *file.KeyCode
		}
		files = append(files, canonical.CompactedFile{Path: file.Path, Summary: &description})
	}
	return files
}

func (r *Runtime) nativeCompaction(x *execution, binding model.Binding, route model.Route, state store.AgentState, selection handoff.Selection, observe func(model.Attempt) error) (model.Compacted, bool, error) {
	key := binding.Provider.ID.String() + "\x00" + route.Native
	if _, rejected := r.nativeRejected.Load(key); rejected {
		return model.Compacted{}, false, nil
	}
	try := func() (model.Compacted, error) {
		languageModel, err := r.model(x.ctx, binding)
		if err != nil {
			return model.Compacted{}, err
		}
		// Select already applied visibility and put the prior checkpoint first.
		state.Records = selection.Prefix
		request, err := r.mainRequest(x, binding, state, prompt.Instruction(x.inv.Session.Delegation != nil))
		if err != nil {
			return model.Compacted{}, err
		}
		call, err := model.BuildProjectedCall(binding, request)
		if err != nil {
			return model.Compacted{}, err
		}
		r.scopeTrace(&call, languageModel, x, binding, canonical.PurposeCompaction)
		return model.CompactNative(x.ctx, languageModel, binding, call, r.clock, observe)
	}
	compacted, err := try()
	switch {
	case err == nil:
		nativeOutcome(binding, route, "native", nil)
		return compacted, true, nil
	case errors.Is(err, model.ErrNativeUnsupported):
		r.nativeRejected.Store(key, true)
		nativeOutcome(binding, route, "unsupported", err)
	case errors.Is(err, model.ErrNativeUnusable):
		nativeOutcome(binding, route, "unusable", err)
	default:
		nativeOutcome(binding, route, "failed", err)
		if x.ctx.Err() != nil {
			return model.Compacted{}, false, x.ctx.Err()
		}
	}
	return model.Compacted{}, false, nil
}

func nativeOutcome(binding model.Binding, route model.Route, outcome string, err error) {
	// The cache key includes the endpoint. Log only its protocol, not a URL
	// or a provider response that could contain credentials or prompt text.
	method, _, _ := strings.Cut(route.Native, "\x00")
	attrs := []slog.Attr{slog.String("area", "agent"), slog.String("operation", "native_compaction"), slog.String("outcome", outcome),
		slog.String("provider", binding.Provider.ID.String()), slog.String("method", method), slog.String("model", binding.Config.Model)}
	level := slog.LevelInfo
	if err != nil {
		level = slog.LevelWarn
		attrs = append(attrs, slog.String("reason", string(model.Classify(err).Kind)))
	}
	logging.Emit(level, attrs...)
}

// readableRecords opens opaque checkpoints back up to the original history.
// A previous readable checkpoint still bounds the transcript.
func readableRecords(records []canonical.Record) []canonical.Record {
	return slices.DeleteFunc(slices.Clone(records), func(record canonical.Record) bool {
		c, ok := record.Data.(*canonical.Compaction)
		return ok && c.Native != nil && !c.Readable()
	})
}

func readableSelection(records []canonical.Record, boundary string) (handoff.Selection, error) {
	records = readableRecords(records)
	cut := slices.IndexFunc(records, func(record canonical.Record) bool { return record.ID == boundary })
	if cut < 0 {
		return handoff.Selection{}, &model.Failure{Kind: model.InvalidRequest, Message: "compacted history has no readable boundary"}
	}
	selection := handoff.Selection{Boundary: records[cut]}
	before := make(map[string]bool, cut+1)
	for _, record := range records[:cut+1] {
		before[record.ID] = true
	}
	var previous *canonical.Record
	for i := range records {
		if records[i].Kind == "compaction" {
			previous = &records[i]
		}
	}
	for _, record := range handoff.Visible(records) {
		if previous != nil && record.ID == previous.ID {
			// Preserve the checkpoint kind so Prepare pins, rather than trims, it.
			summary := *previous.Data.(*canonical.Compaction)
			summary.Summary = *record.Message()
			record.Kind, record.Data = "compaction", &summary
			selection.Prefix = append(selection.Prefix, record)
		} else if before[record.ID] {
			selection.Prefix = append(selection.Prefix, record)
		} else {
			selection.Tail = append(selection.Tail, record)
		}
	}
	if len(selection.Prefix) == 0 {
		return handoff.Selection{}, &model.Failure{Kind: model.InvalidRequest, Message: "compacted history has no visible prefix"}
	}
	return selection, nil
}

// readableCheckpoint adds a handoff at the same boundary, without making
// another continuation or losing the native items for the producing route.
func (r *Runtime) readableCheckpoint(x *execution, binding model.Binding, route model.Route, state store.AgentState, force bool) (bool, error) {
	for i := len(state.Records) - 1; i >= 0; i-- {
		record := state.Records[i]
		if record.Kind != "compaction" {
			continue
		}
		c := record.Data.(*canonical.Compaction)
		if c.Native == nil || c.Readable() || (!force && model.ReplaysNative(binding, record)) {
			return false, nil
		}
		selection, err := readableSelection(state.Records, c.Boundary)
		if err != nil {
			return false, handoffFailure{err}
		}
		plan := compactionPlan{selection: selection, carry: c.Native, window: route.ReplayWindow(capturedModel(binding).Context)}
		return true, r.summarize(x, binding, route, state, plan)
	}
	return false, nil
}

// compactSession follows a saved handoff on an agent session that holds the
// history and advertises `/compact`: the agent shrinks its own context and
// keeps serving the session, as in T3 Code, instead of a new session that
// replays the handoff. The handoff came first, while the session still held
// everything; the command is a second pass over that context. A command that
// fails leaves the recorded handoff, and the next request replays it.
func (r *Runtime) compactSession(x *execution, binding model.Binding, route model.Route, saved error) error {
	if saved != nil || !route.Session || !route.Resident || !route.Compact {
		return saved
	}
	state, err := r.load(x.ctx, x.inv.Session.ID)
	if err != nil {
		return err
	}
	instruction := prompt.Instruction(x.inv.Session.Delegation != nil)
	request, err := r.mainRequest(x, binding, state, instruction)
	if err != nil {
		return err
	}
	call, err := model.BuildProjectedCall(binding, request)
	if err != nil {
		return err
	}
	languageModel, err := r.model(x.ctx, binding)
	if err != nil {
		return err
	}
	_ = model.CompactSession(x.ctx, languageModel, call)
	return x.ctx.Err()
}
func compactionSummaryFault(reason protocol.ConversationCompactionReason) *protocol.Fault {
	message := "Compaction failed: the model did not return a valid structured handoff JSON after retry. Conversation history was preserved; try compacting again, switching models, or starting a new session."
	switch reason {
	case protocol.ConversationCompactionReasonEmptySummary:
		message = "Compaction failed: the model returned an empty summary after retry. Conversation history was preserved; try compacting again, switching models, or starting a new session."
	case protocol.ConversationCompactionReasonTruncatedSummary:
		message = "Compaction failed: the summary reached the output token limit before completing. Conversation history was preserved; try compacting again or starting a new session."
	}
	return &protocol.Fault{
		Code:    protocol.ErrorCodeInvalidRequest,
		Message: message,
		Details: &protocol.FaultDetails{
			Kind: "model_request",
			Data: protocol.FaultDetailsModelRequestPayload{
				Reason: protocol.ModelFailureInvalidResponse,
			},
		},
	}
}
func containsTool(message *canonical.Message) bool {
	for _, block := range message.Content {
		if block.Kind == "tool_request" || block.Kind == "tool_response" {
			return true
		}
	}
	return false
}
func retainedTools(state store.AgentState) []string {
	present := map[string]bool{}
	searches := map[string]bool{}
	for _, record := range handoff.Visible(state.Records) {
		if m := record.Message(); m != nil {
			for _, block := range m.Content {
				switch value := block.Data.(type) {
				case *canonical.ToolRequest:
					if value.Error == nil {
						present[value.Name] = true
						if value.Name == "tool_search" {
							searches[value.ID] = true
						}
					}
				case *canonical.ToolResponse:
					if value.Format == canonical.MCPError || !searches[value.ID] {
						continue
					}
					var discovered struct {
						Tools      []struct{ Name string } `json:"tools"`
						Structured json.RawMessage         `json:"structuredContent"`
					}
					if json.Unmarshal(value.Result.Value, &discovered) != nil {
						continue
					}
					if value.Format == canonical.MCP && len(discovered.Structured) > 0 {
						_ = json.Unmarshal(discovered.Structured, &discovered)
					}
					for _, tool := range discovered.Tools {
						present[tool.Name] = true
					}
				}
			}
		}
	}
	out := []string{}
	for _, name := range state.LoadedTools {
		if present[name] {
			out = append(out, name)
		}
	}
	return out
}
