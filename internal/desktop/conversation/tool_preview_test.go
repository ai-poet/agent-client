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

func TestLongToolOutputWrapsWithinTheRow(t *testing.T) {
	s := New()
	text := strings.Repeat("x", 800)
	tt := toolTester(func(c *ui.Context) { s.output(c, &Output{Text: text}) })
	r, ok := tt.Find(text)
	if !ok || r.W > 700 || r.H < 40 {
		t.Fatalf("long single line did not wrap in the viewport: %+v, found %v", r, ok)
	}
}

func TestDiffPreviewFitsVisibleLines(t *testing.T) {
	s := New()
	d := &Diff{Path: "example.go"}
	for i := range 20 {
		d.Lines = append(d.Lines, DiffLine{Op: '+', Text: fmt.Sprintf("var v%d = %d", i, i)})
	}
	tt := toolTester(func(c *ui.Context) {
		s.diff(c, d)
		ui.Text(c, "After diff")
	})
	after := func() float32 {
		r, ok := tt.Find("After diff")
		if !ok {
			t.Fatal("missing content after diff")
		}
		return r.Y
	}
	short := after()
	clickTool(t, tt, i18n.In("en", "output_show_all", "count", 20))
	if growth := after() - short; growth < 150 || growth > 180 {
		t.Fatalf("eight additional code lines grew the diff by %.1f DIPs", growth)
	}
	clickTool(t, tt, i18n.In("en", "output_show_less"))
	if after() != short {
		t.Fatal("collapsed diff did not return to preview height")
	}
}

func TestExpandedCallSurvivesNewCallsAndPaging(t *testing.T) {
	s, all := New(), readCalls(68)
	s.renderSession = "session"
	calls := all[:20]
	tt := toolTester(func(c *ui.Context) { s.tools(c, calls) })
	clickTool(t, tt, "Read 019")
	clickTool(t, tt, "Read 018")
	calls = all
	tt.Frame()
	if !tt.HasText("Result read18") || tt.HasText("Read 059") {
		t.Fatal("the open call was evicted or the recent window grew")
	}
	// 60 earlier rows minus the open call. The first page shows 50
	// collapsed rows, the second shows the remaining nine.
	clickTool(t, tt, i18n.In("en", "tool_rows_earlier", "count", 50))
	if !tt.HasText("Result read18") || tt.HasText("Read 008") || !tt.HasText("Read 009") {
		t.Fatal("the first page did not reveal 50 collapsed rows")
	}
	clickTool(t, tt, i18n.In("en", "tool_rows_earlier", "count", 9))
	if !tt.HasText("Read 000") {
		t.Fatal("the last page did not reach the oldest call")
	}
	clickTool(t, tt, "Read 067")
	clickTool(t, tt, "Read 067")
	if tt.HasText("Read 018") || tt.HasText("Read 059") || !tt.HasText("Read 060") {
		t.Fatal("reopening the group did not reset the earlier window")
	}
}

func TestExpandedTableAndClipboardIncludeEveryRow(t *testing.T) {
	table := &Table{Columns: []string{"ID", "Value"}}
	for i := range 405 {
		table.Rows = append(table.Rows, []string{fmt.Sprintf("row-%03d", i), "ok"})
	}
	table.Rows[0][1] = "one\ttwo"
	table.Rows[404][1] = "line\n\"quoted\""
	s := New()
	turns := []Turn{{ID: "turn", User: "Inspect table", Reply: Reply{Status: Running, Blocks: []Block{
		{Kind: ToolsBlock, Calls: []Call{{ID: "query", Name: "query", Label: "Query table", Output: &Output{Table: table}}}},
	}}}}
	tt := ui.NewTester(func(c *ui.Context) {
		theme.Apply(c, "", theme.Light)
		View(c, s, "session", turns, nil, still{})
	}, 700, 720)
	tt.SetLocale("en")
	clickTool(t, tt, "Query table")
	if !tt.HasText("row-011") || tt.HasText("row-012") {
		t.Fatal("table preview did not stop after 12 data rows")
	}
	// Copy from the short preview; the result includes the header and all rows.
	header, ok := tt.Find("Query table")
	if !ok {
		t.Fatal("no query header")
	}
	for _, b := range tt.AccessAll() {
		if b.Role == ui.RoleButton && b.Name == i18n.In("en", "tool_copy") && b.Bounds.Y > header.Y {
			tt.ClickAt(b.Bounds.X+b.Bounds.W/2, b.Bounds.Y+b.Bounds.H/2)
			break
		}
	}
	got := tt.Clipboard()
	if !strings.HasPrefix(got, "ID\tValue\nrow-000\t\"one\ttwo\"\n") ||
		!strings.HasSuffix(got, "row-404\t\"line\n\"\"quoted\"\"\"") || strings.Count(got, "\nrow-") != len(table.Rows) {
		t.Fatal("copy did not produce the full escaped TSV")
	}
	clickTool(t, tt, i18n.In("en", "output_show_all", "count", len(table.Rows)))
	tt.Scroll(300, 650, 0, 100000)
	tt.Frame()
	if row, ok := tt.Find("row-404"); !ok || row.Y+row.H <= 0 || row.Y >= 720 {
		t.Fatalf("the last row was not reachable using the outer scroll: %+v, found %v", row, ok)
	}
	clickTool(t, tt, i18n.In("en", "output_show_less"))
	if tt.HasText("row-012") {
		t.Fatal("Show less did not restore the table preview")
	}
}
