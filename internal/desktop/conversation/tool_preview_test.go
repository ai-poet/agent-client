package conversation

import (
	"fmt"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

func toolTester(draw func(*ui.Context)) *ui.Tester {
	tt := ui.NewTester(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		ui.Scroll(c).Fill().Children(func() { draw(c) })
	}, 700, 720)
	tt.SetLocale("en")
	tt.Frame()
	return tt
}

func clickTool(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	if err := tt.Click(label); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
}

func readCalls(n int) []Call {
	calls := named("read", n)
	for i := range calls {
		calls[i].Label = fmt.Sprintf("Read %03d", i)
		calls[i].Output = &Output{Text: "Result " + calls[i].ID}
	}
	return calls
}

func TestToolGroupPagesAndResets(t *testing.T) {
	s, calls := New(), readCalls(77)
	tt := toolTester(func(c *ui.Context) { s.tools(c, calls) })
	clickTool(t, tt, "Read 076")
	for _, first := range []int{69, 19, 0} {
		for i, call := range calls {
			if tt.HasText(call.Label) != (i >= first) {
				t.Fatalf("page starting at %d: wrong visibility for %q", first, call.Label)
			}
		}
		if first != 0 {
			clickTool(t, tt, i18n.In("en", "tool_rows_earlier", "count", min(50, first)))
		}
	}
	clickTool(t, tt, "Read 076")
	clickTool(t, tt, "Read 076")
	if tt.HasText("Read 068") || !tt.HasText("Read 069") {
		t.Fatal("reopening the group kept an earlier page")
	}
}

func TestTextPreviewExpandsInPlace(t *testing.T) {
	lines := make([]string, 405)
	for i := range lines {
		lines[i] = fmt.Sprintf("line %03d", i)
	}
	for _, command := range []bool{false, true} {
		s := New()
		output := &Output{Text: strings.Join(lines, "\n") + "\n", Command: command}
		tt := toolTester(func(c *ui.Context) { s.output(c, output) })
		short, long := lines[:12], lines[:400]
		if command {
			short, long = lines[393:], lines[5:]
		}
		if !tt.HasText(strings.Join(short, "\n")) {
			t.Fatalf("command=%v: wrong preview", command)
		}
		clickTool(t, tt, i18n.In("en", "output_show_all", "count", 405))
		if !tt.HasText(strings.Join(long, "\n")) || !tt.HasText(i18n.In("en", "diff_more_lines", "count", 5)) {
			t.Fatalf("command=%v: wrong expanded range or missing overflow count", command)
		}
		tt.Scroll(300, 650, 0, 100000)
		tt.Frame()
		clickTool(t, tt, i18n.In("en", "output_show_less"))
		if !tt.HasText(strings.Join(short, "\n")) || copyOf(output) != output.Text {
			t.Fatalf("command=%v: collapse or full output copy failed", command)
		}
	}
}
