package shell

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
)

type fakeWorkbenchConnection struct {
	mu              sync.Mutex
	version         string
	state           daemon.TaskState
	sessions        map[string]*daemon.AgentSession
	cursors         []daemon.ReplayCursor
	loadErr         error
	attachErr       error
	hydrateErr      error
	attachCalls     []string
	hydrateCalls    []string
	hydrateStarted  chan string
	hydrateRelease  map[string]chan struct{}
	hydrateReturned chan string
	promptCalls     []promptCall
	promptErr       error
	promptStarted   chan promptCall
	promptRelease   chan struct{}
	events          chan daemon.SequencedEvent
	done            chan struct{}
	closeOnce       sync.Once
}

func newFakeWorkbenchConnection(version string, sessions ...daemon.AgentSession) *fakeWorkbenchConnection {
	return &fakeWorkbenchConnection{
		version:  version,
		state:    daemon.TaskState{Sessions: sessions},
		sessions: make(map[string]*daemon.AgentSession),
		events:   make(chan daemon.SequencedEvent, 16),
		done:     make(chan struct{}),
	}
}

func (c *fakeWorkbenchConnection) DaemonVersion() string { return c.version }

func (c *fakeWorkbenchConnection) LoadTaskState(context.Context) (daemon.TaskState, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	state := c.state
	state.Sessions = append([]daemon.AgentSession(nil), state.Sessions...)
	return state, c.loadErr
}

func (c *fakeWorkbenchConnection) HydrateSession(_ context.Context, sessionID string) (*daemon.AgentSession, error) {
	c.mu.Lock()
	c.hydrateCalls = append(c.hydrateCalls, sessionID)
	session := c.sessions[sessionID]
	err := c.hydrateErr
	started := c.hydrateStarted
	release := c.hydrateRelease[sessionID]
	returned := c.hydrateReturned
	c.mu.Unlock()

	if started != nil {
		started <- sessionID
	}
	if release != nil {
		<-release
	}
	if returned != nil {
		returned <- sessionID
	}
	return session, err
}

func (c *fakeWorkbenchConnection) AttachSession(_ context.Context, sessionID string) (*daemon.SessionRuntime, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.attachCalls = append(c.attachCalls, sessionID)
	if c.attachErr != nil {
		return nil, c.attachErr
	}
	return &daemon.SessionRuntime{RuntimeID: "runtime-" + sessionID, SupportsSteer: true}, nil
}

type promptCall struct {
	sessionID string
	runtimeID string
	text      string
}

func (c *fakeWorkbenchConnection) Prompt(ctx context.Context, sessionID, runtimeID, text string) error {
	call := promptCall{sessionID: sessionID, runtimeID: runtimeID, text: text}
	c.mu.Lock()
	c.promptCalls = append(c.promptCalls, call)
	started, release, err := c.promptStarted, c.promptRelease, c.promptErr
	c.mu.Unlock()
	if started != nil {
		started <- call
	}
	if release != nil {
		select {
		case <-release:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return err
}

func (c *fakeWorkbenchConnection) prompts() []promptCall {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]promptCall(nil), c.promptCalls...)
}

func (c *fakeWorkbenchConnection) Events() <-chan daemon.SequencedEvent { return c.events }

func (c *fakeWorkbenchConnection) ReplayCursors() []daemon.ReplayCursor {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]daemon.ReplayCursor(nil), c.cursors...)
}

func (c *fakeWorkbenchConnection) setReplayCursors(cursors ...daemon.ReplayCursor) {
	c.mu.Lock()
	c.cursors = append([]daemon.ReplayCursor(nil), cursors...)
	c.mu.Unlock()
}

func (c *fakeWorkbenchConnection) calls() ([]string, []string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]string(nil), c.attachCalls...), append([]string(nil), c.hydrateCalls...)
}

func (c *fakeWorkbenchConnection) Done() <-chan struct{} { return c.done }
func (c *fakeWorkbenchConnection) Err() error            { return nil }
func (c *fakeWorkbenchConnection) Close() error {
	c.closeOnce.Do(func() { close(c.done) })
	return nil
}

type fakeWorkbenchConnector struct {
	mu          sync.Mutex
	connections []*fakeWorkbenchConnection
	resume      [][]daemon.ReplayCursor
	closed      bool
}

func (c *fakeWorkbenchConnector) Connect(_ context.Context, resume []daemon.ReplayCursor) (daemon.Connection, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.resume = append(c.resume, append([]daemon.ReplayCursor(nil), resume...))
	if len(c.connections) == 0 {
		return nil, errors.New("没有测试连接")
	}
	index := len(c.resume) - 1
	if index >= len(c.connections) {
		index = len(c.connections) - 1
	}
	return c.connections[index], nil
}

func (c *fakeWorkbenchConnector) Close() error {
	c.mu.Lock()
	c.closed = true
	connections := append([]*fakeWorkbenchConnection(nil), c.connections...)
	c.mu.Unlock()
	for _, connection := range connections {
		_ = connection.Close()
	}
	return nil
}

func (c *fakeWorkbenchConnector) resumeAttempts() [][]daemon.ReplayCursor {
	c.mu.Lock()
	defer c.mu.Unlock()
	attempts := make([][]daemon.ReplayCursor, len(c.resume))
	for index := range c.resume {
		attempts[index] = append([]daemon.ReplayCursor(nil), c.resume[index]...)
	}
	return attempts
}

func TestWorkbenchSessionListRendering(t *testing.T) {
	lastReply := uint64(1_700_000_000)
	connection := newFakeWorkbenchConnection("test-daemon",
		daemon.AgentSession{ID: "session-a", Title: "会话甲", LastReplyAt: &lastReply},
		daemon.AgentSession{ID: "session-b", AutoTitle: stringPointer("自动标题乙")},
	)
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 2 })

	tester := ui.NewTester(model.View, 900, 600)
	for _, text := range []string{"会话甲", sessionLastMessageTime(connection.state.Sessions[0]), "自动标题乙", "最后消息：暂无", "请选择一个会话"} {
		if !tester.HasText(text) {
			t.Errorf("session list does not contain %q; texts: %q", text, tester.Texts())
		}
	}
}

func TestWorkbenchCatalogLoadingEmptyAndErrorStates(t *testing.T) {
	tests := []struct {
		name     string
		snapshot Snapshot
		want     string
	}{
		{name: "loading", snapshot: Snapshot{Status: Connected, SessionsLoading: true}, want: "正在加载会话…"},
		{name: "empty", snapshot: Snapshot{Status: Connected}, want: "还没有会话"},
		{name: "error", snapshot: Snapshot{Status: Connected, SessionsError: "网络不可用"}, want: "会话加载失败：网络不可用"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			model := NewModel(&fakeWorkbenchConnector{})
			model.snapshot = test.snapshot
			tester := ui.NewTester(model.View, 900, 600)
			if !tester.HasText(test.want) {
				t.Fatalf("view does not contain %q; texts: %q", test.want, tester.Texts())
			}
		})
	}
}

func TestWorkbenchSelectionHydratesAndSwitches(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon",
		daemon.AgentSession{ID: "session-a", Title: "会话甲"},
		daemon.AgentSession{ID: "session-b", Title: "会话乙"},
	)
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 2)
	connection.sessions["session-b"] = hydratedSession("session-b", "会话乙", 1)
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 2 })
	tester := ui.NewTester(model.View, 900, 600)

	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-a" && !snapshot.SessionLoading && snapshot.MessageCount == 2
	})
	tester.Frame()
	if !tester.HasText("会话甲") || !tester.HasText("已有消息") {
		t.Fatalf("first hydrate view texts = %q", tester.Texts())
	}

	if err := tester.Click("会话乙"); err != nil {
		t.Fatal(err)
	}
	snapshot := waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-b" && !snapshot.SessionLoading && snapshot.MessageCount == 1
	})
	if len(snapshot.Transcript) != 1 {
		t.Fatalf("second transcript length = %d, want 1", len(snapshot.Transcript))
	}
	tester.Frame()
	if !tester.HasText("会话乙") || !tester.HasText("已有消息") {
		t.Fatalf("switched hydrate view texts = %q", tester.Texts())
	}
	attach, hydrate := connection.calls()
	if !reflect.DeepEqual(attach, []string{"session-a", "session-b"}) ||
		!reflect.DeepEqual(hydrate, []string{"session-a", "session-b"}) {
		t.Fatalf("attach/hydrate calls = %q/%q", attach, hydrate)
	}
}

func TestWorkbenchBuffersTenEventsUntilHydrateWithoutLossOrDuplicates(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon", daemon.AgentSession{ID: "session-a", Title: "会话甲"})
	session := hydratedSession("session-a", "会话甲", 1)
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: 1,
	}
	connection.sessions["session-a"] = session
	connection.hydrateStarted = make(chan string, 1)
	connection.hydrateRelease = map[string]chan struct{}{"session-a": make(chan struct{})}
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	if got := receiveString(t, connection.hydrateStarted); got != "session-a" {
		t.Fatalf("hydrate session = %q", got)
	}

	connection.events <- transcriptTextEvent("session-a", "runtime-session-a", 1, "不应重复")
	deltas := []string{"零", "一", "二", "三", "四", "五", "六", "七", "八", "九"}
	for index, delta := range deltas {
		connection.events <- transcriptTextEvent("session-a", "runtime-session-a", uint64(index+2), delta)
	}
	waitFor(t, func() bool {
		model.drainPending()
		return len(model.pendingEvents) == len(deltas)+1
	})
	if got := model.Snapshot(); len(got.Transcript) != 0 {
		t.Fatalf("events applied before hydrate: %+v", got.Transcript)
	}
	close(connection.hydrateRelease["session-a"])

	snapshot := waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return !snapshot.SessionLoading && len(snapshot.Transcript) == 2 &&
			snapshot.Transcript[1].Text == strings.Join(deltas, "")
	})
	if strings.Contains(snapshot.Transcript[1].Text, "不应重复") {
		t.Fatalf("snapshot event was replayed twice: %+v", snapshot.Transcript)
	}
	pending := len(model.pendingEvents)
	if pending != 0 {
		t.Fatalf("pending events after replay = %d, want 0", pending)
	}
}

func TestWorkbenchHydrateWatermarkReplaysOnlyNewerBufferedEvents(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon", daemon.AgentSession{ID: "session-a", Title: "会话甲"})
	session := hydratedSession("session-a", "会话甲", 0)
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: 5,
	}
	connection.sessions["session-a"] = session
	connection.hydrateStarted = make(chan string, 1)
	connection.hydrateRelease = map[string]chan struct{}{"session-a": make(chan struct{})}
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	receiveString(t, connection.hydrateStarted)

	for _, event := range []struct {
		sequence uint64
		text     string
	}{{3, "三"}, {5, "五"}, {7, "七"}} {
		connection.events <- transcriptTextEvent("session-a", "runtime-session-a", event.sequence, event.text)
	}
	waitFor(t, func() bool {
		model.drainPending()
		return len(model.pendingEvents) == 3
	})
	close(connection.hydrateRelease["session-a"])

	snapshot := waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return !snapshot.SessionLoading && len(snapshot.Transcript) == 1
	})
	if got := snapshot.Transcript[0].Text; got != "七" {
		t.Fatalf("transcript after watermark replay = %q, want only sequence 7", got)
	}
	if model.eventCursor == nil || model.eventCursor.Sequence != 7 {
		t.Fatalf("event cursor after replay = %+v, want sequence 7", model.eventCursor)
	}
}

func TestWorkbenchEventBufferOverflowDebouncesRehydrate(t *testing.T) {
	var logs bytes.Buffer
	previousLogOutput := log.Writer()
	log.SetOutput(&logs)
	t.Cleanup(func() { log.SetOutput(previousLogOutput) })

	connection := newFakeWorkbenchConnection("test-daemon", daemon.AgentSession{ID: "session-a", Title: "会话甲"})
	session := hydratedSession("session-a", "会话甲", 0)
	connection.sessions["session-a"] = session
	connection.hydrateStarted = make(chan string, 4)
	connection.hydrateRelease = map[string]chan struct{}{"session-a": make(chan struct{})}
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	receiveString(t, connection.hydrateStarted)

	for sequence := uint64(1); sequence <= pendingEventLimit+1; sequence++ {
		connection.events <- transcriptTextEvent("session-a", "runtime-session-a", sequence, "首批增量")
	}
	waitFor(t, func() bool {
		tester.Frame()
		return model.rehydrating
	})
	if got := receiveString(t, connection.hydrateStarted); got != "session-a" {
		t.Fatalf("rehydrate session = %q", got)
	}

	// A second full buffer must not cancel and restart the rehydrate already
	// in flight. It is dropped with its own warning and recovered by snapshot.
	for sequence := uint64(1); sequence <= pendingEventLimit+1; sequence++ {
		connection.events <- transcriptTextEvent("session-a", "runtime-session-a", sequence, "第二批增量")
	}
	waitFor(t, func() bool {
		tester.Frame()
		return strings.Contains(logs.String(), "overflow during rehydrate") ||
			strings.Count(logs.String(), "forcing session rehydrate") > 1
	})
	_, hydrateCalls := connection.calls()
	if len(hydrateCalls) != 2 {
		t.Fatalf("hydrate calls after two overflows = %q, want initial plus one rehydrate", hydrateCalls)
	}
	if got := strings.Count(logs.String(), "forcing session rehydrate"); got != 1 {
		t.Fatalf("forced rehydrate warnings = %d, want 1; logs:\n%s", got, logs.String())
	}
	if len(model.pendingEvents) > pendingEventLimit {
		t.Fatalf("pending event buffer length = %d, limit %d", len(model.pendingEvents), pendingEventLimit)
	}

	// Both blocked calls observe this same session pointer. The first result is
	// stale; the single rehydrate installs the authoritative final transcript.
	session.Messages = []daemon.Message{{ID: "final", Role: "assistant", Content: "最终快照"}}
	session.RuntimeEventCursor = &daemon.RuntimeEventCursor{
		RuntimeID: "runtime-session-a", Epoch: "epoch-test", Sequence: pendingEventLimit + 1,
	}
	close(connection.hydrateRelease["session-a"])
	snapshot := waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return !snapshot.SessionLoading && len(snapshot.Transcript) == 1 &&
			snapshot.Transcript[0].Text == "最终快照"
	})
	if strings.Contains(snapshot.Transcript[0].Text, "增量") {
		t.Fatalf("dropped overflow events entered final transcript: %+v", snapshot.Transcript)
	}
	if model.rehydrating {
		t.Fatal("successful rehydrate did not reset its in-progress flag")
	}
}

func TestWorkbenchFailedRehydrateResetsInProgressFlag(t *testing.T) {
	model := NewModel(&fakeConnector{})
	model.generation = 1
	model.sessionGeneration = 1
	model.rehydrating = true
	model.snapshot.SelectedSessionID = "session-a"
	model.snapshot.Sessions = []SessionSummary{{ID: "session-a"}}
	model.snapshot.SessionLoading = true

	model.finishSessionError(context.Background(), 1, 1, "session-a", errors.New("offline"))
	snapshot := model.Snapshot()
	if model.rehydrating || snapshot.SessionLoading || snapshot.SessionError != "offline" {
		t.Fatalf("failed rehydrate state = %+v, in progress = %v", snapshot, model.rehydrating)
	}
}

func TestWorkbenchStaleSessionLoadCannotOverrideSelection(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon",
		daemon.AgentSession{ID: "session-a", Title: "会话甲"},
		daemon.AgentSession{ID: "session-b", Title: "会话乙"},
	)
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 2)
	connection.sessions["session-b"] = hydratedSession("session-b", "会话乙", 1)
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 2 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return snapshot.MessageCount == 2 })
	if err := tester.Click("会话乙"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-b" && !snapshot.SessionLoading && snapshot.MessageCount == 1
	})

	generation := model.generation
	model.startSessionLoad(generation, connection, "session-a")
	if got := model.Snapshot(); got.SelectedSessionID != "session-b" || got.SessionLoading || got.MessageCount != 1 {
		t.Fatalf("stale session load changed selection: %+v", got)
	}
}

func TestWorkbenchReconnectSendsResumeFromAndRehydrates(t *testing.T) {
	summary := daemon.AgentSession{ID: "session-a", Title: "会话甲"}
	oldConnection := newFakeWorkbenchConnection("old-daemon", summary)
	oldConnection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 1)
	newConnection := newFakeWorkbenchConnection("new-daemon", summary)
	newConnection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 1)
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{oldConnection, newConnection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return snapshot.MessageCount == 1 })

	cursor := daemon.ReplayCursor{SessionID: "session-a", RuntimeID: "runtime-session-a", Epoch: "epoch-a", Sequence: 42}
	oldConnection.setReplayCursors(cursor)
	if err := oldConnection.Close(); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool {
		model.drainPending()
		return len(connector.resumeAttempts()) == 2
	})
	attempts := connector.resumeAttempts()
	if !reflect.DeepEqual(attempts[1], []daemon.ReplayCursor{cursor}) {
		t.Fatalf("reconnect resumeFrom = %#v, want %#v", attempts[1], cursor)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.Status == Connected && snapshot.DaemonVersion == "new-daemon" &&
			snapshot.SelectedSessionID == "session-a" && snapshot.MessageCount == 1
	})
	tester.Frame()
	if !tester.HasText("client-daemon 版本：new-daemon") || !tester.HasText("已有消息") {
		t.Fatalf("reconnected view texts = %q", tester.Texts())
	}
}

func TestWorkbenchDeletedSessionLateHydrateDoesNotWriteTranscript(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon", daemon.AgentSession{ID: "session-a", Title: "会话甲"})
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 1)
	connection.hydrateStarted = make(chan string, 1)
	connection.hydrateReturned = make(chan string, 1)
	connection.hydrateRelease = map[string]chan struct{}{"session-a": make(chan struct{})}
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	receiveString(t, connection.hydrateStarted)

	// Isolate catalog membership from the other two guards: the connection and
	// session generations and the selected ID all remain unchanged here.
	model.snapshot.Sessions = nil
	close(connection.hydrateRelease["session-a"])
	if got := receiveString(t, connection.hydrateReturned); got != "session-a" {
		t.Fatalf("late hydrate session = %q", got)
	}
	snapshot := model.Snapshot()
	if snapshot.SelectedSessionID != "session-a" || len(snapshot.Transcript) != 0 || snapshot.MessageCount != 0 {
		t.Fatalf("deleted session hydrate wrote transcript: %+v", snapshot)
	}
	if model.hydratedSessionID != "" || model.hydratedGeneration != 0 {
		t.Fatalf("deleted session was marked hydrated: generation=%d ID=%q", model.hydratedGeneration, model.hydratedSessionID)
	}
}

func TestWorkbenchDropsLateHydrateAndEvents(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon",
		daemon.AgentSession{ID: "session-a", Title: "会话甲"},
		daemon.AgentSession{ID: "session-b", Title: "会话乙"},
	)
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 3)
	connection.sessions["session-b"] = hydratedSession("session-b", "会话乙", 1)
	connection.hydrateStarted = make(chan string, 2)
	connection.hydrateReturned = make(chan string, 2)
	connection.hydrateRelease = map[string]chan struct{}{"session-a": make(chan struct{})}
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 2 })
	tester := ui.NewTester(model.View, 900, 600)

	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	if got := receiveString(t, connection.hydrateStarted); got != "session-a" {
		t.Fatalf("first hydrate = %q", got)
	}
	if err := tester.Click("会话乙"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-b" && snapshot.MessageCount == 1 && !snapshot.SessionLoading
	})
	close(connection.hydrateRelease["session-a"])
	if got := receiveString(t, connection.hydrateReturned); got != "session-b" {
		t.Fatalf("completed hydrate = %q", got)
	}
	if got := receiveString(t, connection.hydrateReturned); got != "session-a" {
		t.Fatalf("late hydrate = %q", got)
	}
	snapshot := model.Snapshot()
	if snapshot.SelectedSessionID != "session-b" || snapshot.MessageCount != 1 || len(snapshot.Transcript) != 1 {
		t.Fatalf("late hydrate overwrote selection: %+v", snapshot)
	}

	late := transcriptTextEvent("session-a", "runtime-session-a", 8, "迟到内容")
	current := transcriptTextEvent("session-b", "runtime-session-b", 9, "当前内容")
	connection.events <- late
	connection.events <- current
	snapshot = waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return len(snapshot.Transcript) == 2 && strings.Contains(snapshot.Transcript[1].Text, "当前内容")
	})
	if strings.Contains(snapshot.Transcript[0].Text, "迟到内容") || len(snapshot.Transcript) != 2 {
		t.Fatalf("late event entered current transcript: %+v", snapshot.Transcript)
	}
	model.applyEvent(model.generation-1, transcriptTextEvent("session-b", "runtime-session-b", 10, "旧连接内容"))
	if got := model.Snapshot().Transcript; len(got) != 2 || strings.Contains(got[1].Text, "旧连接内容") {
		t.Fatalf("old-generation event entered transcript: %+v", got)
	}
	tester.Frame()
	tester.Advance(2 * time.Second)
	if !tester.HasText("已有消息") || !tester.HasText("当前内容") {
		t.Fatalf("view after late results = %q", tester.Texts())
	}
}

func hydratedSession(id, title string, messageCount int) *daemon.AgentSession {
	session := &daemon.AgentSession{ID: id, Title: title}
	for index := 0; index < messageCount; index++ {
		session.Messages = append(session.Messages, daemon.Message{
			ID: id + "-message-" + string(rune('a'+index)), Role: "assistant", Content: "已有消息",
		})
	}
	return session
}

func transcriptTextEvent(sessionID, runtimeID string, sequence uint64, text string) daemon.SequencedEvent {
	return daemon.SequencedEvent{
		SessionID: sessionID,
		RuntimeID: runtimeID,
		Epoch:     "epoch-test",
		Sequence:  sequence,
		Event: daemon.WireDriverEvent{
			Kind:    "textDelta",
			Payload: json.RawMessage(strconvQuote(text)),
		},
	}
}

func strconvQuote(value string) string {
	encoded, _ := json.Marshal(value)
	return string(encoded)
}

func receiveString(t *testing.T, values <-chan string) string {
	t.Helper()
	select {
	case value := <-values:
		return value
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for fake connection call")
		return ""
	}
}

func waitForSnapshot(t *testing.T, model *Model, predicate func(Snapshot) bool) Snapshot {
	t.Helper()
	var snapshot Snapshot
	waitFor(t, func() bool {
		snapshot = model.Snapshot()
		return predicate(snapshot)
	})
	return snapshot
}

func waitFor(t *testing.T, predicate func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		if predicate() {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("timed out waiting for model state")
}
