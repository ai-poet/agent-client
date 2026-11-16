// Package shell contains the native MyGo desktop shell's testable state and
// views. The workbench itself is intentionally outside this first phase.
package shell

import (
	"context"
	"fmt"
	"sync"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
)

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

// Snapshot is an immutable copy of what a frame should render.
type Snapshot struct {
	Status        Status
	DaemonVersion string
	Detail        string
}

// Connector is implemented by daemon.Supervisor and by headless test fakes.
type Connector interface {
	Connect(context.Context) (daemon.Connection, error)
	Close() error
}

// Model owns connection state. Network and process work always runs off the
// UI thread; frames only read the in-memory snapshot.
type Model struct {
	connector Connector

	mu         sync.Mutex
	services   ui.Services
	snapshot   Snapshot
	connection daemon.Connection
	generation uint64
	cancel     context.CancelFunc
	closed     bool
}

func NewModel(connector Connector) *Model {
	return &Model{
		connector: connector,
		snapshot:  Snapshot{Status: Disconnected},
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
	if m.closed {
		m.mu.Unlock()
		return nil, 0, false
	}
	if m.cancel != nil {
		m.cancel()
	}
	m.generation++
	generation := m.generation
	ctx, cancel := context.WithCancel(context.Background())
	m.cancel = cancel
	m.snapshot = Snapshot{Status: Connecting}
	m.mu.Unlock()
	m.invalidate()
	return ctx, generation, true
}

func (m *Model) connect(ctx context.Context, generation uint64) {
	connection, err := m.connector.Connect(ctx)

	m.mu.Lock()
	if m.closed || generation != m.generation {
		m.mu.Unlock()
		if connection != nil {
			_ = connection.Close()
		}
		return
	}
	m.cancel = nil
	if err != nil {
		m.snapshot = Snapshot{Status: Failed, Detail: err.Error()}
	} else if connection == nil {
		m.snapshot = Snapshot{Status: Failed, Detail: "client-daemon 返回了空连接"}
	} else {
		m.connection = connection
		m.snapshot = Snapshot{
			Status:        Connected,
			DaemonVersion: connection.DaemonVersion(),
		}
	}
	m.mu.Unlock()
	m.invalidate()
}

// View draws native GPU UI through MyGo; there is no HTML or webview.
func (m *Model) View(c *ui.Context) {
	m.mu.Lock()
	m.services = c.Services()
	snapshot := m.snapshot
	m.mu.Unlock()
	theme := c.Theme()

	ui.Column(c).Fill().Center().Padding(32).Children(func() {
		ui.Column(c).Width(440).Padding(28).Gap(16).Background(theme.Surface).Radius(12).Children(func() {
			ui.Text(c, "CheapRouter").FontSize(28).Bold()
			ui.Text(c, "MyGo 原生桌面壳 · 第一阶段").TextColor(theme.TextMuted)
			ui.Text(c, snapshot.Status.Label()).FontSize(20).Bold().Role(ui.RoleStatus)

			switch snapshot.Status {
			case Connected:
				ui.Textf(c, "client-daemon 版本：%s", snapshot.DaemonVersion)
				ui.Text(c, "握手已完成。工作台尚未迁移到此客户端。").TextColor(theme.TextMuted)
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

// Snapshot returns a race-free copy for logic tests and embedding code.
func (m *Model) Snapshot() Snapshot {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.snapshot
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
	if m.cancel != nil {
		m.cancel()
	}
	m.mu.Unlock()
	if err := m.connector.Close(); err != nil {
		return fmt.Errorf("could not close client-daemon connection: %w", err)
	}
	return nil
}
