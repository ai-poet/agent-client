package shell

import (
	"fmt"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/daemon"
	"github.com/ai-poet/agent-client/desktop/internal/workbench"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

func TestTranscriptViewLoadingEmptyAndErrorStates(t *testing.T) {
	tests := []struct {
		name     string
		snapshot Snapshot
		want     []string
	}{
		{name: "loading", snapshot: Snapshot{SessionLoading: true}, want: []string{"载入会话…"}},
		{name: "empty", snapshot: Snapshot{}, want: []string{"这个会话还没有消息"}},
		{name: "error", snapshot: Snapshot{SessionError: "网络不可用"}, want: []string{"载入失败，可重试", "网络不可用", "重试"}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			model := NewModel(&fakeWorkbenchConnector{})
			setModelSnapshot(model, test.snapshot)
			model.state.Status = Connected
			model.state.SelectedSessionID = "session-a"
			tester := ui.NewTester(model.View, 900, 600)
			for _, text := range test.want {
				if !tester.HasText(text) {
					t.Errorf("view does not contain %q; texts: %q", text, tester.Texts())
				}
			}
		})
	}
}

func TestTranscriptViewHydrateOrderAndCount(t *testing.T) {
	session := &daemon.AgentSession{
		ID: "session-a",
		Messages: []daemon.Message{
			{ID: "user-1", Role: "user", Content: "请修改"},
			{ID: "assistant-1", Role: "assistant", Content: "已完成"},
			{ID: "system-1", Role: "system", Content: "系统提示"},
		},
		TranscriptBlocks: []daemon.TranscriptBlock{
			{AfterMessage: 1, Content: daemon.TranscriptContent{Kind: "activities", Data: rawJSON(t, []daemon.ActivityItem{{ID: "tool-1", Title: "读取文件", Detail: stringPointer("README.md"), Complete: true}})}},
		},
	}
	model := NewModel(&fakeWorkbenchConnector{})
	setModelSnapshot(model, Snapshot{
		Status: Connected, SelectedSessionID: session.ID,
		Sessions:   []SessionSummary{{ID: session.ID, Title: "会话甲"}},
		Transcript: workbench.ProjectTranscript(session), MessageCount: len(session.Messages),
	})
	if got := len(model.state.Transcript); got != 4 {
		t.Fatalf("hydrated entry count = %d, want 4", got)
	}
	tester := ui.NewTester(model.View, 900, 600)
	text := strings.Join(tester.Texts(), "\n")
	ordered := []string{"请修改", "读取文件", "已完成", "系统提示"}
	previous := -1
	for _, want := range ordered {
		at := strings.Index(text, want)
		if at < 0 {
			t.Fatalf("view does not contain %q; texts: %q", want, tester.Texts())
		}
		if at <= previous {
			t.Fatalf("%q is out of transcript order; texts: %q", want, tester.Texts())
		}
		previous = at
	}
}

func TestTranscriptViewStreamingAppendDoesNotDuplicate(t *testing.T) {
	model := NewModel(&fakeWorkbenchConnector{})
	model.markdown.Reveal = &markdown.Reveal{Unit: markdown.AtOnce}
	setModelSnapshot(model, Snapshot{
		Status: Connected, SelectedSessionID: "s", Sessions: []SessionSummary{{ID: "s"}},
	})
	load := model.state.SelectSession("s")
	model.state.ApplyHydrate(workbench.NewHydrateResult(
		load.Generation, load.SessionGeneration, "s", &daemon.AgentSession{ID: "s"}, nil,
	))
	tester := ui.NewTester(model.View, 900, 600)

	model.applyEvent(0, transcriptTextEvent("s", "r", 1, "你好"))
	tester.Frame()
	model.applyEvent(0, transcriptTextEvent("s", "r", 2, "，世界"))
	tester.Frame()

	snapshot := model.Snapshot()
	if len(snapshot.Transcript) != 1 || snapshot.Transcript[0].Text != "你好，世界" {
		t.Fatalf("streaming transcript = %+v", snapshot.Transcript)
	}
	text := strings.Join(tester.Texts(), "\n")
	if got := strings.Count(text, "你好，世界"); got != 1 {
		t.Fatalf("combined delta appears %d times; texts: %q", got, tester.Texts())
	}
}

func TestTranscriptViewFollowsEndOnlyUntilUserScrollsUp(t *testing.T) {
	model := NewModel(&fakeWorkbenchConnector{})
	model.markdown.Reveal = &markdown.Reveal{Unit: markdown.AtOnce}
	setModelSnapshot(model, Snapshot{Status: Connected, SelectedSessionID: "s"})
	for index := range 40 {
		model.state.Transcript = append(model.state.Transcript, TranscriptEntry{
			ID: fmt.Sprintf("message-%d", index), Kind: TranscriptAssistant, Title: "助手",
			Text: fmt.Sprintf("消息 %d\n\n%s", index, strings.Repeat("用于滚动的内容。", 4)),
		})
	}
	tester := ui.NewTester(model.View, 1000, 500)
	if !model.transcriptList.AtEnd() {
		t.Fatal("transcript did not start at its end")
	}

	model.state.Transcript = append(model.state.Transcript, TranscriptEntry{ID: "bottom-addition", Kind: TranscriptAssistant, Title: "助手", Text: "底部新增"})
	tester.Frame()
	if !model.transcriptList.AtEnd() {
		t.Fatal("transcript did not follow a new entry while at its end")
	}

	listBounds, ok := tester.Find("会话转录")
	if !ok {
		t.Fatal("transcript list has no bounds")
	}
	x, y := listBounds.X+listBounds.W/2, listBounds.Y+listBounds.H/2
	tester.Move(x, y)
	tester.Scroll(x, y, 0, -1e6)
	first, last := model.transcriptList.Visible()
	model.state.Transcript = append(model.state.Transcript,
		TranscriptEntry{ID: "above-addition-1", Kind: TranscriptAssistant, Title: "助手", Text: "上滑后新增一"},
		TranscriptEntry{ID: "above-addition-2", Kind: TranscriptAssistant, Title: "助手", Text: "上滑后新增二"},
	)
	tester.Frame()
	gotFirst, gotLast := model.transcriptList.Visible()
	if gotFirst != first || gotLast != last || model.transcriptList.AtEnd() {
		t.Fatalf("new entries moved a scrolled-up transcript: rows %d-%d -> %d-%d, atEnd=%v", first, last, gotFirst, gotLast, model.transcriptList.AtEnd())
	}

	model.transcriptList.ScrollToEnd()
	tester.Frame()
	model.state.Transcript = append(model.state.Transcript, TranscriptEntry{ID: "resumed-addition", Kind: TranscriptAssistant, Title: "助手", Text: "恢复跟随"})
	tester.Frame()
	if !model.transcriptList.AtEnd() {
		t.Fatal("transcript did not resume following after returning to its end")
	}
}
