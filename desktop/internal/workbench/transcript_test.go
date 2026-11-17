package workbench

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
)

func rawJSON(t *testing.T, value any) json.RawMessage {
	t.Helper()
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

func TestProjectTranscriptPreservesMessageAndActivityOrder(t *testing.T) {
	activities := func(id, title string) json.RawMessage {
		return rawJSON(t, []daemon.ActivityItem{{ID: id, Kind: "command", Title: title, Complete: true}})
	}
	session := &daemon.AgentSession{
		Messages: []daemon.Message{
			{ID: "user-1", Role: "user", Content: "provider-facing", DisplayContent: stringPointer("请修改"), Streaming: false},
			{ID: "assistant-1", Role: "assistant", Content: "已完成", Streaming: false},
		},
		TranscriptBlocks: []daemon.TranscriptBlock{
			{AfterMessage: 1, Content: daemon.TranscriptContent{Kind: "activities", Data: activities("tool-1", "读取文件")}},
			{AfterMessage: 0, Content: daemon.TranscriptContent{Kind: "reasoning", Data: rawJSON(t, map[string]any{"content": "先检查"})}},
			{AfterMessage: 1, Content: daemon.TranscriptContent{Kind: "activities", Data: activities("tool-2", "修改文件")}},
			{AfterMessage: 4, Content: daemon.TranscriptContent{Kind: "activities", Data: activities("tool-3", "末尾工作")}},
		},
	}
	got := ProjectTranscript(session)
	var gotIDs []string
	for _, entry := range got {
		gotIDs = append(gotIDs, entry.ID)
	}
	wantIDs := []string{"reasoning-0-1", "user-1", "tool-1", "tool-2", "assistant-1", "tool-3"}
	if !reflect.DeepEqual(gotIDs, wantIDs) {
		t.Fatalf("transcript order = %#v, want %#v", gotIDs, wantIDs)
	}
	if got[1].Text != "请修改" || got[1].Kind != TranscriptUser {
		t.Fatalf("user display content = %+v", got[1])
	}
	if got[0].Kind != TranscriptReasoning || got[0].Text != "先检查" {
		t.Fatalf("reasoning = %+v", got[0])
	}
}

func TestStreamingProjectionAppendsAndSplitsAroundWork(t *testing.T) {
	event := daemon.SequencedEvent{SessionID: "s", RuntimeID: "r", Epoch: "e", Sequence: 1}
	event.Event = daemon.WireDriverEvent{Kind: "textDelta", Payload: rawJSON(t, "你好")}
	entries := ApplyTranscriptEvent(nil, event)
	event.Sequence = 2
	event.Event.Payload = rawJSON(t, "，世界")
	entries = ApplyTranscriptEvent(entries, event)
	if len(entries) != 1 || entries[0].Text != "你好，世界" || !entries[0].Streaming {
		t.Fatalf("coalesced text = %+v", entries)
	}

	event.Sequence = 3
	event.Event = daemon.WireDriverEvent{Kind: "richActivity", Payload: rawJSON(t, daemon.ActivityItem{
		ID: "tool-1", Kind: "command", Title: "运行测试", Detail: stringPointer("go test ./..."), Complete: false,
	})}
	entries = ApplyTranscriptEvent(entries, event)
	event.Sequence = 4
	event.Event.Payload = rawJSON(t, daemon.ActivityItem{
		ID: "tool-1", Kind: "command", Title: "运行测试", Output: stringPointer("ok"), Complete: true,
	})
	entries = ApplyTranscriptEvent(entries, event)
	if len(entries) != 2 || entries[1].Detail != "ok" || !entries[1].Complete {
		t.Fatalf("updated activity = %+v", entries)
	}

	event.Sequence = 5
	event.Event = daemon.WireDriverEvent{Kind: "textDelta", Payload: rawJSON(t, "测试通过")}
	entries = ApplyTranscriptEvent(entries, event)
	if len(entries) != 3 || entries[0].Streaming || !entries[2].Streaming || entries[2].Text != "测试通过" {
		t.Fatalf("text after activity = %+v", entries)
	}

	event.Sequence = 6
	event.Event = daemon.WireDriverEvent{Kind: "turnFinished", Payload: rawJSON(t, map[string]any{"success": true, "summary": ""})}
	entries = ApplyTranscriptEvent(entries, event)
	if entries[2].Streaming {
		t.Fatalf("turn finish left streaming entry: %+v", entries[2])
	}
}

func TestProjectionKeepsEventKindsAndErrorsOrdered(t *testing.T) {
	event := daemon.SequencedEvent{SessionID: "s", RuntimeID: "r", Epoch: "e", Sequence: 1}
	event.Event = daemon.WireDriverEvent{Kind: "reasoningDelta", Payload: rawJSON(t, "分析")}
	entries := ApplyTranscriptEvent(nil, event)
	event.Sequence = 2
	event.Event = daemon.WireDriverEvent{Kind: "error", Payload: rawJSON(t, " provider failed ")}
	entries = ApplyTranscriptEvent(entries, event)
	if len(entries) != 2 || entries[0].Kind != TranscriptReasoning || entries[0].Streaming || !entries[0].Complete || entries[1].Kind != TranscriptNotice || !entries[1].Failed {
		t.Fatalf("reasoning/error entries = %+v", entries)
	}
}

func stringPointer(value string) *string { return &value }
