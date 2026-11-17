package shell

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
)

type fakeConnection struct {
	version string
	cursors []daemon.ReplayCursor
	closed  bool
}

func (c *fakeConnection) DaemonVersion() string { return c.version }
func (c *fakeConnection) LoadTaskState(context.Context) (daemon.TaskState, error) {
	return daemon.TaskState{}, nil
}
func (c *fakeConnection) HydrateSession(context.Context, string) (*daemon.AgentSession, error) {
	return nil, nil
}
func (c *fakeConnection) AttachSession(context.Context, string) (*daemon.SessionRuntime, error) {
	return nil, nil
}
func (c *fakeConnection) Prompt(context.Context, string, string, string) error { return nil }
func (c *fakeConnection) Events() <-chan daemon.SequencedEvent                 { return nil }
func (c *fakeConnection) ReplayCursors() []daemon.ReplayCursor {
	return append([]daemon.ReplayCursor(nil), c.cursors...)
}
func (c *fakeConnection) Done() <-chan struct{} { return nil }
func (c *fakeConnection) Err() error            { return nil }
func (c *fakeConnection) Close() error {
	c.closed = true
	return nil
}

type fakeConnector struct {
	connection *fakeConnection
	err        error
	resume     [][]daemon.ReplayCursor
	closed     bool
}

func (c *fakeConnector) Connect(_ context.Context, resume []daemon.ReplayCursor) (daemon.Connection, error) {
	c.resume = append(c.resume, append([]daemon.ReplayCursor(nil), resume...))
	if c.err != nil {
		return nil, c.err
	}
	if c.connection == nil {
		c.connection = &fakeConnection{version: "test-daemon"}
	}
	return c.connection, nil
}

func (c *fakeConnector) Close() error {
	c.closed = true
	if c.connection != nil {
		return c.connection.Close()
	}
	return nil
}

func TestStatusLabels(t *testing.T) {
	for status, want := range map[Status]string{
		Disconnected: "未连接",
		Connecting:   "连接中",
		Connected:    "已连接",
		Failed:       "连接失败",
	} {
		if got := status.Label(); got != want {
			t.Errorf("status %d = %q, want %q", status, got, want)
		}
	}
}

func TestModelConnectionResults(t *testing.T) {
	connector := &fakeConnector{}
	model := NewModel(connector)
	ctx, generation, ok := model.beginConnect()
	if !ok {
		t.Fatal("could not begin connection")
	}
	model.connect(ctx, generation)
	if got := model.Snapshot(); got.Status != Connected || got.DaemonVersion != "test-daemon" {
		t.Fatalf("connected snapshot = %+v", got)
	}

	connector.err = errors.New("bad token")
	ctx, generation, ok = model.beginConnect()
	if !ok {
		t.Fatal("could not begin retry")
	}
	model.connect(ctx, generation)
	if got := model.Snapshot(); got.Status != Failed || got.Detail != "bad token" {
		t.Fatalf("failed snapshot = %+v", got)
	}
}

func TestModelReconnectsWithReplayCursors(t *testing.T) {
	connector := &fakeConnector{}
	model := NewModel(connector)
	connect := func() {
		t.Helper()
		ctx, generation, ok := model.beginConnect()
		if !ok {
			t.Fatal("could not begin connection")
		}
		model.connect(ctx, generation)
	}

	connect()
	if len(connector.resume) != 1 || connector.resume[0] != nil {
		t.Fatalf("initial resume cursors = %#v, want nil", connector.resume)
	}

	cursor := daemon.ReplayCursor{
		SessionID: "session-1",
		RuntimeID: "runtime-1",
		Epoch:     "epoch-1",
		Sequence:  42,
	}
	connector.connection.cursors = []daemon.ReplayCursor{cursor}
	connector.err = errors.New("offline")
	connect()
	if got := model.Snapshot(); got.Status != Failed {
		t.Fatalf("failed reconnect snapshot = %+v", got)
	}

	connector.err = nil
	connector.connection = &fakeConnection{version: "new-daemon"}
	connect()
	if len(connector.resume) != 3 {
		t.Fatalf("connection attempts = %d, want 3", len(connector.resume))
	}
	for _, attempt := range []int{1, 2} {
		got := connector.resume[attempt]
		if len(got) != 1 || got[0] != cursor {
			t.Errorf("reconnect attempt %d resume cursors = %#v, want %#v", attempt+1, got, []daemon.ReplayCursor{cursor})
		}
	}
	if got := model.Snapshot(); got.Status != Connected || got.DaemonVersion != "new-daemon" {
		t.Fatalf("reconnected snapshot = %+v", got)
	}
}

func TestNativeViewStates(t *testing.T) {
	tests := []struct {
		name     string
		snapshot Snapshot
		want     []string
	}{
		{name: "disconnected", snapshot: Snapshot{Status: Disconnected}, want: []string{"未连接", "重新连接"}},
		{name: "connecting", snapshot: Snapshot{Status: Connecting}, want: []string{"连接中", "正在与现有 client-daemon 握手…"}},
		{name: "connected", snapshot: Snapshot{Status: Connected, DaemonVersion: "1.2.3"}, want: []string{"已连接", "client-daemon 版本：1.2.3", "还没有会话", "请选择一个会话"}},
		{name: "failed", snapshot: Snapshot{Status: Failed, Detail: "bad token"}, want: []string{"连接失败", "bad token", "重新连接"}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			model := NewModel(&fakeConnector{})
			model.snapshot = test.snapshot
			tester := ui.NewTester(model.View, 720, 480)
			for _, text := range append([]string{"CheapRouter"}, test.want...) {
				if !tester.HasText(text) {
					t.Errorf("view does not contain %q; texts: %q", text, tester.Texts())
				}
			}
		})
	}
}

func TestRetryButtonReconnectsWithoutAWindow(t *testing.T) {
	connector := &fakeConnector{err: errors.New("offline")}
	model := NewModel(connector)
	ctx, generation, _ := model.beginConnect()
	model.connect(ctx, generation)
	tester := ui.NewTester(model.View, 720, 480)

	connector.err = nil
	if err := tester.Click("重新连接"); err != nil {
		t.Fatal(err)
	}
	deadline := time.Now().Add(2 * time.Second)
	for model.Snapshot().Status != Connected && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	tester.Frame()
	if got := model.Snapshot(); got.Status != Connected {
		t.Fatalf("status after retry = %+v", got)
	}
	if !tester.HasText("client-daemon 版本：test-daemon") {
		t.Fatalf("connected view texts = %q", tester.Texts())
	}
	if err := model.Close(); err != nil {
		t.Fatal(err)
	}
	if !connector.closed {
		t.Fatal("model Close did not close its connector")
	}
}
