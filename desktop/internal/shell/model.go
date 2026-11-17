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

// Model owns connection and workbench state. Network and process work always
// runs off the UI thread; frames only read the in-memory snapshot.
type Model struct {
	connector Connector

	mu                   sync.Mutex
	services             ui.Services
	snapshot             Snapshot
	connection           daemon.Connection
	generation           uint64
	sessionGeneration    uint64
	hydratedGeneration   uint64
	hydratedSessionID    string
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
	ctx, generation, ok := m.beginConnect()
	if !ok {
		return
	}
	go m.connect(ctx, generation)
}

func (m *Model) beginConnect() (context.Context, uint64, bool) {
	m.mu.Lock()
	ctx, generation, ok := m.beginConnectLocked()
	m.mu.Unlock()
	if ok {
		m.invalidate()
	}
	return ctx, generation, ok
}

// reconnect invalidates a disconnected watcher and starts its replacement in
// one critical section, so a stale watcher cannot cancel a newer connection.
func (m *Model) reconnect(generation uint64) {
	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) {
		m.mu.Unlock()
		return
	}
	ctx, nextGeneration, ok := m.beginConnectLocked()
	m.mu.Unlock()
	if !ok {
		return
	}
	m.invalidate()
	go m.connect(ctx, nextGeneration)
}

func (m *Model) beginConnectLocked() (context.Context, uint64, bool) {
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
	m.saveActiveDraftLocked()
	m.sendingText = ""
	m.generation++
	m.sessionGeneration++
	m.hydratedGeneration = 0
	m.hydratedSessionID = ""
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
	m.restoreActiveDraftLocked()
	return ctx, generation, true
}

func (m *Model) connect(ctx context.Context, generation uint64) {
	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) || ctx.Err() != nil {
		m.mu.Unlock()
		return
	}
	previous := m.connection
	m.mu.Unlock()

	var resume []daemon.ReplayCursor
	if previous != nil {
		resume = previous.ReplayCursors()
		if len(resume) == 0 {
			resume = nil
		}
	}
	connection, err := m.connector.Connect(ctx, resume)
	var replaced daemon.Connection

	m.mu.Lock()
	if m.closed || generation != m.generation {
		m.mu.Unlock()
		if connection != nil {
			_ = connection.Close()
		}
		return
	}
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
		m.snapshot.DaemonVersion = connection.DaemonVersion()
		m.snapshot.Detail = ""
		m.snapshot.SessionsLoading = true
		m.snapshot.SessionsError = ""
		m.snapshot.SessionLoading = false
		m.snapshot.SessionError = ""
	}
	m.mu.Unlock()
	if replaced != nil {
		_ = replaced.Close()
	}
	m.invalidate()
	if err == nil && connection != nil {
		go m.loadSessions(ctx, generation, connection)
		go m.watchConnection(ctx, generation, connection)
	}
}

func (m *Model) loadSessions(ctx context.Context, generation uint64, connection daemon.Connection) {
	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) {
		m.mu.Unlock()
		return
	}
	selectedBeforeLoad := m.snapshot.SelectedSessionID
	sessionGenerationBeforeLoad := m.sessionGeneration
	m.mu.Unlock()

	state, err := connection.LoadTaskState(ctx)

	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) {
		m.mu.Unlock()
		return
	}
	m.snapshot.SessionsLoading = false
	if err != nil {
		m.snapshot.SessionsError = err.Error()
		m.mu.Unlock()
		m.invalidate()
		return
	}
	m.snapshot.Sessions = append([]SessionSummary(nil), state.Sessions...)
	m.snapshot.SessionsError = ""
	selected := m.snapshot.SelectedSessionID
	if selected != "" && !sessionExists(m.snapshot.Sessions, selected) {
		m.clearSelectionLocked()
		selected = ""
	}
	rehydrate := selected != "" && selected == selectedBeforeLoad &&
		m.sessionGeneration == sessionGenerationBeforeLoad
	m.mu.Unlock()
	m.invalidate()

	if rehydrate {
		m.startSessionLoad(generation, connection, selected)
	}
}

// SelectSession starts attach and hydrate asynchronously. Its session
// generation prevents a late response for an earlier selection from landing.
func (m *Model) SelectSession(sessionID string) {
	m.mu.Lock()
	if m.closed || m.snapshot.Status != Connected || m.connection == nil ||
		!sessionExists(m.snapshot.Sessions, sessionID) {
		m.mu.Unlock()
		return
	}
	if m.snapshot.SelectedSessionID == sessionID &&
		(m.snapshot.SessionLoading || m.hydratedSessionID == sessionID) {
		m.mu.Unlock()
		return
	}
	ctx, generation, sessionGeneration, connection := m.startSessionLoadLocked(sessionID)
	m.mu.Unlock()
	m.invalidate()
	go m.hydrateSession(ctx, generation, sessionGeneration, connection, sessionID)
}

func (m *Model) startSessionLoad(generation uint64, connection daemon.Connection, sessionID string) {
	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) ||
		m.snapshot.SelectedSessionID != sessionID ||
		!sessionExists(m.snapshot.Sessions, sessionID) {
		m.mu.Unlock()
		return
	}
	ctx, generation, sessionGeneration, connection := m.startSessionLoadLocked(sessionID)
	m.mu.Unlock()
	m.invalidate()
	go m.hydrateSession(ctx, generation, sessionGeneration, connection, sessionID)
}

func (m *Model) startSessionLoadLocked(sessionID string) (context.Context, uint64, uint64, daemon.Connection) {
	m.saveActiveDraftLocked()
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
	m.pendingEvents = nil
	m.eventCursor = nil
	m.attachedRuntimeID = ""
	ctx, cancel := context.WithCancel(context.Background())
	m.sessionCancel = cancel
	m.snapshot.SelectedSessionID = sessionID
	m.restoreActiveDraftLocked()
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

	m.mu.Lock()
	if !m.sessionCurrentLocked(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
		m.mu.Unlock()
		return
	}
	if runtime != nil {
		m.attachedRuntimeID = runtime.RuntimeID
	}
	m.mu.Unlock()

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

	m.mu.Lock()
	if !m.sessionCurrentLocked(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
		m.mu.Unlock()
		return
	}
	m.snapshot.SessionLoading = false
	m.snapshot.SessionError = ""
	m.snapshot.MessageCount = messageCount
	m.snapshot.Transcript = transcript
	m.hydratedGeneration = sessionGeneration
	m.hydratedSessionID = sessionID
	if session.RuntimeEventCursor != nil {
		cursor := *session.RuntimeEventCursor
		m.eventCursor = &cursor
	}
	pending := m.pendingEvents
	m.pendingEvents = nil
	for _, event := range pending {
		if event.SessionID == sessionID &&
			(m.attachedRuntimeID == "" || m.attachedRuntimeID == event.RuntimeID) {
			m.applyTranscriptEventLocked(event)
		}
	}
	m.mu.Unlock()
	m.invalidate()
}

func (m *Model) finishSessionError(ctx context.Context, generation, sessionGeneration uint64, sessionID string, err error) {
	m.mu.Lock()
	if !m.sessionCurrentLocked(generation, sessionGeneration, sessionID) || ctx.Err() != nil {
		m.mu.Unlock()
		return
	}
	m.snapshot.SessionLoading = false
	m.snapshot.SessionError = err.Error()
	m.mu.Unlock()
	m.invalidate()
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
			m.reconnect(generation)
			return
		}
	}
}

func (m *Model) applyEvent(generation uint64, event daemon.SequencedEvent) {
	m.mu.Lock()
	if !m.connectionCurrentLocked(generation) ||
		m.snapshot.SelectedSessionID != event.SessionID ||
		(m.attachedRuntimeID != "" && m.attachedRuntimeID != event.RuntimeID) {
		m.mu.Unlock()
		return
	}
	if m.hydratedGeneration == m.sessionGeneration && m.hydratedSessionID == event.SessionID {
		m.applyTranscriptEventLocked(event)
		m.mu.Unlock()
		m.invalidate()
		return
	}
	if len(m.pendingEvents) >= pendingEventLimit {
		m.pendingEvents = nil
		connection := m.connection
		m.mu.Unlock()
		log.Printf("WARN desktop: hydrate event buffer overflow; forcing session rehydrate")
		m.startSessionLoad(generation, connection, event.SessionID)
		return
	}
	m.pendingEvents = append(m.pendingEvents, event)
	m.mu.Unlock()
}

func (m *Model) applyTranscriptEventLocked(event daemon.SequencedEvent) {
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

func (m *Model) saveActiveDraftLocked() {
	sessionID := m.snapshot.SelectedSessionID
	if sessionID == "" {
		return
	}
	draft := m.draft
	if m.snapshot.Sending && m.sendingText != "" {
		draft = m.sendingText
	}
	m.setSessionDraftLocked(sessionID, draft)
}

func (m *Model) restoreActiveDraftLocked() {
	m.draft = m.drafts[m.snapshot.SelectedSessionID]
	m.snapshot.Draft = m.draft
}

func (m *Model) setSessionDraftLocked(sessionID, draft string) {
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

func (m *Model) clearSelectionLocked() {
	m.saveActiveDraftLocked()
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
	m.pendingEvents = nil
	m.eventCursor = nil
	m.attachedRuntimeID = ""
	m.snapshot.SelectedSessionID = ""
	m.restoreActiveDraftLocked()
	m.snapshot.SessionLoading = false
	m.snapshot.SessionError = ""
	m.snapshot.MessageCount = 0
	m.snapshot.Transcript = nil
	m.snapshot.Sending = false
	m.snapshot.SendError = ""
}

func (m *Model) connectionCurrentLocked(generation uint64) bool {
	return !m.closed && generation == m.generation
}

func (m *Model) sessionCurrentLocked(generation, sessionGeneration uint64, sessionID string) bool {
	return m.connectionCurrentLocked(generation) &&
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
	m.mu.Lock()
	m.services = c.Services()
	m.snapshot.Draft = m.draft
	snapshot := cloneSnapshot(m.snapshot)
	m.mu.Unlock()

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

// Snapshot returns a race-free copy for logic tests and embedding code.
func (m *Model) Snapshot() Snapshot {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.snapshot.Draft = m.draft
	return cloneSnapshot(m.snapshot)
}

func (m *Model) invalidate() {
	m.mu.Lock()
	services := m.services
	m.mu.Unlock()
	services.Invalidate()
}

func (m *Model) Close() error {
	m.mu.Lock()
	if m.closed {
		m.mu.Unlock()
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
	m.mu.Unlock()
	if err := m.connector.Close(); err != nil {
		return fmt.Errorf("could not close client-daemon connection: %w", err)
	}
	return nil
}
