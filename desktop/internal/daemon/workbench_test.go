package daemon

import (
	"context"
	"encoding/json"
	"reflect"
	"testing"
	"time"
)

func TestLoadTaskStateUsesProtocolWireShape(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	type answer struct {
		state TaskState
		err   error
	}
	answers := make(chan answer, 1)
	go func() {
		state, err := client.LoadTaskState(context.Background())
		answers <- answer{state: state, err: err}
	}()
	request := awaitFakeRequest(t, peer)
	if request.SessionID != NilUUID || request.RuntimeID != NilUUID {
		t.Fatalf("loadTaskState IDs = %q/%q", request.SessionID, request.RuntimeID)
	}
	command := decodeFakeCommand(t, request)
	if !reflect.DeepEqual(command, map[string]any{"type": "loadTaskState"}) {
		t.Fatalf("loadTaskState command = %#v", command)
	}
	writeFakeOK(t, peer, request.RequestID, map[string]any{
		"type":            "taskState",
		"defaultCwd":      "/workspace",
		"projectlessRoot": "/workspace/no-project",
		"projects": []any{map[string]any{
			"id": "project-a", "name": "项目甲", "path": "/workspace/a", "created_at": 11,
		}},
		"sessions": []any{map[string]any{
			"id": "session-a", "title": "任务甲", "auto_title": "自动标题", "project_id": "project-a",
			"provider": "claude", "model": "sonnet", "status": "idle",
			"created_at": 12, "updated_at": 13, "last_reply_at": 14,
		}},
	})
	select {
	case got := <-answers:
		if got.err != nil {
			t.Fatal(got.err)
		}
		if got.state.DefaultCwd != "/workspace" || got.state.ProjectlessRoot == nil || *got.state.ProjectlessRoot != "/workspace/no-project" {
			t.Fatalf("task state roots = %+v", got.state)
		}
		if len(got.state.Projects) != 1 || got.state.Projects[0].CreatedAt != 11 || got.state.Projects[0].Name != "项目甲" {
			t.Fatalf("projects = %+v", got.state.Projects)
		}
		if len(got.state.Sessions) != 1 {
			t.Fatalf("sessions = %+v", got.state.Sessions)
		}
		session := got.state.Sessions[0]
		if session.AutoTitle == nil || *session.AutoTitle != "自动标题" || session.ProjectID != "project-a" || session.CreatedAt != 12 || session.UpdatedAt != 13 || session.LastReplyAt == nil || *session.LastReplyAt != 14 {
			t.Fatalf("session snake_case fields = %+v", session)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("LoadTaskState did not complete")
	}
}

func TestHydrateSessionDecodesMessagesAndTranscript(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	type answer struct {
		session *AgentSession
		err     error
	}
	answers := make(chan answer, 1)
	go func() {
		session, err := client.HydrateSession(context.Background(), "session-a")
		answers <- answer{session: session, err: err}
	}()
	request := awaitFakeRequest(t, peer)
	if request.SessionID != NilUUID || request.RuntimeID != NilUUID {
		t.Fatalf("hydrateSession IDs = %q/%q", request.SessionID, request.RuntimeID)
	}
	command := decodeFakeCommand(t, request)
	if command["type"] != "hydrateSession" || command["sessionId"] != "session-a" {
		t.Fatalf("hydrateSession command = %#v", command)
	}
	writeFakeOK(t, peer, request.RequestID, map[string]any{
		"type": "session",
		"session": map[string]any{
			"id": "session-a", "title": "任务甲", "project_id": "project-a", "provider": "claude",
			"status": "idle", "created_at": 12, "updated_at": 13,
			"messages": []any{
				map[string]any{"id": "message-a", "turn_id": "turn-a", "role": "user", "content": "内部内容", "display_content": "你好", "created_at": 15, "streaming": false},
				map[string]any{"id": "message-b", "role": "assistant", "content": "**回答**", "created_at": 16, "streaming": false},
			},
			"transcript_blocks": []any{map[string]any{
				"after_message": 1, "turn_id": "turn-a",
				"content": map[string]any{"kind": "activities", "data": []any{map[string]any{
					"id": "activity-a", "source_id": "tool-a", "kind": "command", "title": "运行测试",
					"detail": "go test ./...", "arguments": "{}", "output": "ok", "failed": false, "complete": true,
					"file_changes": []any{map[string]any{"path": "main.go", "additions": 2, "deletions": 1, "status": "modified", "diff": "@@\n-old\n+new"}},
				}}},
			}},
		},
	})
	select {
	case got := <-answers:
		if got.err != nil {
			t.Fatal(got.err)
		}
		session := got.session
		if session == nil || len(session.Messages) != 2 || len(session.TranscriptBlocks) != 1 {
			t.Fatalf("hydrated session = %+v", session)
		}
		if session.Messages[0].TurnID == nil || *session.Messages[0].TurnID != "turn-a" || session.Messages[0].VisibleContent() != "你好" || session.Messages[1].VisibleContent() != "**回答**" {
			t.Fatalf("messages = %+v", session.Messages)
		}
		block := session.TranscriptBlocks[0]
		if block.AfterMessage != 1 || block.TurnID == nil || *block.TurnID != "turn-a" || block.Content.Kind != "activities" {
			t.Fatalf("transcript block = %+v", block)
		}
		var activities []ActivityItem
		if err := json.Unmarshal(block.Content.Data, &activities); err != nil {
			t.Fatal(err)
		}
		if len(activities) != 1 || activities[0].SourceID == nil || *activities[0].SourceID != "tool-a" || len(activities[0].FileChanges) != 1 || activities[0].FileChanges[0].Diff == nil {
			t.Fatalf("activities = %+v", activities)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("HydrateSession did not complete")
	}
}

func TestAttachSessionAndPrompt(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	type runtimeAnswer struct {
		runtime *SessionRuntime
		err     error
	}
	runtimeAnswers := make(chan runtimeAnswer, 1)
	go func() {
		runtime, err := client.AttachSession(context.Background(), "session-a")
		runtimeAnswers <- runtimeAnswer{runtime: runtime, err: err}
	}()
	request := awaitFakeRequest(t, peer)
	if request.SessionID != "session-a" || request.RuntimeID != NilUUID {
		t.Fatalf("attachSession IDs = %q/%q", request.SessionID, request.RuntimeID)
	}
	if command := decodeFakeCommand(t, request); !reflect.DeepEqual(command, map[string]any{"type": "attachSession"}) {
		t.Fatalf("attachSession command = %#v", command)
	}
	writeFakeOK(t, peer, request.RequestID, map[string]any{
		"type": "sessionRuntime", "runtimeId": "runtime-a", "supportsSteer": true,
	})
	select {
	case got := <-runtimeAnswers:
		if got.err != nil || got.runtime == nil || got.runtime.RuntimeID != "runtime-a" || !got.runtime.SupportsSteer {
			t.Fatalf("AttachSession = %+v, %v", got.runtime, got.err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("AttachSession did not complete")
	}

	promptAnswers := make(chan error, 1)
	go func() {
		promptAnswers <- client.Prompt(context.Background(), "session-a", "runtime-a", "继续实现")
	}()
	request = awaitFakeRequest(t, peer)
	if request.SessionID != "session-a" || request.RuntimeID != "runtime-a" {
		t.Fatalf("prompt IDs = %q/%q", request.SessionID, request.RuntimeID)
	}
	command := decodeFakeCommand(t, request)
	if !reflect.DeepEqual(command, map[string]any{"type": "prompt", "prompt": "继续实现"}) {
		t.Fatalf("prompt command = %#v", command)
	}
	writeFakeOK(t, peer, request.RequestID, map[string]any{"type": "ack"})
	select {
	case err := <-promptAnswers:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("Prompt did not complete")
	}
}

func TestWorkbenchRejectsUnexpectedPayload(t *testing.T) {
	address, peers := newFakeDaemon(t)
	client, peer := connectFakeDaemon(t, address, peers)
	answers := make(chan error, 1)
	go func() {
		_, err := client.LoadTaskState(context.Background())
		answers <- err
	}()
	request := awaitFakeRequest(t, peer)
	writeFakeOK(t, peer, request.RequestID, map[string]any{"type": "ack"})
	select {
	case err := <-answers:
		if err == nil || err.Error() != `unexpected client-daemon response payload "ack"; want "taskState"` {
			t.Fatalf("unexpected payload error = %v", err)
		}
	case <-time.After(fakeDaemonTimeout):
		t.Fatal("LoadTaskState did not complete")
	}
}
