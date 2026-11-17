package workbench

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
)

func TestStateDropsSupersededConnectionResults(t *testing.T) {
	state := NewState()
	first, _ := state.BeginConnect()
	second, _ := state.BeginConnect()
	if actions := state.ApplyConnect(ConnectResult{Generation: first, DaemonVersion: "old"}); actions.LoadSessions {
		t.Fatal("stale connection requested a catalog load")
	}
	if state.Status != Connecting {
		t.Fatalf("stale connection changed status to %v", state.Status)
	}
	actions := state.ApplyConnect(ConnectResult{Generation: second, DaemonVersion: "new"})
	if !actions.LoadSessions || state.Status != Connected || state.DaemonVersion != "new" {
		t.Fatalf("current connection state = %+v, actions = %+v", state.Snapshot(), actions)
	}
	state.ApplySessions(SessionsResult{
		Generation: first, Sessions: []SessionSummary{{ID: "old-session"}},
	})
	if state.Sessions != nil || !state.SessionsLoading {
		t.Fatalf("stale catalog changed state: %+v", state.Snapshot())
	}
}

func TestStateBuffersEventsUntilHydrateWithoutLossOrDuplicates(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	load := state.SelectSession("session-a")
	state.ApplyAttach(AttachResult{
		Generation: load.Generation, SessionGeneration: load.SessionGeneration,
		SessionID: "session-a", Runtime: &daemon.SessionRuntime{RuntimeID: "runtime-session-a"},
	})
	state.ApplyEvent(EventResult{Generation: load.Generation, Event: transcriptTextEvent("session-a", "runtime-session-a", 1, "不应重复")})
	deltas := []string{"零", "一", "二", "三", "四", "五", "六", "七", "八", "九"}
	for index, delta := range deltas {
		state.ApplyEvent(EventResult{
			Generation: load.Generation,
			Event:      transcriptTextEvent("session-a", "runtime-session-a", uint64(index+2), delta),
		})
	}
	if got := len(state.pendingEvents); got != len(deltas)+1 {
		t.Fatalf("pending events = %d, want %d", got, len(deltas)+1)
	}
	if len(state.Transcript) != 0 {
		t.Fatalf("events applied before hydrate: %+v", state.Transcript)
	}
	session := hydratedSession("session-a", 1)
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: 1,
	}
	if !state.ApplyHydrate(NewHydrateResult(load.Generation, load.SessionGeneration, "session-a", session, nil)) {
		t.Fatal("current hydrate was rejected")
	}
	if len(state.Transcript) != 2 || state.Transcript[1].Text != strings.Join(deltas, "") {
		t.Fatalf("hydrated transcript = %+v", state.Transcript)
	}
	if strings.Contains(state.Transcript[1].Text, "不应重复") {
		t.Fatalf("snapshot event was replayed twice: %+v", state.Transcript)
	}
	if len(state.pendingEvents) != 0 {
		t.Fatalf("pending events after replay = %d, want 0", len(state.pendingEvents))
	}
	if state.eventCursor == nil || state.eventCursor.Sequence != 11 {
		t.Fatalf("event cursor after replay = %+v, want sequence 11", state.eventCursor)
	}
}

func TestStateHydrateWatermarkReplaysOnlyNewerBufferedEvents(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	load := state.SelectSession("session-a")
	for _, event := range []struct {
		sequence uint64
		text     string
	}{{3, "三"}, {5, "五"}, {7, "七"}} {
		state.ApplyEvent(EventResult{
			Generation: load.Generation,
			Event:      transcriptTextEvent("session-a", "runtime-session-a", event.sequence, event.text),
		})
	}
	if got := len(state.pendingEvents); got != 3 {
		t.Fatalf("pending events = %d, want 3", got)
	}
	session := hydratedSession("session-a", 0)
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: 5,
	}
	state.ApplyHydrate(NewHydrateResult(load.Generation, load.SessionGeneration, "session-a", session, nil))
	if len(state.Transcript) != 1 || state.Transcript[0].Text != "七" {
		t.Fatalf("transcript after watermark replay = %+v, want only sequence 7", state.Transcript)
	}
	if state.eventCursor == nil || state.eventCursor.Sequence != 7 {
		t.Fatalf("event cursor after replay = %+v, want sequence 7", state.eventCursor)
	}
}

func TestStateEventBufferOverflowDebouncesRehydrate(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	initial := state.SelectSession("session-a")
	var rehydrate Actions
	for sequence := uint64(1); sequence <= PendingEventLimit+1; sequence++ {
		actions := state.ApplyEvent(EventResult{
			Generation: initial.Generation,
			Event:      transcriptTextEvent("session-a", "runtime-session-a", sequence, "首批增量"),
		})
		if actions.Rehydrate {
			rehydrate = actions
		}
	}
	if !rehydrate.EventBufferOverflow || !rehydrate.Rehydrate || rehydrate.HydrateSessionID != "session-a" {
		t.Fatalf("first overflow actions = %+v", rehydrate)
	}
	if !state.rehydrating || len(state.pendingEvents) != 0 {
		t.Fatalf("first overflow state: rehydrating=%v pending=%d", state.rehydrating, len(state.pendingEvents))
	}

	for sequence := uint64(1); sequence <= PendingEventLimit+1; sequence++ {
		actions := state.ApplyEvent(EventResult{
			Generation: initial.Generation,
			Event:      transcriptTextEvent("session-a", "runtime-session-a", sequence, "第二批增量"),
		})
		if actions.Rehydrate || actions.HydrateSessionID != "" {
			t.Fatalf("second overflow restarted hydration: %+v", actions)
		}
	}
	if len(state.pendingEvents) > PendingEventLimit {
		t.Fatalf("pending event buffer length = %d, limit %d", len(state.pendingEvents), PendingEventLimit)
	}
	if state.ApplyHydrate(NewHydrateResult(initial.Generation, initial.SessionGeneration, "session-a", hydratedSession("session-a", 1), nil)) {
		t.Fatal("superseded initial hydrate was accepted")
	}
	session := hydratedSession("session-a", 0)
	session.Messages = []daemon.Message{{ID: "final", Role: "assistant", Content: "最终快照"}}
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: PendingEventLimit + 1,
	}
	if !state.ApplyHydrate(NewHydrateResult(rehydrate.Generation, rehydrate.SessionGeneration, "session-a", session, nil)) {
		t.Fatal("authoritative rehydrate was rejected")
	}
	if len(state.Transcript) != 1 || state.Transcript[0].Text != "最终快照" {
		t.Fatalf("final transcript = %+v", state.Transcript)
	}
	if strings.Contains(state.Transcript[0].Text, "增量") {
		t.Fatalf("dropped overflow events entered final transcript: %+v", state.Transcript)
	}
	if state.rehydrating {
		t.Fatal("successful rehydrate did not reset its in-progress flag")
	}
}

func TestStateFailedRehydrateResetsInProgressFlag(t *testing.T) {
	state := NewState()
	state.generation = 1
	state.sessionGeneration = 1
	state.rehydrating = true
	state.SelectedSessionID = "session-a"
	state.Sessions = []SessionSummary{{ID: "session-a"}}
	state.SessionLoading = true

	state.ApplyHydrate(HydrateResult{
		Generation: 1, SessionGeneration: 1, SessionID: "session-a", Err: errors.New("offline"),
	})
	if state.rehydrating || state.SessionLoading || state.SessionError != "offline" {
		t.Fatalf("failed rehydrate state = %+v, in progress = %v", state.Snapshot(), state.rehydrating)
	}
}

func TestStateSessionRefreshPrunesDeletedSessionDrafts(t *testing.T) {
	state := NewState()
	generation, _ := state.BeginConnect()
	state.drafts["session-a"] = "保留"
	state.drafts["deleted-session"] = "清理"

	state.ApplySessions(SessionsResult{
		Generation: generation, SessionGeneration: state.sessionGeneration,
		Sessions: []SessionSummary{{ID: "session-a"}},
	})
	if len(state.Sessions) != 1 || state.Sessions[0].ID != "session-a" {
		t.Fatalf("loaded sessions = %+v", state.Sessions)
	}
	if got := state.drafts["session-a"]; got != "保留" {
		t.Fatalf("existing session draft = %q, want 保留", got)
	}
	if draft, ok := state.drafts["deleted-session"]; ok {
		t.Fatalf("deleted session draft survived refresh: %q", draft)
	}
}

func TestStateDraftsFollowSelectionAndPreserveInFlightText(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"}, daemon.AgentSession{ID: "session-b"})
	selectAndHydrate(t, &state, "session-a", 0)
	const original = "发送途中切会话也要保留"
	state.SetDraft(original)
	send, ok := state.BeginSend(original)
	if !ok || state.Draft != "" || !state.Sending {
		t.Fatalf("begin send = %+v, ok=%v, state=%+v", send, ok, state.Snapshot())
	}

	selectAndHydrate(t, &state, "session-b", 0)
	if state.Draft != "" {
		t.Fatalf("session-b draft = %q, want empty", state.Draft)
	}
	selectAndHydrate(t, &state, "session-a", 0)
	if state.Draft != original {
		t.Fatalf("restored in-flight draft = %q, want %q", state.Draft, original)
	}
	if state.FinishSend(SendResult{
		Generation: send.Generation, SessionGeneration: send.SessionGeneration,
		SessionID: send.SessionID, Text: send.Text,
	}) {
		t.Fatal("send from superseded selection was applied")
	}

	next, ok := state.BeginSend(original)
	if !ok {
		t.Fatal("restored draft could not be sent")
	}
	state.FinishSend(SendResult{
		Generation: next.Generation, SessionGeneration: next.SessionGeneration,
		SessionID: next.SessionID, Text: next.Text,
	})
	if _, ok := state.drafts["session-a"]; ok {
		t.Fatalf("successful send kept session-a draft %q", state.drafts["session-a"])
	}
	selectAndHydrate(t, &state, "session-b", 0)
	selectAndHydrate(t, &state, "session-a", 0)
	if state.Draft != "" {
		t.Fatalf("successful send restored stale draft %q", state.Draft)
	}
}

func TestStateDeletedSessionLateHydrateDoesNotWriteTranscript(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	load := state.SelectSession("session-a")
	// Isolate catalog membership from the generation and selected-ID guards.
	state.Sessions = nil
	if state.ApplyHydrate(NewHydrateResult(load.Generation, load.SessionGeneration, "session-a", hydratedSession("session-a", 1), nil)) {
		t.Fatal("hydrate for deleted session was accepted")
	}
	if state.SelectedSessionID != "session-a" || len(state.Transcript) != 0 || state.MessageCount != 0 {
		t.Fatalf("deleted session hydrate wrote transcript: %+v", state.Snapshot())
	}
	if state.hydratedSessionID != "" || state.hydratedGeneration != 0 {
		t.Fatalf("deleted session was marked hydrated: generation=%d ID=%q", state.hydratedGeneration, state.hydratedSessionID)
	}
}

func TestStateSelectionRequiresCatalogMembership(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	if actions := state.SelectSession("missing"); actions.HydrateSessionID != "" {
		t.Fatalf("unknown selection requested hydrate: %+v", actions)
	}
	load := state.SelectSession("session-a")
	state.Sessions = []SessionSummary{{ID: "other"}}
	actions := state.ApplySessions(SessionsResult{
		Generation: load.Generation, SessionGeneration: load.SessionGeneration,
		SelectedSessionID: "session-a", Sessions: []SessionSummary{{ID: "other"}},
	})
	if !actions.SelectionCleared || state.SelectedSessionID != "" {
		t.Fatalf("deleted selection was not cleared: state=%+v actions=%+v", state.Snapshot(), actions)
	}
	if state.ApplyHydrate(NewHydrateResult(load.Generation, load.SessionGeneration, "session-a", hydratedSession("session-a", 1), nil)) {
		t.Fatal("late hydrate landed after selection was cleared")
	}
}

func TestStateDropsLateHydrateAndEvents(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"}, daemon.AgentSession{ID: "session-b"})
	first := state.SelectSession("session-a")
	second := selectAndHydrate(t, &state, "session-b", 1)
	if state.ApplyHydrate(NewHydrateResult(first.Generation, first.SessionGeneration, "session-a", hydratedSession("session-a", 3), nil)) {
		t.Fatal("late hydrate was accepted")
	}
	state.ApplyEvent(EventResult{Generation: second.Generation, Event: transcriptTextEvent("session-a", "runtime-session-a", 8, "迟到内容")})
	state.ApplyEvent(EventResult{Generation: second.Generation, Event: transcriptTextEvent("session-b", "runtime-session-b", 9, "当前内容")})
	state.ApplyEvent(EventResult{Generation: second.Generation - 1, Event: transcriptTextEvent("session-b", "runtime-session-b", 10, "旧连接内容")})
	if state.SelectedSessionID != "session-b" || state.MessageCount != 1 || len(state.Transcript) != 2 {
		t.Fatalf("late results overwrote selection: %+v", state.Snapshot())
	}
	if strings.Contains(state.Transcript[0].Text, "迟到内容") || !strings.Contains(state.Transcript[1].Text, "当前内容") || strings.Contains(state.Transcript[1].Text, "旧连接内容") {
		t.Fatalf("late event entered current transcript: %+v", state.Transcript)
	}
}

func TestStateSendFailureAndSuccessTransitions(t *testing.T) {
	state := connectedState(t, daemon.AgentSession{ID: "session-a"})
	selectAndHydrate(t, &state, "session-a", 0)
	if _, ok := state.BeginSend("  \n"); ok {
		t.Fatal("whitespace-only prompt was accepted")
	}
	const original = "请保留这段输入"
	state.SetDraft(original)
	action, ok := state.BeginSend(original)
	if !ok || !state.Sending || state.Draft != "" {
		t.Fatalf("begin send state = %+v, ok=%v", state.Snapshot(), ok)
	}
	if _, ok := state.BeginSend("并发发送"); ok {
		t.Fatal("concurrent send was accepted")
	}
	state.FinishSend(SendResult{
		Generation: action.Generation, SessionGeneration: action.SessionGeneration,
		SessionID: action.SessionID, Text: action.Text, Err: errors.New("offline"),
	})
	if state.Sending || state.Draft != original || state.SendError != "发送失败，已保留输入：offline" {
		t.Fatalf("failed send state = %+v", state.Snapshot())
	}
	if got := state.drafts["session-a"]; got != original {
		t.Fatalf("saved failure draft = %q, want %q", got, original)
	}
	action, ok = state.BeginSend(original)
	if !ok {
		t.Fatal("retry after failure was rejected")
	}
	state.FinishSend(SendResult{
		Generation: action.Generation, SessionGeneration: action.SessionGeneration,
		SessionID: action.SessionID, Text: action.Text,
	})
	if state.Sending || state.Draft != "" || state.SendError != "" {
		t.Fatalf("successful send state = %+v", state.Snapshot())
	}
}

func TestFinishSendSuccessDeletesSavedSessionDraft(t *testing.T) {
	state := NewState()
	state.generation = 1
	state.sessionGeneration = 1
	state.SelectedSessionID = "session-a"
	state.Sessions = []SessionSummary{{ID: "session-a"}}
	state.Sending = true
	state.sendingText = "旧稿"
	state.drafts["session-a"] = "旧稿"

	state.FinishSend(SendResult{Generation: 1, SessionGeneration: 1, SessionID: "session-a", Text: "旧稿"})
	if state.Sending || state.Draft != "" {
		t.Fatalf("successful send state = %+v", state.Snapshot())
	}
	if draft, ok := state.drafts["session-a"]; ok {
		t.Fatalf("successful send kept saved draft %q", draft)
	}
}

func connectedState(t *testing.T, sessions ...daemon.AgentSession) State {
	t.Helper()
	state := NewState()
	generation, _ := state.BeginConnect()
	connected := state.ApplyConnect(ConnectResult{Generation: generation, DaemonVersion: "test-daemon"})
	state.ApplySessions(SessionsResult{
		Generation: generation, SessionGeneration: connected.SessionGeneration,
		SelectedSessionID: connected.SelectedSessionID, Sessions: sessions,
	})
	return state
}

func selectAndHydrate(t *testing.T, state *State, sessionID string, messageCount int) Actions {
	t.Helper()
	actions := state.SelectSession(sessionID)
	if actions.HydrateSessionID != sessionID {
		t.Fatalf("selection actions = %+v, want hydrate %q", actions, sessionID)
	}
	state.ApplyAttach(AttachResult{
		Generation: actions.Generation, SessionGeneration: actions.SessionGeneration,
		SessionID: sessionID, Runtime: &daemon.SessionRuntime{RuntimeID: "runtime-" + sessionID},
	})
	if !state.ApplyHydrate(NewHydrateResult(actions.Generation, actions.SessionGeneration, sessionID, hydratedSession(sessionID, messageCount), nil)) {
		t.Fatalf("hydrate %q was rejected", sessionID)
	}
	return actions
}

func hydratedSession(id string, messageCount int) *daemon.AgentSession {
	session := &daemon.AgentSession{ID: id, Title: id}
	for index := range messageCount {
		session.Messages = append(session.Messages, daemon.Message{
			ID: id + "-message-" + string(rune('a'+index)), Role: "assistant", Content: "已有消息",
		})
	}
	return session
}

func transcriptTextEvent(sessionID, runtimeID string, sequence uint64, text string) daemon.SequencedEvent {
	payload, _ := json.Marshal(text)
	return daemon.SequencedEvent{
		SessionID: sessionID, RuntimeID: runtimeID, Epoch: "epoch-test", Sequence: sequence,
		Event: daemon.WireDriverEvent{Kind: "textDelta", Payload: payload},
	}
}
