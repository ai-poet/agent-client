package shell

import (
	"image/png"
	"os"
	"strings"
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

	writeFrame(t, tester, path)
}

func TestCrychicTranscriptLayout(t *testing.T) {
	model := NewModel(&fakeWorkbenchConnector{})
	setModelSnapshot(model, Snapshot{
		Status: Connected, SelectedSessionID: "session-a",
		Transcript: []TranscriptEntry{
			{ID: "user-short", Kind: TranscriptUser, Title: "用户", Text: "短消息"},
			{ID: "assistant-long", Kind: TranscriptAssistant, Title: "助手", Text: "- " + strings.Repeat("长列表需要按内容区宽度正常换行，", 18)},
		},
	})
	tester := ui.NewTester(model.View, 1200, 800)
	tester.Advance(time.Second)

	list, ok := tester.Find("会话转录")
	if !ok {
		t.Fatal("transcript list has no bounds")
	}
	user, ok := tester.Find("转录消息：user-short")
	if !ok {
		t.Fatal("user bubble has no bounds")
	}
	assistant, ok := tester.Find("转录消息：assistant-long")
	if !ok {
		t.Fatal("assistant bubble has no bounds")
	}
	if user.Y < list.Y || user.Y > list.Y+24 {
		t.Fatalf("short transcript does not start at the top: list=%v, first bubble=%v", list, user)
	}
	if maxWidth := list.W * 0.78; assistant.W > maxWidth+1 || assistant.W < maxWidth*0.75 {
		t.Fatalf("long assistant bubble width = %v, want approximately 78%% of %v", assistant.W, list.W)
	}
	if user.W >= assistant.W/2 {
		t.Fatalf("short user bubble did not size to content: user=%v, assistant=%v", user.W, assistant.W)
	}
	role, ok := tester.Find("助手")
	if !ok {
		t.Fatal("assistant role label has no bounds")
	}
	if role.X < assistant.X || role.Y < assistant.Y || role.X+role.W > assistant.X+assistant.W || role.Y+role.H > assistant.Y+assistant.H {
		t.Fatalf("role label is outside its bubble: role=%v, bubble=%v", role, assistant)
	}
}

func TestCrychicTranscriptLongStringWrapping(t *testing.T) {
	longASCII := strings.Repeat("A", 120)
	longURL := "https://example.com/" + strings.Repeat("very-long-path-segment/", 12) + "?query=" + strings.Repeat("x", 120)
	chinese := strings.Repeat("中文内容应当按字符正常换行，不能撑破消息气泡。", 4)
	model := NewModel(&fakeWorkbenchConnector{})
	setModelSnapshot(model, Snapshot{
		Status: Connected, SelectedSessionID: "session-a",
		Transcript: []TranscriptEntry{
			{
				ID: "assistant-long-string", Kind: TranscriptAssistant, Title: "助手",
				Text: longASCII + "\n\n" + longURL + "\n\n" + chinese,
			},
		},
	})
	tester := ui.NewTester(model.View, 1200, 800)
	tester.Advance(time.Second)

	list, ok := tester.Find("会话转录")
	if !ok {
		t.Fatal("transcript list has no bounds")
	}
	bubble, ok := tester.Find("转录消息：assistant-long-string")
	if !ok {
		t.Fatal("long-string bubble has no bounds")
	}
	if maxWidth := list.W * 0.78; bubble.W > maxWidth+1 {
		t.Fatalf("long-string bubble width = %v, exceeds 78%% of transcript width %v", bubble.W, list.W)
	}
	for _, text := range []string{longASCII, longURL, chinese} {
		bounds, ok := tester.Find(text)
		if !ok {
			t.Fatalf("long-string paragraph has no bounds: %.40q", text)
		}
		if bounds.X < bubble.X-1 || bounds.Y < bubble.Y-1 || bounds.X+bounds.W > bubble.X+bubble.W+1 || bounds.Y+bounds.H > bubble.Y+bubble.H+1 {
			t.Errorf("paragraph overflows its bubble: text=%v, bubble=%v, content=%.40q", bounds, bubble, text)
		}
	}

	if path := os.Getenv("CHEAPROUTER_LONGSTRING_FRAME_PATH"); path != "" {
		writeFrame(t, tester, path)
	}
}

func writeFrame(t *testing.T, tester *ui.Tester, path string) {
	t.Helper()
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
