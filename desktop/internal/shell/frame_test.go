package shell

import (
	"image/png"
	"os"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/markdown"
)

// TestCrychicFrame renders the real workbench view without a window or daemon.
// Set CHEAPROUTER_FRAME_PATH to write the frame for visual inspection.
func TestCrychicFrame(t *testing.T) {
	path := os.Getenv("CHEAPROUTER_FRAME_PATH")
	if path == "" {
		t.Skip("set CHEAPROUTER_FRAME_PATH to save the rendered frame")
	}
	lastReplyAt := uint64(time.Date(2026, time.November, 17, 10, 30, 0, 0, time.Local).Unix())
	model := NewModel(&fakeWorkbenchConnector{})
	model.markdown.Reveal = &markdown.Reveal{Unit: markdown.AtOnce}
	setModelSnapshot(model, Snapshot{
		Status:            Connected,
		DaemonVersion:     "test-daemon",
		SelectedSessionID: "session-a",
		Sessions: []SessionSummary{
			{ID: "session-a", Title: "CrychicUI 组件切换", LastReplyAt: &lastReplyAt},
			{ID: "session-b", Title: "桌面工作台"},
		},
		Transcript: []TranscriptEntry{
			{ID: "user-1", Kind: TranscriptUser, Title: "用户", Text: "请把工作台切换到 **CrychicUI**，并保持现有行为。"},
			{ID: "assistant-1", Kind: TranscriptAssistant, Title: "助手", Text: "已完成组件切换：\n\n- 会话列表使用虚拟列表\n- 消息使用 Markdown 气泡\n- 输入框保留 Enter 发送"},
			{ID: "assistant-2", Kind: TranscriptAssistant, Title: "助手", Text: "流式 **Markdown** 仍由 `markdown.Stream` 渲染。", Streaming: true},
		},
	})
	tester := ui.NewTester(model.View, 1200, 800)
	tester.Advance(time.Second)

	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := file.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := png.Encode(file, tester.Image()); err != nil {
		t.Fatal(err)
	}
}
