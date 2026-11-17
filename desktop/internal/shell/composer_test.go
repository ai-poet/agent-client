package shell

import (
	"context"
	"errors"
	"fmt"
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
	if !tester.HasText("发送失败，已保留输入：offline") {
		t.Fatalf("failed send view texts = %q", tester.Texts())
	}
	if got := connection.prompts(); len(got) != 1 {
		t.Fatalf("Prompt calls = %d, want 1", len(got))
	}
}

func TestComposerSendResultDrainsDuringConcurrentFrames(t *testing.T) {
	model, connection, tester := readyComposer(t)
	connection.promptStarted = make(chan promptCall, 1)
	connection.promptRelease = make(chan struct{})
	connection.promptErr = errors.New("offline")

	if err := tester.Click("消息输入"); err != nil {
		t.Fatal(err)
	}
	const original = "并发渲染期间也要保留"
	tester.Type(original)
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	receivePrompt(t, connection.promptStarted)
	tester.Frame()
	if !tester.HasText("发送中…") {
		t.Fatalf("view before Prompt completed = %q", tester.Texts())
	}

	close(connection.promptRelease)
	deadline := time.Now().Add(3 * time.Second)
	const failure = "发送失败，已保留输入：offline"
	for !tester.HasText(failure) && time.Now().Before(deadline) {
		tester.Frame()
		time.Sleep(time.Millisecond)
	}
	if !tester.HasText(failure) {
		t.Fatalf("send result was not rendered; view texts = %q", tester.Texts())
	}
	snapshot := model.Snapshot()
	if snapshot.Sending || snapshot.Draft != original || snapshot.SendError == "" {
		t.Fatalf("drained send state = %+v", snapshot)
	}
	if got := connection.prompts(); len(got) != 1 {
		t.Fatalf("Prompt calls = %d, want 1", len(got))
	}
}

func TestSendErrorCategory(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want string
	}{
		{name: "rejected", err: &daemon.RPCError{Message: "prompt rejected"}, want: "rejected"},
		{name: "timeout", err: fmt.Errorf("request failed: %w", context.DeadlineExceeded), want: "timeout"},
		{name: "other", err: errors.New("offline"), want: "other"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := sendErrorCategory(test.err); got != test.want {
				t.Fatalf("send error category = %q, want %q", got, test.want)
			}
		})
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

	// Let a subsequent send succeed, then switch away and back. Neither the
	// visible input nor the saved per-session draft may resurrect the old text.
	close(connection.promptRelease)
	if err := tester.Click("发送"); err != nil {
		t.Fatal(err)
	}
	receivePrompt(t, connection.promptStarted)
	waitForSnapshot(t, model, func(snapshot Snapshot) bool {
		return !snapshot.Sending && snapshot.SendError == ""
	})
	for _, title := range []string{"会话乙", "会话甲"} {
		if err := tester.Click(title); err != nil {
			t.Fatal(err)
		}
		waitForSnapshot(t, model, func(snapshot Snapshot) bool {
			wantID := "session-a"
			if title == "会话乙" {
				wantID = "session-b"
			}
			return snapshot.SelectedSessionID == wantID && !snapshot.SessionLoading
		})
		tester.Frame()
	}
	if got := model.Snapshot().Draft; got != "" {
		t.Fatalf("successful send restored stale draft %q", got)
	}
	if _, ok := model.drafts["session-a"]; ok {
		t.Fatalf("successful send kept session-a draft %q", model.drafts["session-a"])
	}
}

func TestFinishSendSuccessDeletesSavedSessionDraft(t *testing.T) {
	model := NewModel(&fakeConnector{})
	model.generation = 1
	model.sessionGeneration = 1
	model.snapshot.SelectedSessionID = "session-a"
	model.snapshot.Sessions = []SessionSummary{{ID: "session-a"}}
	model.snapshot.Sending = true
	model.sendingText = "旧稿"
	model.drafts["session-a"] = "旧稿"

	model.finishSend(context.Background(), 1, 1, "session-a", "旧稿", nil)
	snapshot := model.Snapshot()
	if snapshot.Sending || snapshot.Draft != "" {
		t.Fatalf("successful send state = %+v", snapshot)
	}
	if draft, ok := model.drafts["session-a"]; ok {
		t.Fatalf("successful send kept saved draft %q", draft)
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
	model.attachedRuntimeID = ""

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
