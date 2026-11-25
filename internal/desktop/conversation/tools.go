package conversation

import (
	"cmp"
	"fmt"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"
)

// Layout metrics.
const detailMax = 480 // mono details

// textSize is the size of the transcript's content text in DIPs: the
// message text size setting (kit.MessageTextSize, 14 unless set), which the
// message bodies take too. Every text of a turn derives from it and keeps
// what it was apart from 14: a mono detail is a DIP smaller, counts and
// captions two. Controls (buttons, menus, dialogs, tooltips) keep Kit's
// sizes.
func textSize(c *ui.Context) float32 { return kit.MessageTextSize(c) }

// A row's metrics for text of size, the reference's at 14: 24 DIPs high,
// its icon 16 and its caret 12.
func rowHeight(size float32) float32 { return size + 10 }
func rowIcon(size float32) float32   { return size + 2 }
func caretSize(size float32) float32 { return size - 2 }

// railMargin puts the rail's 1 DIP line under the icon's middle, and
// railPad starts the body where the label does, past the icon and the
// row's gap of 8: 7.5 and 15.5 at 14.
func railMargin(size float32) float32 { return rowIcon(size)/2 - 0.5 }
func railPad(size float32) float32    { return rowIcon(size)/2 + 7.5 }

// opened reports whether a disclosure is open. They start closed, and the
// state keeps the open ones alone, so the rows a long transcript scrolls
// through leave nothing behind.
func (s *State) opened(key string) bool { return s.open[key] }

// toggle opens a closed disclosure and closes an open one.
func (s *State) toggle(key string) {
	if s.open[key] {
		delete(s.open, key)
	} else {
		s.open[key] = true
	}
}

// rowColor is the color of a row's icon and label, first match winning.
func rowColor(p *theme.Palette, call Call) ui.Color {
	switch {
	case call.Failed:
		return p.Danger
	case call.Pending || flagged(call):
		return p.Warning
	case call.Running:
		return p.MutedForeground.Mix(p.Foreground, 0.35)
	}
	return p.MutedForeground
}

// flagged is whether a row takes the warning color: a denied,
// interrupted or unexecuted call, or one whose background command ended
// badly. Only a failed call (Failed: the call itself did not run) is red;
// an error the tool returned from a call that ran (Warning without a state)
// reads as a plain row.
func flagged(call Call) bool {
	return call.Warning && call.State != StateNone || backgroundFailed(call.Background)
}

// backgroundFailed is a background command that did not end well: it
// failed, timed out, was lost or ended unconfirmed. One the user stopped
// reads plain.
func backgroundFailed(b *Background) bool {
	if b == nil || b.Running {
		return false
	}
	switch b.Outcome {
	case OutcomeFailed, OutcomeTimedOut, OutcomeInterrupted, OutcomeUnknown:
		return true
	}
	return false
}

// summary builds a row's content: status icon, label, input detail and caret.
// A running row shimmers its label and detail with one band across the
// row, sw's; the icon has none.
func summary(c *ui.Context, call Call, hovered, caret, open bool, sw *kit.Sweep) {
	p := theme.Of(c)
	size := textSize(c)
	color := rowColor(p, call)
	quiet := !call.Failed && !call.Pending && !flagged(call)
	if hovered && quiet {
		color = p.Foreground
	}
	icon := icons.ByName(call.Icon)
	if call.Pending {
		icon = icons.Help
	} else if icon == nil {
		icon = icons.Settings2
	}
	ic := ui.Icon(c, icon).FontSize(rowIcon(size)).Shrink(0).TextColor(color)
	if call.wrap {
		ic.Margin((rowHeight(size)-rowIcon(size))/2, 0, 0, 0)
	}
	label := call.Label
	if label == "" && call.LabelKey != "" {
		label = i18n.T(c, call.LabelKey)
	}
	shimmer := call.Running && quiet
	switch {
	case label != "" && (shimmer || !call.wrap):
		// Laid out as the sweep draws it (kit.Sweep.Text), running or not,
		// so the label stays where it is when the row settles.
		span := ui.Span{Text: label, Size: size, Color: color}
		t := levelSpan(c, ui.RichText(c, span).SingleLine().Shrink(0), span, size, call.wrap)
		if shimmer {
			sw.Text(c, t, span)
		}
	case call.wrap:
		level(c, ui.Text(c, label).FontSize(size).Grow(1).Shrink(1).MinWidth(0).TextColor(color), size, true, label)
	}
	detail, code := call.Detail, call.Code
	if detail != "" {
		dc := p.MutedForeground.Alpha(0.65)
		if !quiet {
			dc = color
		}
		span := ui.Span{Text: detail, Size: size, Color: dc}
		if code {
			span.Font, span.Size = "monospace", size-1
		}
		// Laid out as the sweep draws it, on the label's baseline.
		d := levelSpan(c, ui.RichText(c, span).SingleLine().Grow(1).Shrink(1).MinWidth(0), span, size, call.wrap)
		if code {
			d.MaxWidth(detailMax)
		}
		if shimmer {
			sw.Text(c, d, span)
		}
	}
	if caret {
		chevron := icons.ChevronRight
		if open {
			chevron = icons.ChevronDown
		}
		ch := ui.Icon(c, chevron).FontSize(caretSize(size)).Shrink(0).TextColor(color)
		if call.wrap {
			ch.Margin((rowHeight(size)-caretSize(size))/2, 0, 0, 0)
		}
		if !hovered {
			ch.Opacity(0)
		}
	}
}

// level puts a row's text of size level with its icon (kit.Level): centred
// in the row, or on the first line of a row that wraps.
func level(c *ui.Context, e ui.Element, size float32, wrap bool, label string) ui.Element {
	if wrap {
		return kit.LevelTop(c, e, size, rowHeight(size), label)
	}
	return kit.Level(c, e, size, label)
}

// levelSpan is level for a single-line ui.RichText of span in a row whose
// text is of size (kit.LevelSpan): what a sweep crosses, and a row's
// detail.
func levelSpan(c *ui.Context, e ui.Element, span ui.Span, size float32, wrap bool) ui.Element {
	if wrap {
		return kit.LevelSpanTop(c, e, span, size, rowHeight(size))
	}
	return kit.LevelSpan(c, e, span, size)
}

// rowSize gives a row the summary's height for text of size: 24 DIPs at
// 14, or, for a label that wraps, the full width, at least that tall, with
// the icon and caret where a row of one line has them, level with
// the first line (kit.LevelTop).
func rowSize(e ui.Element, size float32, wrap bool) ui.Element {
	if wrap {
		return e.FillWidth().MinHeight(rowHeight(size)).AlignItems(ui.Start)
	}
	return e.Height(rowHeight(size))
}

// plain is a row without details: no caret.
func plain(c *ui.Context, key any, call Call) {
	row := rowSize(ui.Row(c.Key(key)), textSize(c), call.wrap).Gap(8).MaxWidthPercent(100)
	hovered := row.Hovered()
	sw := &kit.Sweep{}
	row.Children(func() { summary(c, call, hovered, false, false, sw) })
	sw.Frame(row)
}

// pathRow is a file a call touched, as a plain row: it opens the file as a
// link of the content does (State.follow) when the application opens files.
func (s *State) pathRow(c *ui.Context, path string) {
	call := Call{Label: "", Icon: "file-text", Detail: path, Code: true}
	if s.OpenLink == nil {
		plain(c, path, call)
		return
	}
	session := s.renderSession
	row := rowSize(ui.ButtonBase(c.Key(path)).Label(path), textSize(c), false).Gap(8).AlignSelf(ui.Start).MaxWidthPercent(100).
		Justify(ui.Start).Cursor(ui.CursorPointer)
	hovered := row.Hovered() || row.FocusVisible()
	sw := &kit.Sweep{}
	row.Children(func() { summary(c, call, hovered, false, false, sw) }).OnClick(func() { s.follow(c, session, pathLink(path)) })
	sw.Frame(row)
}

// disclosure is a row whose body shows below it on a rail while open.
// Opening is instant, as the reference's (Kit Collapsible without a
// motion_id).
// head builds the row's content; texts it gives the row's sweep shimmer
// as one band across the row.
func (s *State) disclosure(c *ui.Context, key string, head func(hovered, open bool, sw *kit.Sweep), top, bottom float32, body func()) {
	s.wrapping(c, key, false, head, top, bottom, body)
}

// wrapping is a disclosure whose row may wrap its label (rowSize).
func (s *State) wrapping(c *ui.Context, key string, wrap bool, head func(hovered, open bool, sw *kit.Sweep), top, bottom float32, body func()) {
	ownerKey := s.renderSession + "\x00" + key
	open := s.opened(ownerKey)
	size := textSize(c)
	ui.Column(c.Key(key)).Children(func() {
		t := rowSize(ui.ButtonBase(c), size, wrap).Gap(8).AlignSelf(ui.Start).MaxWidthPercent(100).
			Justify(ui.Start).Expanded(open).Cursor(ui.CursorPointer)
		hovered := t.Hovered() || t.FocusVisible()
		sw := &kit.Sweep{}
		t.Children(func() { head(hovered, open, sw) }).OnClick(func() { s.toggle(ownerKey) })
		sw.Frame(t)
		if open {
			Rail(c).Padding(top, 0, bottom, railPad(size)).Children(body)
		}
	})
}

// railGap is between the rows on a rail.
const railGap = 8

// Rail is the indented body of a disclosure: a 1 DIP line under the icon.
func Rail(c *ui.Context) ui.Element {
	return ui.Column(c).MarginStart(railMargin(textSize(c))).BorderWidth(0, 0, 0, 1).BorderColor(theme.Of(c).Border).Gap(railGap)
}

func (s *State) tools(c *ui.Context, calls []Call) {
	runs := Runs(calls)
	if len(runs) > 1 {
		head := Summary(calls)
		s.disclosure(c, "tool-sequence-"+calls[0].ID, func(hovered, open bool, sw *kit.Sweep) {
			summary(c, head, hovered, true, open, sw)
		}, 8, 8, func() {
			groupRows(c, len(runs), func(i int) { s.run(c, runs[i]) })
		})
		return
	}
	if len(runs) != 0 {
		s.run(c, runs[0])
	}
}

// groupRows lays out ordinary transcript content. Only the mounted body
// owns the page count, so reopening a group starts on its latest rows.
func groupRows(c *ui.Context, n int, draw func(int)) {
	box := ui.Column(c.Key("rows")).Gap(railGap)
	more := ui.Local(box, "earlier", func() int { return 0 })
	first := max(0, n-shownRows-*more)
	box.Children(func() {
		if first > 0 {
			count := min(earlierPage, first)
			revealRow(c, "earlier", "chevron-up", i18n.T(c, "tool_rows_earlier", "count", count), func() { *more += count })
		}
		for i := first; i < n; i++ {
			draw(i)
		}
	})
}

func revealRow(c *ui.Context, key, icon, label string, click func()) {
	row := rowSize(ui.ButtonBase(c.Key(key)).Label(label), textSize(c), false).Gap(8).AlignSelf(ui.Start).
		MaxWidthPercent(100).Justify(ui.Start).Cursor(ui.CursorPointer)
	hovered := row.Hovered() || row.FocusVisible()
	row.Children(func() { summary(c, Call{Label: label, Icon: icon}, hovered, false, false, &kit.Sweep{}) }).OnClick(click)
}

// run groups calls of one name under the last one's summary.
func (s *State) run(c *ui.Context, run []Call) {
	if len(run) == 1 && !pathList(run[0]) {
		s.row(c, run[0])
		return
	}
	head := Summary(run)
	if len(run) == 1 && run[0].Output == nil && run[0].BackgroundDetail == nil {
		plain(c, "tool-group-"+run[0].ID, head)
		return
	}
	s.disclosure(c, "tool-group-"+run[0].ID, func(hovered, open bool, sw *kit.Sweep) {
		summary(c, head, hovered, true, open, sw)
	}, 8, 8, func() {
		if pathList(run[len(run)-1]) {
			s.pathRows(c, run)
			return
		}
		groupRows(c, len(run), func(i int) { s.row(c, run[i]) })
	})
}

func (s *State) pathRows(c *ui.Context, run []Call) {
	paths := uniquePaths(run)
	var details []Call
	for _, call := range run {
		if call.Background != nil || call.BackgroundDetail != nil || call.Output != nil && call.Output.Failure != "" {
			details = append(details, call)
		}
	}
	groupRows(c, len(paths)+len(details), func(i int) {
		if i < len(paths) {
			s.pathRow(c, paths[i])
			return
		}
		call := details[i-len(paths)]
		if call.Background != nil || call.BackgroundDetail != nil {
			s.row(c, call)
		} else {
			notice(c, call.Output.Failure, failureTone(c, call.Output))
		}
	})
}

// row is one call: its body is its output.
func (s *State) row(c *ui.Context, call Call) {
	if call.Name == "generate_image" && call.Output != nil && len(call.Output.Images) > 0 {
		// Generated pictures belong in the answer and output inventory. Keep the
		// original references for reply embeds without repeating the preview here.
		output := *call.Output
		output.Images = nil
		call.Output = &output
	}
	if call.Load != nil && !call.TrackLoad {
		if s.requestedDetails == nil {
			s.requestedDetails = map[string]bool{}
		}
		ownerKey := s.renderSession + "\x00tools-" + call.ID
		if s.opened(ownerKey) {
			if !s.requestedDetails[ownerKey] {
				s.requestedDetails[ownerKey] = true
				call.Load()
			}
		} else {
			delete(s.requestedDetails, ownerKey)
		}
	}
	// A call whose loaded output has nothing to draw does not open: its
	// row already says all there is.
	card := s.toolCards[s.renderSession][call.ID]
	if call.Background == nil && call.BackgroundDetail == nil && (call.Output == nil || call.Load == nil && card == nil && bare(call)) {
		plain(c, "tools-"+call.ID, call)
		return
	}
	s.disclosure(c, "tools-"+call.ID, func(hovered, open bool, sw *kit.Sweep) {
		summary(c, call, hovered, true, open, sw)
	}, 4, 8, func() {
		if call.TrackLoad && call.Load != nil {
			call.Load()
		}
		if b := call.Background; b != nil {
			notice(c, backgroundStatus(c, b), rowColor(theme.Of(c), call))
			if b.Preview != "" && (call.Output == nil || call.Output.Text != b.Preview && call.Output.Failure != b.Preview) {
				s.output(c, &Output{Text: b.Preview, Command: true})
			}
		}
		if call.BackgroundDetail != nil {
			detail := call.BackgroundDetail()
			if detail.Denied {
				notice(c, i18n.T(c, "permission_denied"), theme.Of(c).Warning)
			}
			if detail.Status != nil {
				withStatus := call
				withStatus.Background = detail.Status
				notice(c, backgroundStatus(c, detail.Status), rowColor(theme.Of(c), withStatus))
			}
			if detail.Output != nil {
				s.output(c, detail.Output)
			}
		}
		if call.Output != nil && (call.Background == nil || !bare(call)) {
			s.output(c, call.Output)
		}
		if card != nil {
			card(c)
		}
	})
}

func backgroundStatus(c *ui.Context, b *Background) string {
	if b.Running {
		return i18n.T(c, "background_running")
	}
	keys := [...]string{"background_outcome_completed", "background_outcome_failed", "background_outcome_cancelled",
		"background_outcome_stopped", "background_outcome_timed_out", "background_outcome_interrupted",
		"background_outcome_unknown", "background_outcome_stalled"}
	label := i18n.T(c, keys[b.Outcome])
	if b.Exit != nil {
		label = i18n.T(c, "background_exit", "code", *b.Exit)
	}
	if b.Waiting {
		label += " · " + i18n.T(c, "background_result_waiting")
	}
	return label
}

// bare reports an output with nothing to display below its row.
func bare(call Call) bool {
	o := call.Output
	if o.Table != nil || o.Diff != nil || o.Text != "" || len(o.Images) > 0 || len(o.Notices) > 0 || o.Partial {
		return false
	}
	return o.Failure == ""
}

func failureTone(c *ui.Context, o *Output) ui.Color {
	if o.ToolFailure {
		return theme.Of(c).Warning
	}
	return theme.Of(c).Danger
}

// notice is a line of the output's notes.
func notice(c *ui.Context, text string, tone ui.Color) {
	ui.Text(c, text).FontSize(textSize(c)).Padding(8, 12).TextColor(tone).Selectable()
}

// output builds a call's result: failure, table, diff, text, notices, with
// a copy button showing while the pointer is over it.
func (s *State) output(c *ui.Context, o *Output) {
	p := theme.Of(c)
	size := textSize(c)
	box := ui.Column(c).Gap(4)
	hovered := box.Hovered()
	box.Children(func() {
		if o.Failure != "" {
			notice(c, o.Failure, failureTone(c, o))
		}
		if o.Table != nil {
			table(c, o.Table)
		}
		if o.Diff != nil {
			s.diff(c, o.Diff)
		}
		if o.Text != "" {
			full := ui.Local(box, "text-full", func() bool { return false })
			lines := strings.Split(strings.TrimSuffix(o.Text, "\n"), "\n")
			shown, hidden := previewLinesOf(lines, *full, o.Command)
			ui.Text(c, strings.Join(shown, "\n")).Font("monospace").FontSize(size).LineHeight(20.0 / 14).
				Selectable().TextColor(p.MutedForeground).PaddingEnd(28)
			previewToggle(c, "text-more", full, len(lines), hidden)
		}
		if len(o.Images) > 0 {
			s.thumbnails(c, "output-images", o.Images, false)
		}
		for _, n := range o.Notices {
			notice(c, n, p.MutedForeground)
		}
		if o.Partial {
			notice(c, i18n.T(c, "tool_partial"), p.MutedForeground)
		}
		b := kit.IconButton(c.Key("copy"), icons.Copy, i18n.T(c, "tool_copy"), 20, false).
			TextColor(p.MutedForeground).Absolute().Top(4).Right(4)
		if !hovered && !b.FocusVisible() {
			b.Opacity(0)
		}
		b.OnClick(func() { c.WriteClipboard(copyOf(o)) })
	})
}

// copyOf is what a call's copy button copies.
func copyOf(o *Output) string {
	var parts []string
	for _, s := range []string{o.Failure, o.Text} {
		if s != "" {
			parts = append(parts, s)
		}
	}
	if o.Diff != nil {
		for _, l := range o.Diff.Lines {
			parts = append(parts, string(o.Diff.Op(l))+l.Text)
		}
	}
	return strings.Join(parts, "\n")
}

// Op returns a diff line's marker.
func (*Diff) Op(l DiffLine) byte {
	if l.Op == 0 {
		return ' '
	}
	return l.Op
}

// diff is a file change preview: an outlined box, its header the file name
// and +N −M, its body fitted to the visible diff in Kit's code size. The
// name opens the file as a link of the content does (State.follow) when the
// application opens files.
func (s *State) diff(c *ui.Context, d *Diff) {
	p := theme.Of(c)
	size := textSize(c)
	name := d.Path
	if i := strings.LastIndexByte(name, '/'); i >= 0 {
		name = name[i+1:]
	}
	ui.Column(c).Border(1, p.Border).Radius(p.Radius2xl()).Background(p.Background).Clip().Children(func() {
		ui.Row(c).Height(size+18).PaddingX(12).Gap(8).BorderWidth(0, 0, 1, 0).BorderColor(p.Border).
			TextColor(p.MutedForeground).Children(func() {
			label := ui.Text(c, name).FontSize(size).SingleLine().Shrink(1)
			if s.OpenLink != nil && d.Path != "" {
				session := s.renderSession
				label.Cursor(ui.CursorPointer).OnClick(func() { s.follow(c, session, pathLink(d.Path)) })
			}
			kit.Tooltip(c, label, d.Path)
			ui.Spacer(c)
			ui.Row(c).Gap(6).Shrink(0).Children(func() {
				if d.Added > 0 || d.Removed == 0 {
					ui.Textf(c, "+%d", d.Added).Font("monospace").FontSize(size - 2).TextColor(p.Success)
				}
				if d.Removed > 0 {
					ui.Textf(c, "−%d", d.Removed).Font("monospace").FontSize(size - 2).TextColor(p.Danger)
				}
			})
		})
		box := ui.Column(c.Key("diff"))
		full := ui.Local(box, "diff-full", func() bool { return false })
		lines, hidden := previewLinesOf(d.Lines, *full, false)
		old, new := (&Diff{Lines: lines}).Texts()
		kit.Diff(c, box, old, new, kit.DiffOptions{Language: editor.LanguageFor(d.Path), Fit: true, NoNumbers: true})
		if len(d.Lines) > previewLines {
			ui.Column(c).PaddingX(12).BorderWidth(1, 0, 0, 0).BorderColor(p.Border).Children(func() {
				previewToggle(c, "diff-more", full, len(d.Lines), hidden)
			})
		}
	})
}

// Table columns scroll sideways; rows take their height in the transcript.
func table(c *ui.Context, t *Table) {
	p := theme.Of(c)
	size := textSize(c)
	height := size + 14
	box := ui.ScrollHorizontal(c.Key("table"))
	full := ui.Local(box, "table-full", func() bool { return false })
	rows, hidden := previewLinesOf(t.Rows, *full, false)
	box.Children(func() {
		cells := func(values []string, head bool) {
			ui.Row(c).Height(height).BorderWidth(0, 0, 1, 0).BorderColor(p.Border).Children(func() {
				for _, v := range values {
					cell := ui.Text(c, v).FontSize(size).SingleLine().Width(180).PaddingX(8)
					kit.Tooltip(c, cell, v)
					if head {
						cell.FontWeight(500).TextColor(p.MutedForeground)
					}
				}
			})
		}
		cells(t.Columns, true)
		for _, r := range rows {
			cells(r, false)
		}
	})
	previewToggle(c, "table-more", full, len(t.Rows), hidden)
}

func previewToggle(c *ui.Context, key string, full *bool, total, hidden int) {
	if total <= previewLines {
		return
	}
	if *full {
		if hidden > 0 {
			notice(c, i18n.T(c, "diff_more_lines", "count", hidden), theme.Of(c).MutedForeground)
		}
		revealRow(c, key, "chevron-up", i18n.T(c, "output_show_less"), func() { *full = false })
		return
	}
	revealRow(c, key, "chevron-down", i18n.T(c, "output_show_all", "count", total), func() { *full = true })
}

// steps is a task's progress.
func (s *State) steps(c *ui.Context, key string, steps []Step, active bool) {
	done := 0
	for _, st := range steps {
		if st.State == StepCompleted || st.State == StepSkipped {
			done++
		}
	}
	head := Call{Label: i18n.T(c, "task_progress"), Icon: "circle-check", Detail: fmt.Sprintf("%d/%d", done, len(steps)), Running: active}
	// The plan is what the turn works through: it stays open, with no fold,
	// under its count.
	ui.Column(c.Key("progress-" + key)).Children(func() {
		plain(c, "head", head)
		Rail(c).Padding(4, 0, 8, railPad(textSize(c))).Children(func() {
			ui.Scroll(c).MaxHeight(320).PaddingEnd(12).Children(func() { stepRows(c, steps, active, textSize(c), false) })
		})
	})
}

// Steps are a task's progress rows: its state's icon and the step, settled
// ones struck through, the step in progress shimmering while active. The
// composer's task capsule shows them, in Kit's 14; the transcript shows
// them in its text size (stepRows).
func Steps(c *ui.Context, steps []Step, active bool) { stepRows(c, steps, active, 14, false) }

// QuietSteps are Steps for a summary that lists a whole plan at a glance
// (the session overview): a step to a line of 32 DIPs, cut short with its
// whole text as the tooltip, the settled ones muted but not struck through.
func QuietSteps(c *ui.Context, steps []Step, active bool) { stepRows(c, steps, active, 14, true) }

// stepRows are Steps for text of size: each row 32 DIPs at 14 for a step
// of one line, a long step wrapping whole under its first line's icon, or
// cut to its one line in a summary.
func stepRows(c *ui.Context, steps []Step, active bool, size float32, summary bool) {
	p := theme.Of(c)
	for i, st := range steps {
		icon, color := icons.CircleCheck, p.MutedForeground
		switch st.State {
		case StepInProgress:
			icon = icons.Loader
		case StepCompleted:
			color = p.Success
		case StepSkipped:
			icon = icons.Minus
		case StepPaused:
			icon, color = icons.Pause, p.Warning
		case StepBlocked:
			icon, color = icons.TriangleAlert, p.Warning
		case StepFailed:
			icon, color = icons.CircleX, p.Danger
		}
		row := ui.Row(c.Key(i)).Gap(8)
		if summary {
			row.FillWidth().Height(32).PaddingEnd(8).AlignItems(ui.Center)
		} else {
			rowSize(row, size, true).Padding(4, 8, 4, 0)
		}
		row.Children(func() {
			// The step in progress turns while the run does, and stands
			// still once it stopped; the mark is level with the first line.
			mark := ui.Box(c.Key("mark")).Shrink(0).Center()
			if !summary {
				mark.Height(rowHeight(size))
			}
			mark.Children(func() {
				if st.State == StepInProgress && active {
					kit.BusySized(c, rowIcon(size)).Label(i18n.T(c, "chat_running"))
				} else {
					ui.Icon(c, icon).FontSize(rowIcon(size)).TextColor(color)
				}
			})
			text := ui.Text(c, st.Text).FontSize(size).Grow(1).Shrink(1).MinWidth(0).TextColor(p.Foreground)
			if st.State == StepCompleted || st.State == StepSkipped {
				text.TextColor(p.MutedForeground)
				if !summary {
					text.Strikethrough()
				}
			}
			if summary {
				level(c, text.SingleLine().Tooltip(st.Text), size, false, st.Text)
				return
			}
			level(c, text, size, true, st.Text)
		})
	}
}

// question is the history row of a question the model asked: its prompt, the
// state, and the answers below.
func (s *State) question(c *ui.Context, q *Question) {
	p := theme.Of(c)
	size := textSize(c)
	// The state shows as the icon alone, without a text.
	head := Call{Label: q.Prompt, Icon: "circle-check", Pending: q.State == QuestionPending, wrap: true}
	switch q.State {
	case QuestionPending:
		head.Icon = "bot"
	case QuestionCancelled, QuestionDeclined:
		head.Icon = "circle-x"
	case QuestionInterrupted, QuestionTimedOut:
		head.Icon = "triangle-alert"
	}
	if len(q.Answers) == 0 {
		plain(c, "question-"+q.CallID, head)
		return
	}
	s.wrapping(c, "question-"+q.CallID, true, func(hovered, open bool, sw *kit.Sweep) { summary(c, head, hovered, true, open, sw) }, 4, 8, func() {
		ui.Scroll(c).MaxHeight(100 * size / 14).Gap(4).Children(func() {
			for _, a := range q.Answers {
				ui.Text(c, a).FontSize(size).Selectable().TextColor(p.Foreground)
			}
		})
	})
}

// children is the rows of subagent sessions: one row, or a disclosure of
// their marks and count over a row each.
func (s *State) children(c *ui.Context, kids []Child, actions Actions) {
	if len(kids) == 1 {
		child(c, kids[0], actions)
		return
	}
	p := theme.Of(c)
	size := textSize(c)
	done, running := 0, false
	for _, k := range kids {
		if k.Status == ChildCompleted {
			done++
		}
		running = running || !k.Unsynced && k.Status.Active()
	}
	label := i18n.T(c, "subagent_count", "completed", done, "total", len(kids))
	s.disclosure(c, "children-"+cmp.Or(kids[0].Turn, kids[0].ID), func(hovered, open bool, sw *kit.Sweep) {
		color := p.MutedForeground
		if hovered {
			color = p.Foreground
		}
		ui.Row(c).Gap(4).Shrink(0).Children(func() {
			for _, k := range kids {
				kit.RoleIdenticon(c.Key(cmp.Or(k.Turn, k.ID)), k.ID, k.Avatar, !k.Unsynced && k.Status.Active(), rowIcon(size))
			}
		})
		// Laid out as the sweep draws it, running or not (summary).
		span := ui.Span{Text: label, Size: size, Color: color}
		t := levelSpan(c, ui.RichText(c, span).SingleLine(), span, size, false)
		if running {
			sw.Text(c, t, span)
		}
		ch := ui.Icon(c, map[bool]*ui.SVG{true: icons.ChevronDown, false: icons.ChevronRight}[open]).FontSize(caretSize(size)).TextColor(color)
		if !hovered {
			ch.Opacity(0)
		}
	}, 8, 0, func() {
		ui.Column(c).PaddingStart(16).Gap(8).Children(func() {
			for _, k := range kids {
				child(c, k, actions)
			}
		})
	})
}

// child is a subagent's row; a click opens its session.
func child(c *ui.Context, k Child, actions Actions) {
	p := theme.Of(c)
	size := textSize(c)
	title := k.Title
	if title == "" {
		title = i18n.T(c, "chat_child")
	}
	color := p.MutedForeground
	if k.Status == ChildFailed || k.Status == ChildInterrupted {
		color = p.Danger
	}
	b := ui.ButtonBase(c.Key("child-" + cmp.Or(k.Turn, k.ID))).Label(title).Height(rowHeight(size)).Gap(8).AlignSelf(ui.Start).Justify(ui.Start).Cursor(ui.CursorPointer)
	hovered := b.Hovered()
	if hovered && color == p.MutedForeground {
		color = p.Foreground
	}
	b.Children(func() {
		if k.Status == ChildContinued {
			ui.Icon(c, icons.ArrowRight).FontSize(rowIcon(size)).TextColor(color)
		} else {
			kit.RoleIdenticon(c, k.ID, k.Avatar, !k.Unsynced && k.Status.Active(), rowIcon(size))
		}
		if !k.Unsynced && k.Status.Active() {
			levelSpan(c, kit.Shimmer(c, title, size, color), ui.Span{Text: title, Size: size}, size, false)
		} else {
			level(c, ui.Text(c, title).FontSize(size).SingleLine().TextColor(color), size, false, title)
		}
		model := strings.TrimSpace(k.Model + " " + k.Effort)
		level(c, ui.Text(c, model).FontSize(size).SingleLine().TextColor(p.MutedForeground.Alpha(0.65)), size, false, model)
		ch := ui.Icon(c, icons.ChevronRight).FontSize(caretSize(size)).TextColor(color)
		if !hovered {
			ch.Opacity(0)
		}
	}).OnClick(func() { actions.OpenChild(k.ID) })
	keys := [...]string{ChildRunning: "chat_running", ChildCompleted: "turn_completed", ChildFailed: "turn_failed",
		ChildInterrupted: "chat_interrupted", ChildCancelled: "turn_cancelled", ChildQueued: "turn_queued",
		ChildStopping: "chat_stopping", ChildContinued: "turn_continued"}
	key := keys[k.Status]
	if k.Unsynced && k.Status.Active() {
		key = "chat_status_unsynced"
	}
	status := i18n.T(c, key)
	if k.Error != "" && (k.Status == ChildFailed || k.Status == ChildInterrupted) {
		status += "\n" + k.Error
	}
	kit.Tooltip(c, b, status)
}
