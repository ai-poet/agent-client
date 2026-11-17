// Package workbench projects daemon results and events into UI-independent
// workbench state. It owns no connections, performs no I/O, and has no MyGo UI
// dependency. A State is deliberately unsynchronized: exactly one goroutine
// (the desktop UI thread) calls it, while background workers only produce the
// immutable result values declared here.
package workbench

import (
	"fmt"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
)

// PendingEventLimit bounds events received between attach and hydration.
const PendingEventLimit = 256

// Status is the user-visible daemon connection state.
type Status uint8

const (
	Disconnected Status = iota
	Connecting
	Connected
	Failed
)

func (s Status) Label() string {
	switch s {
	case Connecting:
		return "连接中"
	case Connected:
		return "已连接"
	case Failed:
		return "连接失败"
	default:
		return "未连接"
	}
}

// SessionSummary is the daemon's existing lightweight session representation,
// not a second wire type owned by the projection.
type SessionSummary = daemon.AgentSession

// Snapshot is an immutable copy of what a frame should render.
type Snapshot struct {
	Status        Status
	DaemonVersion string
	Detail        string

	Sessions          []SessionSummary
	SelectedSessionID string
	SessionsLoading   bool
	SessionsError     string
	SessionLoading    bool
	SessionError      string
	MessageCount      int
	Transcript        []TranscriptEntry
	Draft             string
	Sending           bool
	SendError         string
}

// Actions describes I/O that the shell must trigger after a projection change.
// The projection never retains or calls a connection itself.
type Actions struct {
	LoadSessions        bool
	HydrateSessionID    string
	Generation          uint64
	SessionGeneration   uint64
	SelectedSessionID   string
	SelectionCleared    bool
	EventBufferOverflow bool
	Rehydrate           bool
}

// ConnectResult is the UI-thread delivery of one completed connection attempt.
type ConnectResult struct {
	Generation    uint64
	DaemonVersion string
	Err           error
}

// SessionsResult is one completed loadTaskState call. The two selection tokens
// capture what was selected when the call started, so a refresh can rehydrate
// only if that selection has not changed in the meantime.
type SessionsResult struct {
	Generation        uint64
	SessionGeneration uint64
	SelectedSessionID string
	Sessions          []SessionSummary
	Err               error
}

// AttachResult is one completed attachSession call.
type AttachResult struct {
	Generation        uint64
	SessionGeneration uint64
	SessionID         string
	Runtime           *daemon.SessionRuntime
}

// HydrateResult is one completed attach/hydrate sequence.
type HydrateResult struct {
	Generation        uint64
	SessionGeneration uint64
	SessionID         string
	Session           *daemon.AgentSession
	Transcript        []TranscriptEntry
	MessageCount      int
	Err               error
}

// NewHydrateResult projects the immutable daemon snapshot away from the UI
// thread. ApplyHydrate subsequently validates and installs this result.
func NewHydrateResult(generation, sessionGeneration uint64, sessionID string, session *daemon.AgentSession, err error) HydrateResult {
	result := HydrateResult{
		Generation: generation, SessionGeneration: sessionGeneration,
		SessionID: sessionID, Session: session, Err: err,
	}
	if session != nil {
		result.Transcript = ProjectTranscript(session)
		result.MessageCount = len(session.Messages)
	}
	return result
}

// EventResult tags a provider event with the connection that produced it.
type EventResult struct {
	Generation uint64
	Event      daemon.SequencedEvent
}

// SendAction contains everything needed for an asynchronous prompt call.
type SendAction struct {
	Generation        uint64
	SessionGeneration uint64
	SessionID         string
	RuntimeID         string
	Text              string
}

// SendResult is one completed prompt call.
type SendResult struct {
	Generation        uint64
	SessionGeneration uint64
	SessionID         string
	Text              string
	Err               error
}

// State owns all session, transcript, draft, and send projection state. The
// exported fields are render values; transition invariants live in the private
// generation, hydration, buffer, cursor, and per-session draft fields.
type State struct {
	Status        Status
	DaemonVersion string
	Detail        string

	Sessions          []SessionSummary
	SelectedSessionID string
	SessionsLoading   bool
	SessionsError     string
	SessionLoading    bool
	SessionError      string
	MessageCount      int
	Transcript        []TranscriptEntry
	Draft             string
	Sending           bool
	SendError         string

	closed             bool
	generation         uint64
	sessionGeneration  uint64
	hydratedGeneration uint64
	hydratedSessionID  string
	rehydrating        bool
	pendingEvents      []daemon.SequencedEvent
	eventCursor        *daemon.RuntimeEventCursor
	attachedRuntimeID  string
	sendingText        string
	drafts             map[string]string
}

func NewState() State {
	return State{Status: Disconnected, drafts: make(map[string]string)}
}

// Snapshot returns a frame-safe copy of the render values.
func (s *State) Snapshot() Snapshot {
	return Snapshot{
		Status: s.Status, DaemonVersion: s.DaemonVersion, Detail: s.Detail,
		Sessions:          append([]SessionSummary(nil), s.Sessions...),
		SelectedSessionID: s.SelectedSessionID,
		SessionsLoading:   s.SessionsLoading, SessionsError: s.SessionsError,
		SessionLoading: s.SessionLoading, SessionError: s.SessionError,
		MessageCount: s.MessageCount,
		Transcript:   append([]TranscriptEntry(nil), s.Transcript...),
		Draft:        s.Draft, Sending: s.Sending, SendError: s.SendError,
	}
}

// BeginConnect invalidates every result from the preceding connection while
// retaining the catalog selection and its per-session draft for rehydration.
func (s *State) BeginConnect() (uint64, bool) {
	if s.closed {
		return 0, false
	}
	s.saveActiveDraft()
	s.sendingText = ""
	s.generation++
	s.sessionGeneration++
	s.resetSessionProjection()
	s.Status = Connecting
	s.Detail = ""
	s.DaemonVersion = ""
	s.SessionsLoading = false
	s.SessionsError = ""
	s.Sending = false
	s.SendError = ""
	s.restoreActiveDraft()
	return s.generation, true
}

// ApplyConnect installs a current connection result and requests the initial
// catalog load. Superseded attempts have no effect.
func (s *State) ApplyConnect(result ConnectResult) Actions {
	if !s.ConnectionCurrent(result.Generation) {
		return Actions{}
	}
	if result.Err != nil {
		s.Status = Failed
		s.Detail = result.Err.Error()
		s.SessionsLoading = false
		return Actions{}
	}
	s.Status = Connected
	s.DaemonVersion = result.DaemonVersion
	s.Detail = ""
	s.SessionsLoading = true
	s.SessionsError = ""
	s.SessionLoading = false
	s.SessionError = ""
	return Actions{
		LoadSessions: true, Generation: s.generation,
		SessionGeneration: s.sessionGeneration, SelectedSessionID: s.SelectedSessionID,
	}
}

// ApplySessions installs a current catalog, prunes drafts for deleted sessions,
// validates selection membership, and requests hydration when appropriate.
func (s *State) ApplySessions(result SessionsResult) Actions {
	if !s.ConnectionCurrent(result.Generation) {
		return Actions{}
	}
	s.SessionsLoading = false
	if result.Err != nil {
		s.SessionsError = result.Err.Error()
		return Actions{}
	}
	s.Sessions = append([]SessionSummary(nil), result.Sessions...)
	// Selection is limited to this list, so successful refreshes are a simple
	// bounded-drain point for drafts whose sessions no longer exist.
	s.pruneDrafts(s.Sessions)
	s.SessionsError = ""
	selected := s.SelectedSessionID
	if selected != "" && !sessionExists(s.Sessions, selected) {
		s.clearSelection()
		return Actions{SelectionCleared: true}
	}
	if selected != "" && selected == result.SelectedSessionID &&
		s.sessionGeneration == result.SessionGeneration {
		return s.beginSessionLoad(selected)
	}
	return Actions{}
}

// SelectSession validates catalog membership and starts a new session
// generation. A late result from any earlier selection cannot pass ApplyHydrate.
func (s *State) SelectSession(sessionID string) Actions {
	if s.closed || s.Status != Connected || !sessionExists(s.Sessions, sessionID) {
		return Actions{}
	}
	if s.SelectedSessionID == sessionID &&
		(s.SessionLoading || s.hydratedSessionID == sessionID) {
		return Actions{}
	}
	return s.beginSessionLoad(sessionID)
}

func (s *State) beginSessionLoad(sessionID string) Actions {
	s.saveActiveDraft()
	s.sendingText = ""
	s.sessionGeneration++
	s.resetSessionProjection()
	s.SelectedSessionID = sessionID
	s.restoreActiveDraft()
	s.SessionLoading = true
	s.SessionError = ""
	s.Sending = false
	s.SendError = ""
	return Actions{
		HydrateSessionID: sessionID, Generation: s.generation,
		SessionGeneration: s.sessionGeneration,
	}
}

func (s *State) resetSessionProjection() {
	s.hydratedGeneration = 0
	s.hydratedSessionID = ""
	s.rehydrating = false
	s.pendingEvents = nil
	s.eventCursor = nil
	s.attachedRuntimeID = ""
	s.SessionLoading = false
	s.SessionError = ""
	s.MessageCount = 0
	s.Transcript = nil
}

// ApplyAttach records the runtime only for the selection that requested it.
func (s *State) ApplyAttach(result AttachResult) bool {
	if !s.sessionCurrent(result.Generation, result.SessionGeneration, result.SessionID) {
		return false
	}
	if result.Runtime != nil {
		s.attachedRuntimeID = result.Runtime.RuntimeID
	}
	return true
}

// ApplyHydrate installs an authoritative snapshot, then replays only buffered
// events newer than its watermark and belonging to the attached runtime.
func (s *State) ApplyHydrate(result HydrateResult) bool {
	if !s.sessionCurrent(result.Generation, result.SessionGeneration, result.SessionID) {
		return false
	}
	if result.Err != nil || result.Session == nil {
		s.SessionLoading = false
		if result.Err != nil {
			s.SessionError = result.Err.Error()
		} else {
			s.SessionError = "client-daemon 返回了空会话"
		}
		s.rehydrating = false
		return true
	}
	s.SessionLoading = false
	s.SessionError = ""
	s.MessageCount = result.MessageCount
	s.Transcript = result.Transcript
	s.hydratedGeneration = result.SessionGeneration
	s.hydratedSessionID = result.SessionID
	s.rehydrating = false
	// The selection may have changed while I/O was in flight. The guard above
	// rejects stale writes; restore only from the still-current selection.
	s.restoreActiveDraft()
	if result.Session.RuntimeEventCursor != nil {
		cursor := *result.Session.RuntimeEventCursor
		s.eventCursor = &cursor
	}
	pending := s.pendingEvents
	s.pendingEvents = nil
	for _, event := range pending {
		if event.SessionID == result.SessionID &&
			(s.attachedRuntimeID == "" || s.attachedRuntimeID == event.RuntimeID) {
			s.applyTranscriptEvent(event)
		}
	}
	return true
}

// ApplyEvent applies a live event or buffers it until hydration. Overflow drops
// the untrustworthy buffer and requests one authoritative rehydrate; subsequent
// overflows during that call are dropped without restarting it.
func (s *State) ApplyEvent(result EventResult) Actions {
	event := result.Event
	if !s.sessionCurrent(result.Generation, s.sessionGeneration, event.SessionID) ||
		(s.attachedRuntimeID != "" && s.attachedRuntimeID != event.RuntimeID) {
		return Actions{}
	}
	if s.hydratedGeneration == s.sessionGeneration && s.hydratedSessionID == event.SessionID {
		s.applyTranscriptEvent(event)
		return Actions{}
	}
	if len(s.pendingEvents) >= PendingEventLimit {
		s.pendingEvents = nil
		if s.rehydrating {
			return Actions{EventBufferOverflow: true}
		}
		actions := s.beginSessionLoad(event.SessionID)
		actions.EventBufferOverflow = true
		actions.Rehydrate = true
		// beginSessionLoad clears the previous attempt's flag. Set ours only after
		// the replacement action exists, so later overflows only drop and report.
		s.rehydrating = true
		return actions
	}
	s.pendingEvents = append(s.pendingEvents, event)
	return Actions{}
}

func (s *State) applyTranscriptEvent(event daemon.SequencedEvent) {
	if cursor := s.eventCursor; cursor != nil &&
		cursor.RuntimeID == event.RuntimeID && cursor.Epoch == event.Epoch &&
		event.Sequence <= cursor.Sequence {
		return
	}
	s.Transcript = ApplyTranscriptEvent(s.Transcript, event)
	s.eventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: event.RuntimeID,
		Epoch:     event.Epoch,
		Sequence:  event.Sequence,
	}
}

// SetDraft changes only the active editor value. Per-session persistence happens
// at explicit transitions, matching the native text area's existing lifecycle.
func (s *State) SetDraft(draft string) {
	s.Draft = draft
}

func (s *State) saveActiveDraft() {
	sessionID := s.SelectedSessionID
	if sessionID == "" {
		return
	}
	draft := s.Draft
	if s.Sending && s.sendingText != "" {
		draft = s.sendingText
	}
	s.setSessionDraft(sessionID, draft)
}

func (s *State) restoreActiveDraft() {
	s.Draft = s.drafts[s.SelectedSessionID]
}

func (s *State) setSessionDraft(sessionID, draft string) {
	if sessionID == "" {
		return
	}
	if draft == "" {
		delete(s.drafts, sessionID)
		return
	}
	if s.drafts == nil {
		s.drafts = make(map[string]string)
	}
	s.drafts[sessionID] = draft
}

func (s *State) pruneDrafts(sessions []SessionSummary) {
	for sessionID := range s.drafts {
		if !sessionExists(sessions, sessionID) {
			delete(s.drafts, sessionID)
		}
	}
}

func (s *State) clearSelection() {
	s.saveActiveDraft()
	s.sendingText = ""
	s.sessionGeneration++
	s.resetSessionProjection()
	s.SelectedSessionID = ""
	s.restoreActiveDraft()
	s.Sending = false
	s.SendError = ""
}

// BeginSend validates the hydrated selection and moves the exact input into an
// in-flight draft. The visible editor is cleared until the prompt finishes.
func (s *State) BeginSend(text string) (SendAction, bool) {
	if strings.TrimSpace(text) == "" {
		return SendAction{}, false
	}
	sessionID := s.SelectedSessionID
	if s.closed || s.Status != Connected || s.Sending || sessionID == "" ||
		s.hydratedGeneration != s.sessionGeneration || s.hydratedSessionID != sessionID {
		return SendAction{}, false
	}
	s.sendingText = text
	s.Draft = ""
	s.setSessionDraft(sessionID, "")
	s.Sending = true
	s.SendError = ""
	return SendAction{
		Generation: s.generation, SessionGeneration: s.sessionGeneration,
		SessionID: sessionID, RuntimeID: s.attachedRuntimeID, Text: text,
	}, true
}

// FinishSend completes only the still-current session's send. Failure restores
// the exact input; success also clears the saved in-flight per-session draft.
func (s *State) FinishSend(result SendResult) bool {
	if !s.sessionCurrent(result.Generation, result.SessionGeneration, result.SessionID) {
		return false
	}
	s.sendingText = ""
	s.Sending = false
	if result.Err != nil {
		s.Draft = result.Text
		s.setSessionDraft(result.SessionID, result.Text)
		s.SendError = sendErrorMessage(result.Err)
	} else {
		delete(s.drafts, result.SessionID)
		s.SendError = ""
	}
	return true
}

func sendErrorMessage(err error) string {
	detail := strings.Join(strings.Fields(err.Error()), " ")
	if detail == "" {
		detail = "未知错误"
	}
	const detailLimit = 240
	runes := []rune(detail)
	if len(runes) > detailLimit {
		detail = string(runes[:detailLimit]) + "…"
	}
	return fmt.Sprintf("发送失败，已保留输入：%s", detail)
}

func (s *State) ConnectionCurrent(generation uint64) bool {
	return !s.closed && generation == s.generation
}

// PendingEventCount and Rehydrating expose read-only progress for headless
// integration tests and diagnostics; they do not bypass projection commands.
func (s *State) PendingEventCount() int { return len(s.pendingEvents) }
func (s *State) Rehydrating() bool      { return s.rehydrating }

// sessionCurrent also requires catalog membership: a deletion can race a late
// callback while both generation tokens and the selected ID still match.
func (s *State) sessionCurrent(generation, sessionGeneration uint64, sessionID string) bool {
	return s.ConnectionCurrent(generation) &&
		sessionGeneration == s.sessionGeneration && s.SelectedSessionID == sessionID &&
		sessionExists(s.Sessions, sessionID)
}

func sessionExists(sessions []SessionSummary, sessionID string) bool {
	for _, session := range sessions {
		if session.ID == sessionID {
			return true
		}
	}
	return false
}

// Close invalidates all future result deliveries without changing the last
// render snapshot. The returned generation is mirrored by shell I/O ownership.
func (s *State) Close() uint64 {
	if s.closed {
		return s.generation
	}
	s.closed = true
	s.generation++
	s.sessionGeneration++
	return s.generation
}
