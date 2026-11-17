package workbench

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
)

// TranscriptKind identifies the native row used to render one ordered entry.
type TranscriptKind uint8

const (
	TranscriptUser TranscriptKind = iota
	TranscriptAssistant
	TranscriptReasoning
	TranscriptActivity
	TranscriptNotice
)

// TranscriptEntry is an immutable render value. Updates replace the containing
// slice and changed entry, so a frame never observes a background mutation.
type TranscriptEntry struct {
	ID        string
	Kind      TranscriptKind
	Title     string
	Text      string
	Detail    string
	Streaming bool
	Failed    bool
	Complete  bool
}

// ProjectTranscript converts one daemon snapshot into ordered render entries.
func ProjectTranscript(session *daemon.AgentSession) []TranscriptEntry {
	if session == nil {
		return nil
	}
	entries := make([]TranscriptEntry, 0, len(session.Messages)+len(session.TranscriptBlocks))
	appendBlocks := func(afterMessage int) {
		for index, block := range session.TranscriptBlocks {
			if block.AfterMessage == afterMessage {
				entries = appendTranscriptBlock(entries, block, index)
			}
		}
	}
	appendBlocks(0)
	for index, message := range session.Messages {
		kind := TranscriptAssistant
		title := "助手"
		if message.Role == "user" {
			kind = TranscriptUser
			title = "你"
		} else if message.Role == "system" {
			kind = TranscriptNotice
			title = "系统"
		}
		entries = append(entries, TranscriptEntry{
			ID: message.ID, Kind: kind, Title: title, Text: message.VisibleContent(), Streaming: message.Streaming,
		})
		appendBlocks(index + 1)
	}
	// Be liberal toward older state with an out-of-range block index. Valid
	// blocks were already emitted above; this preserves their relative order.
	for index, block := range session.TranscriptBlocks {
		if block.AfterMessage > len(session.Messages) {
			entries = appendTranscriptBlock(entries, block, index)
		}
	}
	return entries
}

func appendTranscriptBlock(entries []TranscriptEntry, block daemon.TranscriptBlock, blockIndex int) []TranscriptEntry {
	switch block.Content.Kind {
	case "activities":
		var activities []daemon.ActivityItem
		if json.Unmarshal(block.Content.Data, &activities) != nil {
			return entries
		}
		for activityIndex, activity := range activities {
			id := activity.ID
			if id == "" {
				id = fmt.Sprintf("block-%d-%d-%d", block.AfterMessage, blockIndex, activityIndex)
			}
			entries = append(entries, activityEntry(id, activity))
		}
	case "reasoning":
		var reasoning struct {
			Content string `json:"content"`
		}
		if json.Unmarshal(block.Content.Data, &reasoning) == nil && reasoning.Content != "" {
			entries = append(entries, TranscriptEntry{
				ID:   fmt.Sprintf("reasoning-%d-%d", block.AfterMessage, blockIndex),
				Kind: TranscriptReasoning, Title: "思考过程", Text: reasoning.Content, Complete: true,
			})
		}
	}
	return entries
}

func activityEntry(id string, activity daemon.ActivityItem) TranscriptEntry {
	details := make([]string, 0, 3)
	if activity.Detail != nil && *activity.Detail != "" {
		details = append(details, *activity.Detail)
	}
	if activity.Arguments != nil && *activity.Arguments != "" {
		details = append(details, *activity.Arguments)
	}
	if activity.Output != nil && *activity.Output != "" {
		details = append(details, *activity.Output)
	}
	title := activity.Title
	if title == "" {
		title = "工具活动"
	}
	return TranscriptEntry{
		ID: id, Kind: TranscriptActivity, Title: title, Detail: strings.Join(details, "\n\n"),
		Failed: activity.Failed, Complete: activity.Complete,
	}
}

// ApplyTranscriptEvent folds one live provider event into ordered entries. The
// caller owns generation and watermark checks; this function only projects the
// event payload.
func ApplyTranscriptEvent(entries []TranscriptEntry, event daemon.SequencedEvent) []TranscriptEntry {
	switch event.Event.Kind {
	case "textDelta":
		var delta string
		if json.Unmarshal(event.Event.Payload, &delta) != nil {
			return entries
		}
		return appendStreamingText(entries, event, TranscriptAssistant, "助手", delta)
	case "reasoningDelta":
		var delta string
		if json.Unmarshal(event.Event.Payload, &delta) != nil {
			return entries
		}
		return appendStreamingText(entries, event, TranscriptReasoning, "思考过程", delta)
	case "activity":
		var activity daemon.ActivityItem
		if json.Unmarshal(event.Event.Payload, &activity) != nil {
			return entries
		}
		return upsertActivity(entries, activity)
	case "richActivity":
		var activity daemon.ActivityItem
		if json.Unmarshal(event.Event.Payload, &activity) != nil {
			return entries
		}
		return upsertActivity(entries, activity)
	case "error":
		var message string
		if json.Unmarshal(event.Event.Payload, &message) != nil || message == "" {
			return entries
		}
		updated := settleStreaming(entries)
		return append(updated, TranscriptEntry{
			ID: liveEventID(event), Kind: TranscriptNotice, Title: "运行错误", Detail: message, Failed: true, Complete: true,
		})
	case "turnFinished", "processExited":
		return settleStreaming(entries)
	default:
		return entries
	}
}

func appendStreamingText(entries []TranscriptEntry, event daemon.SequencedEvent, kind TranscriptKind, title, delta string) []TranscriptEntry {
	last := len(entries) - 1
	if last >= 0 && entries[last].Kind == kind && entries[last].Streaming {
		updated := append([]TranscriptEntry(nil), entries...)
		updated[last].Text += delta
		return updated
	}
	if strings.TrimSpace(delta) == "" {
		return entries
	}
	updated := settleStreaming(entries)
	return append(updated, TranscriptEntry{
		ID: liveEventID(event), Kind: kind, Title: title, Text: delta, Streaming: true,
	})
}

func upsertActivity(entries []TranscriptEntry, activity daemon.ActivityItem) []TranscriptEntry {
	entry := activityEntry(activity.ID, activity)
	updated := append([]TranscriptEntry(nil), entries...)
	for index := range updated {
		if updated[index].Kind == TranscriptActivity && updated[index].ID == activity.ID {
			updated[index] = entry
			return updated
		}
	}
	return append(updated, entry)
}

func settleStreaming(entries []TranscriptEntry) []TranscriptEntry {
	changed := false
	for _, entry := range entries {
		if entry.Streaming {
			changed = true
			break
		}
	}
	if !changed {
		return entries
	}
	updated := append([]TranscriptEntry(nil), entries...)
	for index := range updated {
		updated[index].Streaming = false
		if updated[index].Kind == TranscriptActivity || updated[index].Kind == TranscriptReasoning {
			updated[index].Complete = true
		}
	}
	return updated
}

func liveEventID(event daemon.SequencedEvent) string {
	return fmt.Sprintf("live-%s-%s-%s-%d", event.SessionID, event.RuntimeID, event.Epoch, event.Sequence)
}
