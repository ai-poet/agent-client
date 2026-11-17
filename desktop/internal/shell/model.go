// Package shell contains the native MyGo desktop shell's testable state and
// views. Network and process work stays outside the UI thread.
package shell

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

const pendingEventLimit = 256

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
// not a second wire type owned by the UI.
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

// Connector is implemented by daemon.Supervisor and by headless test fakes.
type Connector interface {
	Connect(context.Context, []daemon.ReplayCursor) (daemon.Connection, error)
	Close() error
}

// pendingResults is the Model's only cross-goroutine state. Background workers
// append immutable results; the UI thread drains and applies them.
type pendingResults struct {
	mu       sync.Mutex
	results  []func()
	services ui.Services
}

// Model owns connection and workbench state. Except for pending, every field
// is confined to the UI thread. Network and process work always runs off it.
type Model struct {
	connector Connector
	pending   pendingResults

	snapshot             Snapshot
	connection           daemon.Connection
	generation           uint64
	sessionGeneration    uint64
	hydratedGeneration   uint64
	hydratedSessionID    string
	rehydrating          bool
	pendingEvents        []daemon.SequencedEvent
	eventCursor          *daemon.RuntimeEventCursor
	attachedRuntimeID    string
	cancel               context.CancelFunc
	sessionCancel        context.CancelFunc
	sendCancel           context.CancelFunc
	sendingText          string
	drafts               map[string]string
	draft                string
	closed               bool
	sessionList          ui.ListState
	selectedSessionIndex int
	transcriptList       ui.ListState
	transcriptSessionID  string
	markdown             markdown.Options
}

func NewModel(connector Connector) *Model {
	return &Model{
		connector:            connector,
		snapshot:             Snapshot{Status: Disconnected},
		drafts:               make(map[string]string),
		selectedSessionIndex: -1,
	}
}

// Connect starts one asynchronous connection attempt. A generation counter
// prevents a superseded attempt from replacing newer state.
func (m *Model) Connect() {
	previous := m.connection
	ctx, generation, ok := m.beginConnect()
	if !ok {
		return
	}
	go m.connect(ctx, generation, previous)
}

func (m *Model) beginConnect() (context.Context, uint64, bool) {
	ctx, generation, ok := m.beginConnectState()
	if ok {
		m.invalidate()
	}
	return ctx, generation, ok
}

// reconnect runs on the UI thread. Validating and starting the replacement in
// one turn prevents a stale watcher from cancelling a newer connection.
func (m *Model) reconnect(generation uint64) {
	if !m.connectionCurrent(generation) {
		return
	}
	previous := m.connection
	ctx, nextGeneration, ok := m.beginConnectState()
	if !ok {
		return
	}
	m.invalidate()
	go m.connect(ctx, nextGeneration, previous)
}

func (m *Model) beginConnectState() (context.Context, uint64, bool) {
	if m.closed {
		return nil, 0, false
	}
	if m.cancel != nil {
		m.cancel()
	}
	if m.sessionCancel != nil {
		m.sessionCancel()
		m.sessionCancel = nil
	}
	if m.sendCancel != nil {
		m.sendCancel()
		m.sendCancel = nil
	}
	m.saveActiveDraft()
	m.sendingText = ""
	m.generation++
	m.sessionGeneration++
	m.hydratedGeneration = 0
	m.hydratedSessionID = ""
	m.rehydrating = false
	m.pendingEvents = nil
	m.eventCursor = nil
	m.attachedRuntimeID = ""
	generation := m.generation
	ctx, cancel := context.WithCancel(context.Background())
	m.cancel = cancel
	m.snapshot.Status = Connecting
	m.snapshot.Detail = ""
	m.snapshot.DaemonVersion = ""
	m.snapshot.SessionsLoading = false
	m.snapshot.SessionsError = ""
	m.snapshot.SessionLoading = false
	m.snapshot.SessionError = ""
	m.snapshot.MessageCount = 0
	m.snapshot.Transcript = nil
	m.snapshot.Sending = false
	m.snapshot.SendError = ""
	m.restoreActiveDraft()
	return ctx, generation, true
}

func (m *Model) connect(ctx context.Context, generation uint64, previous daemon.Connection) {
	if ctx.Err() != nil {
		return
	}
	var resume []daemon.ReplayCursor
	if previous != nil {
		resume = previous.ReplayCursors()
		if len(resume) == 0 {
			resume = nil
		}
	}
	connection, err := m.connector.Connect(ctx, resume)
	version := ""
	if connection != nil {
		version = connection.DaemonVersion()
	}

	m.enqueue(func() {
		if m.closed || generation != m.generation {
			go closeConnection(connection)
			return
		}
		var replaced daemon.Connection
		if err != nil {
			if m.cancel != nil {
				m.cancel()
				m.cancel = nil
			}
			m.snapshot.Status = Failed
			m.snapshot.Detail = err.Error()
			m.snapshot.SessionsLoading = false
		} else if connection == nil {
			if m.cancel != nil {
				m.cancel()
				m.cancel = nil
			}
			m.snapshot.Status = Failed
			m.snapshot.Detail = "client-daemon 返回了空连接"
			m.snapshot.SessionsLoading = false
		} else {
			m.connection = connection
			if previous != nil && previous != connection {
				replaced = previous
			}
			m.snapshot.Status = Connected
			m.snapshot.DaemonVersion = version
			m.snapshot.Detail = ""
			m.snapshot.SessionsLoading = true
			m.snapshot.SessionsError = ""
			m.snapshot.SessionLoading = false
			m.snapshot.SessionError = ""
		}
		if replaced != nil {
			go closeConnection(replaced)
		}
		if err == nil && connection != nil {
			go m.loadSessions(ctx, generation, connection, m.snapshot.SelectedSessionID, m.sessionGeneration)
			go m.watchConnection(ctx, generation, connection)
		}
	})
}

func closeConnection(connection daemon.Connection) {
	if connection != nil {
		_ = connection.Close()
	}
}

func (m *Model) loadSessions(ctx context.Context, generation uint64, connection daemon.Connection, selectedBeforeLoad string, sessionGenerationBeforeLoad uint64) {
	if ctx.Err() != nil {
		return
	}
	state, err := connection.LoadTaskState(ctx)

	m.enqueue(func() {
		if !m.connectionCurrent(generation) {
			return
		}
		m.snapshot.SessionsLoading = false
		if err != nil {
			m.snapshot.SessionsError = err.Error()
			return
		}
		m.snapshot.Sessions = append([]SessionSummary(nil), state.Sessions...)
		// Selection is limited to this list, so successful refreshes are a simple
		// bounded-drain point for drafts whose sessions no longer exist.
		m.pruneDrafts(m.snapshot.Sessions)
		m.snapshot.SessionsError = ""
		selected := m.snapshot.SelectedSessionID
		if selected != "" && !sessionExists(m.snapshot.Sessions, selected) {
			m.clearSelection()
			selected = ""
		}
		if selected != "" && selected == selectedBeforeLoad &&
			m.sessionGeneration == sessionGenerationBeforeLoad {
			m.startSessionLoad(generation, connection, selected)
		}
	})
}

// SelectSession starts attach and hydrate asynchronously. Its session
// generation prevents a late response for an earlier selection from landing.
func (m *Model) SelectSession(sessionID string) {
	if m.closed || m.snapshot.Status != Connected || m.connection == nil ||
		!sessionExists(m.snapshot.Sessions, sessionID) {
		return
	}
	if m.snapshot.SelectedSessionID == sessionID &&
		(m.snapshot.SessionLoading || m.hydratedSessionID == sessionID) {
		return
	}
	ctx, generation, sessionGeneration, connection := m.beginSessionLoad(sessionID)
	m.invalidate()
	go m.hydrateSession(ctx, generation, sessionGeneration, connection, sessionID)
}

func (m *Model) startSessionLoad(generation uint64, connection daemon.Connection, sessionID string) {
	if !m.connectionCurrent(generation) ||
		m.snapshot.SelectedSessionID != sessionID ||
		!sessionExists(m.snapshot.Sessions, sessionID) {
		return
	}
	ctx, generation, sessionGeneration, connection := m.beginSessionLoad(sessionID)
	m.invalidate()
	go m.hydrateSession(ctx, generation, sessionGeneration, connection, sessionID)
}

func (m *Model) beginSessionLoad(sessionID string) (context.Context, uint64, uint64, daemon.Connection) {
	m.saveActiveDraft()
	if m.sessionCancel != nil {
		m.sessionCancel()
	}
	if m.sendCancel != nil {
		m.sendCancel()
		m.sendCancel = nil
	}
	m.sendingText = ""
	m.sessionGeneration++
	m.hydratedGeneration = 0
	m.hydratedSessionID = ""
	m.rehydrating = false
	m.pendingEvents = nil
	m.eventCursor = nil
	m.attachedRuntimeID = ""
	ctx, cancel := context.WithCancel(context.Background())
	m.sessionCancel = cancel
	m.snapshot.SelectedSessionID = sessionID
	m.restoreActiveDraft()
	m.snapshot.SessionLoading = true
	m.snapshot.SessionError = ""
	m.snapshot.MessageCount = 0
	m.snapshot.Transcript = nil
	m.snapshot.Sending = false
	m.snapshot.SendError = ""
	return ctx, m.generation, m.sessionGeneration, m.connection
}

func (m *Model) hydrateSession(ctx context.Context, generation, sessionGeneration uint64, connection daemon.Connection, sessionID string) {
	runtime, err := connection.AttachSession(ctx, sessionID)
	if err != nil {
		m.finishSessionError(ctx, generation, sessionGeneration, sessionID, err)
		return
	}
	if runtime != nil {
		runtimeID := runtime.RuntimeID
		m.enqueue(func() {
			if m.sessionCurrent(generation, sessionGeneration, sessionID) && ctx.Err() == nil {
				m.attachedRuntimeID = runtimeID
			}
		})
	}
	if ctx.Err() != nil {
		return
	}

	session, err := connection.HydrateSession(ctx, sessionID)
	if err != nil {
		m.finishSessionError(ctx, generation, sessionGeneration, sessionID, err)
		return
	}
	if session == nil {
		m.finishSessionError(ctx, generation, sessionGeneration, sessionID, fmt.Errorf("client-daemon 返回了空会话"))
		return
	}
	transcript := projectTranscript(session)
	messageCount := len(session.Messages)

	m.enqueue(func() {
		if !m.sessionCurrent(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
			return
		}
		m.snapshot.SessionLoading = false
		m.snapshot.SessionError = ""
		m.snapshot.MessageCount = messageCount
		m.snapshot.Transcript = transcript
		m.hydratedGeneration = sessionGeneration
		m.hydratedSessionID = sessionID
		m.rehydrating = false
		// The selection may have changed while I/O was in flight. The guard above
		// rejects stale writes; restore only from the still-current selection.
		m.restoreActiveDraft()
		if session.RuntimeEventCursor != nil {
			cursor := *session.RuntimeEventCursor
			m.eventCursor = &cursor
		}
		pending := m.pendingEvents
		m.pendingEvents = nil
		for _, event := range pending {
			if event.SessionID == sessionID &&
				(m.attachedRuntimeID == "" || m.attachedRuntimeID == event.RuntimeID) {
				m.applyTranscriptEvent(event)
			}
		}
	})
}

func (m *Model) finishSessionError(ctx context.Context, generation, sessionGeneration uint64, sessionID string, err error) {
	m.enqueue(func() {
		if !m.sessionCurrent(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
			return
		}
		m.snapshot.SessionLoading = false
		m.snapshot.SessionError = err.Error()
		m.rehydrating = false
	})
}

func (m *Model) watchConnection(ctx context.Context, generation uint64, connection daemon.Connection) {
	events := connection.Events()
	for {
		select {
		case <-ctx.Done():
			return
		case event, ok := <-events:
			if !ok {
				events = nil
				continue
			}
			m.applyEvent(generation, event)
		case <-connection.Done():
			m.enqueue(func() { m.reconnect(generation) })
			return
		}
	}
}

// applyEvent may be called by a watcher goroutine; only its queued closure
// touches the Model, and drainPending runs that closure on the UI thread.
func (m *Model) applyEvent(generation uint64, event daemon.SequencedEvent) {
	m.enqueue(func() {
		if !m.connectionCurrent(generation) ||
			m.snapshot.SelectedSessionID != event.SessionID ||
			(m.attachedRuntimeID != "" && m.attachedRuntimeID != event.RuntimeID) {
			return
		}
		if m.hydratedGeneration == m.sessionGeneration && m.hydratedSessionID == event.SessionID {
			m.applyTranscriptEvent(event)
			return
		}
		if len(m.pendingEvents) >= pendingEventLimit {
			m.pendingEvents = nil
			if m.rehydrating {
				log.Printf("WARN desktop: hydrate event buffer overflow during rehydrate; dropping buffered events")
				return
			}
			connection := m.connection
			log.Printf("WARN desktop: hydrate event buffer overflow; forcing session rehydrate")
			m.startSessionLoad(generation, connection, event.SessionID)
			// beginSessionLoad clears the previous attempt's flag. Set ours only
			// after the replacement is launched; its result cannot drain until
			// this UI-thread closure returns. Later overflows only log and drop.
			m.rehydrating = true
			return
		}
		m.pendingEvents = append(m.pendingEvents, event)
	})
}

func (m *Model) applyTranscriptEvent(event daemon.SequencedEvent) {
	if cursor := m.eventCursor; cursor != nil &&
		cursor.RuntimeID == event.RuntimeID && cursor.Epoch == event.Epoch &&
		event.Sequence <= cursor.Sequence {
		return
	}
	m.snapshot.Transcript = applyTranscriptEvent(m.snapshot.Transcript, event)
	m.eventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: event.RuntimeID,
		Epoch:     event.Epoch,
		Sequence:  event.Sequence,
	}
}

func (m *Model) saveActiveDraft() {
	sessionID := m.snapshot.SelectedSessionID
	if sessionID == "" {
		return
	}
	draft := m.draft
	if m.snapshot.Sending && m.sendingText != "" {
		draft = m.sendingText
	}
	m.setSessionDraft(sessionID, draft)
}

func (m *Model) restoreActiveDraft() {
	m.draft = m.drafts[m.snapshot.SelectedSessionID]
	m.snapshot.Draft = m.draft
}

func (m *Model) setSessionDraft(sessionID, draft string) {
	if sessionID == "" {
		return
	}
	if draft == "" {
		delete(m.drafts, sessionID)
		return
	}
	if m.drafts == nil {
		m.drafts = make(map[string]string)
	}
	m.drafts[sessionID] = draft
}

func (m *Model) pruneDrafts(sessions []SessionSummary) {
	for sessionID := range m.drafts {
		if !sessionExists(sessions, sessionID) {
			delete(m.drafts, sessionID)
		}
	}
}

func (m *Model) clearSelection() {
	m.saveActiveDraft()
	if m.sessionCancel != nil {
		m.sessionCancel()
		m.sessionCancel = nil
	}
	if m.sendCancel != nil {
		m.sendCancel()
		m.sendCancel = nil
	}
	m.sendingText = ""
	m.sessionGeneration++
	m.hydratedGeneration = 0
	m.hydratedSessionID = ""
	m.rehydrating = false
	m.pendingEvents = nil
	m.eventCursor = nil
	m.attachedRuntimeID = ""
	m.snapshot.SelectedSessionID = ""
	m.restoreActiveDraft()
	m.snapshot.SessionLoading = false
	m.snapshot.SessionError = ""
	m.snapshot.MessageCount = 0
	m.snapshot.Transcript = nil
	m.snapshot.Sending = false
	m.snapshot.SendError = ""
}

func (m *Model) connectionCurrent(generation uint64) bool {
	return !m.closed && generation == m.generation
}

func (m *Model) sessionCurrent(generation, sessionGeneration uint64, sessionID string) bool {
	return m.connectionCurrent(generation) &&
		sessionGeneration == m.sessionGeneration && m.snapshot.SelectedSessionID == sessionID
}

func sessionExists(sessions []SessionSummary, sessionID string) bool {
	for _, session := range sessions {
		if session.ID == sessionID {
			return true
		}
	}
	return false
}

// View draws native GPU UI through MyGo; there is no HTML or webview.
func (m *Model) View(c *ui.Context) {
	m.drainPending()
	m.setServices(c.Services())
	m.snapshot.Draft = m.draft
	snapshot := cloneSnapshot(m.snapshot)

	if snapshot.Status != Connected {
		m.connectionView(c, snapshot)
		return
	}
	m.workbenchView(c, snapshot)
}

func (m *Model) connectionView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	ui.Column(c).Fill().Center().Padding(32).Children(func() {
		ui.Column(c).Width(440).Padding(28).Gap(16).Background(theme.Surface).Radius(12).Children(func() {
			ui.Text(c, "CheapRouter").FontSize(28).Bold()
			ui.Text(c, "MyGo 原生桌面壳").TextColor(theme.TextMuted)
			ui.Text(c, snapshot.Status.Label()).FontSize(20).Bold().Role(ui.RoleStatus)

			switch snapshot.Status {
			case Failed:
				ui.Text(c, snapshot.Detail).TextColor(theme.Danger).Role(ui.RoleAlert)
			case Connecting:
				ui.Text(c, "正在与现有 client-daemon 握手…").TextColor(theme.TextMuted)
			default:
				ui.Text(c, "尚未连接到 client-daemon。").TextColor(theme.TextMuted)
			}

			if snapshot.Status == Failed || snapshot.Status == Disconnected {
				if ui.PrimaryButton(c.Key("retry"), "重新连接").Clicked() {
					m.Connect()
				}
			}
		})
	})
}

func (m *Model) workbenchView(c *ui.Context, snapshot Snapshot) {
	theme := c.Theme()
	ui.Row(c).Fill().Children(func() {
		ui.Column(c).Width(300).Fill().Padding(16).Gap(12).Background(theme.Surface).Children(func() {
			ui.Text(c, "CheapRouter").FontSize(22).Bold()
			ui.Row(c).Gap(8).Children(func() {
				ui.Text(c, "会话").FontSize(18).Bold().Grow(1)
				ui.Text(c, snapshot.Status.Label()).FontSize(12).TextColor(theme.TextMuted).Role(ui.RoleStatus)
			})
			ui.Textf(c, "client-daemon 版本：%s", snapshot.DaemonVersion).FontSize(11).TextColor(theme.TextMuted)

			switch {
			case snapshot.SessionsLoading:
				ui.Text(c, "正在加载会话…").TextColor(theme.TextMuted).Role(ui.RoleStatus)
			case snapshot.SessionsError != "":
				ui.Textf(c, "会话加载失败：%s", snapshot.SessionsError).TextColor(theme.Danger).Role(ui.RoleAlert)
			case len(snapshot.Sessions) == 0:
				ui.Text(c, "还没有会话").TextColor(theme.TextMuted)
			default:
				m.sessionListView(c, snapshot)
			}
		})

		ui.Column(c).Grow(1).Fill().Children(func() {
			if snapshot.SelectedSessionID == "" {
				ui.Column(c).Fill().Center().Children(func() {
					ui.Text(c, "请选择一个会话").TextColor(theme.TextMuted)
				})
				return
			}
			ui.Row(c).Padding(16, 24).BorderWidth(0, 0, 1, 0).BorderColor(theme.Border).Children(func() {
				ui.Text(c, selectedSessionName(snapshot)).FontSize(18).Bold()
			})
			m.transcriptView(c, snapshot)
			m.composerView(c, snapshot)
		})
	})
}

func (m *Model) sessionListView(c *ui.Context, snapshot Snapshot) {
	m.selectedSessionIndex = -1
	for index, session := range snapshot.Sessions {
		if session.ID == snapshot.SelectedSessionID {
			m.selectedSessionIndex = index
			break
		}
	}
	m.sessionList.Selected = &m.selectedSessionIndex
	m.sessionList.Key = func(index int) any { return snapshot.Sessions[index].ID }
	m.sessionList.Label = func(index int) string { return sessionName(snapshot.Sessions[index]) }
	list := ui.List(c, &m.sessionList, len(snapshot.Sessions), func(index int) {
		session := snapshot.Sessions[index]
		ui.Column(c).Padding(10, 12).Gap(4).Children(func() {
			ui.Text(c, sessionName(session)).Bold()
			ui.Text(c, sessionLastMessageTime(session)).FontSize(12)
		})
	}).Grow(1).Label("会话列表")
	if list.Changed() && m.selectedSessionIndex >= 0 && m.selectedSessionIndex < len(snapshot.Sessions) {
		m.SelectSession(snapshot.Sessions[m.selectedSessionIndex].ID)
	}
}

func selectedSessionName(snapshot Snapshot) string {
	for _, session := range snapshot.Sessions {
		if session.ID == snapshot.SelectedSessionID {
			return sessionName(session)
		}
	}
	return "未命名会话"
}

func sessionName(session SessionSummary) string {
	if title := strings.TrimSpace(session.Title); title != "" {
		return title
	}
	if session.AutoTitle != nil {
		if title := strings.TrimSpace(*session.AutoTitle); title != "" {
			return title
		}
	}
	return "未命名会话"
}

func sessionLastMessageTime(session SessionSummary) string {
	if session.LastReplyAt == nil {
		return "最后消息：暂无"
	}
	return "最后消息：" + time.Unix(int64(*session.LastReplyAt), 0).Local().Format("2006-01-02 15:04")
}

func cloneSnapshot(snapshot Snapshot) Snapshot {
	snapshot.Sessions = append([]SessionSummary(nil), snapshot.Sessions...)
	snapshot.Transcript = append([]TranscriptEntry(nil), snapshot.Transcript...)
	return snapshot
}

// Snapshot applies completed work and returns a copy. Like View, it must be
// called from the UI thread; background workers only use enqueue.
func (m *Model) Snapshot() Snapshot {
	m.drainPending()
	m.snapshot.Draft = m.draft
	return cloneSnapshot(m.snapshot)
}

// enqueue is the only Model method a background worker may use to deliver
// state. Invalidate is explicitly goroutine-safe in MyGo.
func (m *Model) enqueue(apply func()) {
	m.pending.mu.Lock()
	m.pending.results = append(m.pending.results, apply)
	services := m.pending.services
	m.pending.mu.Unlock()
	services.Invalidate()
}

// drainPending applies completed I/O results on the UI thread. Testers reach
// it through View on every frame; Snapshot also drives headless state tests.
func (m *Model) drainPending() {
	m.pending.mu.Lock()
	pending := m.pending.results
	m.pending.results = nil
	services := m.pending.services
	m.pending.mu.Unlock()
	if len(pending) == 0 {
		return
	}
	for _, apply := range pending {
		apply()
	}
	services.Invalidate()
}

func (m *Model) setServices(services ui.Services) {
	m.pending.mu.Lock()
	m.pending.services = services
	m.pending.mu.Unlock()
}

func (m *Model) invalidate() {
	m.pending.mu.Lock()
	services := m.pending.services
	m.pending.mu.Unlock()
	services.Invalidate()
}

func (m *Model) Close() error {
	if m.closed {
		return nil
	}
	m.closed = true
	m.generation++
	m.sessionGeneration++
	if m.cancel != nil {
		m.cancel()
	}
	if m.sessionCancel != nil {
		m.sessionCancel()
	}
	if m.sendCancel != nil {
		m.sendCancel()
	}
	if err := m.connector.Close(); err != nil {
		return fmt.Errorf("could not close client-daemon connection: %w", err)
	}
	return nil
}
