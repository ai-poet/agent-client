// Package shell renders workbench.State through native MyGo views, translates
// input into projection commands, and drives all connection and RPC I/O.
package shell

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/ELY-Open-Source/CrychicUI/data"
	"github.com/ELY-Open-Source/CrychicUI/feedback"
	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/ai-poet/agent-client/desktop/internal/workbench"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// Connector is implemented by daemon.Supervisor and by headless test fakes.
type Connector interface {
	Connect(context.Context, []daemon.ReplayCursor) (daemon.Connection, error)
	Close() error
}

// pendingResults is the Model's only cross-goroutine result queue. Background
// workers append immutable deliveries; the UI thread drains and applies them.
type pendingResults struct {
	mu       sync.Mutex
	results  []func()
	services ui.Services
}

// ownedConnections tracks which connection the model owns, so that a replaced
// or superseded connection is closed on the connecting goroutine rather than
// queued behind a frame that may never come (headless windows never drain).
type ownedConnections struct {
	mu      sync.Mutex
	current daemon.Connection
}

func (o *ownedConnections) install(connection daemon.Connection) (old daemon.Connection) {
	o.mu.Lock()
	defer o.mu.Unlock()
	old = o.current
	o.current = connection
	return old
}

func (o *ownedConnections) take() daemon.Connection {
	o.mu.Lock()
	defer o.mu.Unlock()
	connection := o.current
	o.current = nil
	return connection
}

// Model owns I/O lifetime and native view state. All workbench projection
// fields live in state and are touched only by the UI thread. gen and closedFlag
// mirror I/O ownership tokens for goroutines that cannot wait for a frame drain.
type Model struct {
	connector Connector
	pending   pendingResults

	gen        atomic.Uint64
	closedFlag atomic.Bool
	owned      ownedConnections

	state                workbench.State
	connection           daemon.Connection
	cancel               context.CancelFunc
	sessionCancel        context.CancelFunc
	sendCancel           context.CancelFunc
	closed               bool
	selectedSessionIndex int
	transcriptList       ui.ListState
	transcriptSessionID  string
	markdown             markdown.Options
}

func NewModel(connector Connector) *Model {
	return &Model{
		connector:            connector,
		state:                workbench.NewState(),
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

// reconnect runs inside one drained UI callback, the connection-state critical
// section. Generation and identity validation, capturing previous, and starting
// its replacement are atomic with respect to every other Model state change.
// A watcher never closes m.connection: connect closes only its own stale result
// or the previous connection captured by this validated turn. Consequently a
// stale watcher cannot close a newer connection between a check and Close.
func (m *Model) reconnect(generation uint64, watched daemon.Connection) {
	if !m.connectionCurrent(generation) || m.connection != watched {
		return
	}
	previous := watched
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
	m.cancelSessionAndSend()
	generation, ok := m.state.BeginConnect()
	if !ok {
		return nil, 0, false
	}
	m.gen.Store(generation)
	ctx, cancel := context.WithCancel(context.Background())
	m.cancel = cancel
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

	// Ownership is decided here, off the UI thread: a superseded attempt closes
	// its own connection at once, and a successful install closes whatever it
	// replaced at once. Waiting for a queued callback would leak both whenever
	// no frame is drawn (tests, background windows, idle headless runs).
	stale := m.closedFlag.Load() || m.gen.Load() != generation
	if connection != nil {
		if stale || err != nil {
			go closeConnection(connection)
			connection = nil
		} else if replaced := m.owned.install(connection); replaced != nil && replaced != connection {
			go closeConnection(replaced)
		}
	}
	if stale {
		return
	}

	m.enqueue(func() {
		if m.closed || !m.state.ConnectionCurrent(generation) {
			return
		}
		if err == nil && connection == nil {
			err = fmt.Errorf("client-daemon 返回了空连接")
		}
		if err != nil {
			if m.cancel != nil {
				m.cancel()
				m.cancel = nil
			}
		} else {
			m.connection = connection
		}
		actions := m.state.ApplyConnect(workbench.ConnectResult{
			Generation: generation, DaemonVersion: version, Err: err,
		})
		if actions.LoadSessions {
			go m.loadSessions(ctx, generation, connection, actions.SelectedSessionID, actions.SessionGeneration)
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
		actions := m.state.ApplySessions(workbench.SessionsResult{
			Generation: generation, SessionGeneration: sessionGenerationBeforeLoad,
			SelectedSessionID: selectedBeforeLoad, Sessions: state.Sessions, Err: err,
		})
		if actions.SelectionCleared {
			m.cancelSessionAndSend()
		}
		m.startHydrate(actions)
	})
}

// SelectSession translates one user selection into a projection command, then
// drives the requested attach/hydrate I/O outside the UI thread.
func (m *Model) SelectSession(sessionID string) {
	if m.closed || m.connection == nil {
		return
	}
	m.startHydrate(m.state.SelectSession(sessionID))
}

func (m *Model) startHydrate(actions workbench.Actions) {
	if actions.HydrateSessionID == "" || m.closed ||
		!m.connectionCurrent(actions.Generation) || m.connection == nil {
		return
	}
	m.cancelSessionAndSend()
	ctx, cancel := context.WithCancel(context.Background())
	m.sessionCancel = cancel
	connection := m.connection
	m.invalidate()
	go m.hydrateSession(ctx, actions, connection)
}

func (m *Model) cancelSessionAndSend() {
	if m.sessionCancel != nil {
		m.sessionCancel()
		m.sessionCancel = nil
	}
	if m.sendCancel != nil {
		m.sendCancel()
		m.sendCancel = nil
	}
}

func (m *Model) hydrateSession(ctx context.Context, actions workbench.Actions, connection daemon.Connection) {
	sessionID := actions.HydrateSessionID
	runtime, err := connection.AttachSession(ctx, sessionID)
	if err != nil {
		m.deliverHydrate(ctx, workbench.NewHydrateResult(
			actions.Generation, actions.SessionGeneration, sessionID, nil, err,
		))
		return
	}
	if runtime != nil {
		m.enqueue(func() {
			if ctx.Err() == nil {
				m.state.ApplyAttach(workbench.AttachResult{
					Generation: actions.Generation, SessionGeneration: actions.SessionGeneration,
					SessionID: sessionID, Runtime: runtime,
				})
			}
		})
	}
	if ctx.Err() != nil {
		return
	}

	session, err := connection.HydrateSession(ctx, sessionID)
	result := workbench.NewHydrateResult(
		actions.Generation, actions.SessionGeneration, sessionID, session, err,
	)
	m.deliverHydrate(ctx, result)
}

func (m *Model) deliverHydrate(ctx context.Context, result workbench.HydrateResult) {
	m.enqueue(func() {
		if ctx.Err() == nil {
			m.state.ApplyHydrate(result)
		}
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
			// Carry the watcher's own token and connection into the serialized UI
			// turn. This goroutine neither inspects nor closes current Model state.
			m.enqueue(func() { m.reconnect(generation, connection) })
			return
		}
	}
}

// applyEvent may be called by a watcher goroutine; only its queued delivery
// touches workbench.State, and drainPending runs it on the UI thread.
func (m *Model) applyEvent(generation uint64, event daemon.SequencedEvent) {
	m.enqueue(func() {
		actions := m.state.ApplyEvent(workbench.EventResult{Generation: generation, Event: event})
		if actions.EventBufferOverflow {
			if actions.Rehydrate {
				log.Printf("WARN desktop: hydrate event buffer overflow; forcing session rehydrate")
			} else {
				log.Printf("WARN desktop: hydrate event buffer overflow during rehydrate; dropping buffered events")
			}
		}
		m.startHydrate(actions)
	})
}

func (m *Model) connectionCurrent(generation uint64) bool {
	return !m.closed && m.state.ConnectionCurrent(generation)
}

// View draws native GPU UI through MyGo; there is no HTML or webview.
func (m *Model) View(c *ui.Context) {
	m.drainPending()
	m.setServices(c.Services())
	snapshot := m.state.Snapshot()

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
				feedback.Alert(c, feedback.ToneDanger, "连接失败", snapshot.Detail).Role(ui.RoleAlert)
			case Connecting:
				ui.Text(c, "正在与现有 client-daemon 握手…").TextColor(theme.TextMuted)
				feedback.IndeterminateBar(c, "正在连接 client-daemon")
			default:
				feedback.EmptyState(c, "message-circle", "尚未连接", "尚未连接到 client-daemon。", "")
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
		ui.Column(c).Width(300).FillHeight().Shrink(0).Padding(16).Gap(12).Background(theme.Surface).Children(func() {
			ui.Text(c, "CheapRouter").FontSize(22).Bold()
			ui.Row(c).Gap(8).Children(func() {
				ui.Text(c, "会话").FontSize(18).Bold().Grow(1)
				ui.Text(c, snapshot.Status.Label()).FontSize(12).TextColor(theme.TextMuted).Role(ui.RoleStatus)
			})
			ui.Textf(c, "client-daemon 版本：%s", snapshot.DaemonVersion).FontSize(11).TextColor(theme.TextMuted)

			switch {
			case snapshot.SessionsLoading:
				ui.Text(c, "正在加载会话…").TextColor(theme.TextMuted).Role(ui.RoleStatus)
				feedback.IndeterminateBar(c, "正在加载会话")
			case snapshot.SessionsError != "":
				feedback.Alert(c, feedback.ToneDanger, "会话加载失败："+snapshot.SessionsError, "").Role(ui.RoleAlert)
			case len(snapshot.Sessions) == 0:
				feedback.EmptyState(c, "messages-square", "还没有会话", "", "")
			default:
				m.sessionListView(c, snapshot)
			}
		})

		ui.Column(c).Grow(1).FillHeight().MinWidth(0).Children(func() {
			if snapshot.SelectedSessionID == "" {
				ui.Column(c).Fill().Center().Children(func() {
					feedback.EmptyState(c, "message-circle", "请选择一个会话", "", "")
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
	list := data.VirtualList(c, "会话列表", len(snapshot.Sessions), &m.selectedSessionIndex, func(index int) {
		session := snapshot.Sessions[index]
		ui.Column(c).Gap(4).Children(func() {
			ui.Text(c, sessionName(session)).Bold()
			ui.Text(c, sessionLastMessageTime(session)).FontSize(12)
		})
	}, data.VirtualListOptions{}).Grow(1)
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

// Snapshot applies completed work and returns a copy. Like View, it must be
// called from the UI thread; background workers only use enqueue.
func (m *Model) Snapshot() Snapshot {
	m.drainPending()
	return m.state.Snapshot()
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
	m.gen.Store(m.state.Close())
	m.closedFlag.Store(true)
	if m.cancel != nil {
		m.cancel()
	}
	m.cancelSessionAndSend()
	if err := m.connector.Close(); err != nil {
		return fmt.Errorf("could not close client-daemon connection: %w", err)
	}
	return nil
}
