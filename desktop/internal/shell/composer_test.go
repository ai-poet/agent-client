package shell

import (
	"errors"
	"reflect"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/egoist/mygo/ui"
)

func TestComposerSendClearsInputAndDisablesWhilePromptRuns(t *testing.T) {
	model, connection, tester := readyComposer(t)
	connection.promptStarted = make(chan promptCall, 1)
	connection.promptRelease = make(chan struct{})

	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	tester.Type("请继续")
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	call := receivePrompt(t, connection.promptStarted)
	want := promptCall{sessionID: "session-a", runtimeID: "runtime-session-a", text: "请继续"}
	if call != want {
		t.Fatalf("Prompt call = %+v, want %+v", call, want)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return snapshot.Sending })
	tester.Frame()
	if got := model.Snapshot().Draft; got != "" {
		t.Fatalf("draft while sending = %q, want empty", got)
	}
	if !tester.HasText("发送中…") {
		t.Fatalf("sending view texts = %q", tester.Texts())
	}
	tester.Type("发送中不应进入输入框")
	if got := model.Snapshot().Draft; got != "" {
		t.Fatalf("disabled input changed to %q", got)
	}

	close(connection.promptRelease)
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return !snapshot.Sending })
	tester.Frame()
	if got := model.Snapshot(); got.Draft != "" || got.SendError != "" {
		t.Fatalf("successful send state = %+v", got)
	}
	if got := connection.prompts(); len(got) != 1 {
		t.Fatalf("Prompt calls = %d, want 1", len(got))
	}
}

func TestComposerFailureRestoresInput(t *testing.T) {
	model, connection, tester := readyComposer(t)
	connection.promptErr = errors.New("offline")

	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	tester.Type("请保留这段输入")
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return snapshot.SendError != "" })
	tester.Frame()
	if got := model.Snapshot().Draft; got != "请保留这段输入" {
		t.Fatalf("draft after failed send = %q, want original", got)
	}
	if !tester.HasText("发送失败，已保留输入") {
		t.Fatalf("failed send view texts = %q", tester.Texts())
	}
	if got := connection.prompts(); len(got) != 1 {
		t.Fatalf("Prompt calls = %d, want 1", len(got))
	}
}

func TestComposerSwitchingSessionRestoresInFlightDraft(t *testing.T) {
	connection := newFakeWorkbenchConnection("test-daemon",
		daemon.AgentSession{ID: "session-a", Title: "会话甲"},
		daemon.AgentSession{ID: "session-b", Title: "会话乙"},
	)
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 0)
	connection.sessions["session-b"] = hydratedSession("session-b", "会话乙", 0)
	connection.promptStarted = make(chan promptCall, 1)
	connection.promptRelease = make(chan struct{})
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
		return snapshot.SelectedSessionID == "session-a" && !snapshot.SessionLoading
	})
	tester.Frame()
	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	const original = "发送途中切会话也要保留"
	tester.Type(original)
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	receivePrompt(t, connection.promptStarted)
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return snapshot.Sending })

	if err := tester.Click("会话乙"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-b" && !snapshot.SessionLoading
	})
	tester.Frame()
	if got := model.Snapshot().Draft; got != "" {
		t.Fatalf("session-b draft = %q, want empty", got)
	}

	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-a" && !snapshot.SessionLoading
	})
	tester.Frame()
	if got := model.Snapshot().Draft; got != original {
		t.Fatalf("restored in-flight draft = %q, want %q", got, original)
	}
	if got := connection.prompts(); len(got) != 1 {
		t.Fatalf("Prompt calls = %d, want 1", len(got))
	}
}

func TestComposerEnterSendsAndShiftEnterBreaksLine(t *testing.T) {
	model, connection, tester := readyComposer(t)
	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	tester.Type("第一行")
	tester.Key(ui.Shift, ui.KeyEnter)
	if got := model.Snapshot().Draft; got != "第一行\n" {
		t.Fatalf("Shift+Enter draft = %q", got)
	}
	tester.Type("第二行")
	tester.Key(0, ui.KeyEnter)
	waitFor(t, func() bool { return len(connection.prompts()) == 1 })
	want := []promptCall{{sessionID: "session-a", runtimeID: "runtime-session-a", text: "第一行\n第二行"}}
	if got := connection.prompts(); !reflect.DeepEqual(got, want) {
		t.Fatalf("Prompt calls = %+v, want %+v", got, want)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return !snapshot.Sending })
	tester.Frame()
	if got := model.Snapshot().Draft; got != "" {
		t.Fatalf("draft after Enter send = %q, want empty", got)
	}
}

func TestComposerAttachesBeforePromptWhenRuntimeIsMissing(t *testing.T) {
	model, connection, tester := readyComposer(t)
	model.mu.Lock()
	model.attachedRuntimeID = ""
	model.mu.Unlock()

	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	tester.Type("重新附着后发送")
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, func() bool { return len(connection.prompts()) == 1 })
	attach, _ := connection.calls()
	if !reflect.DeepEqual(attach, []string{"session-a", "session-a"}) {
		t.Fatalf("AttachSession calls = %q, want selection and send", attach)
	}
	if got := connection.prompts()[0]; got.runtimeID != "runtime-session-a" {
		t.Fatalf("Prompt runtime = %q", got.runtimeID)
	}
}

func readyComposer(t *testing.T) (*Model, *fakeWorkbenchConnection, *ui.Tester) {
	t.Helper()
	connection := newFakeWorkbenchConnection("test-daemon", daemon.AgentSession{ID: "session-a", Title: "会话甲"})
	connection.sessions["session-a"] = hydratedSession("session-a", "会话甲", 0)
	connector := &fakeWorkbenchConnector{connections: []*fakeWorkbenchConnection{connection}}
	model := NewModel(connector)
	t.Cleanup(func() { _ = model.Close() })
	model.Connect()
	waitForSnapshot(t, model, func(snapshot Snapshot) bool { return len(snapshot.Sessions) == 1 })
	tester := ui.NewTester(model.View, 900, 600)
	if err := tester.Click("会话甲"); err != nil {
		t.Fatal(err)
	}
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return snapshot.SelectedSessionID == "session-a" && !snapshot.SessionLoading && snapshot.SessionError == ""
	})
	tester.Frame()
	return model, connection, tester
}

func receivePrompt(t *testing.T, calls <-chan promptCall) promptCall {
	t.Helper()
	select {
	case call := <-calls:
		return call
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for Prompt")
		return promptCall{}
	}
}
