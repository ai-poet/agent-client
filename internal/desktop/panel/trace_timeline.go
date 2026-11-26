package panel

import (
	"fmt"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// TraceMarker is an observed transcript event, not a measured interval.
type TraceMarker struct {
	Turn   string
	TimeMs int64
	Tool   bool
}

type tracePoint struct {
	TraceMarker
	row int
}

func tracePoints(rows []traceRow, markers []TraceMarker) []tracePoint {
	var points []tracePoint
	for _, m := range markers {
		if m.TimeMs <= 0 || m.Turn == "" {
			continue
		}
		at := -1
		var best int64
		for i, r := range rows {
			if r.Turn != m.Turn || r.TS <= 0 {
				continue
			}
			d := max(m.TimeMs, r.TS) - min(m.TimeMs, r.TS)
			if at < 0 || d < best {
				at, best = i, d
			}
		}
		if at >= 0 {
			points = append(points, tracePoint{m, at})
		}
	}
	return points
}

// traceTimeline lays the listed attempts and their transcript events out
// in lanes, equally spaced or on their actual clock; selected marks the
// attempt the detail shows. A lane without anything to draw is left out.
func (s *State) traceTimeline(c *ui.Context, rows []traceRow, selected *traceRow) {
	if len(rows) == 0 {
		return
	}
	v := &s.trace
	p := theme.Of(c)
	points := tracePoints(rows, s.TraceMarkers)
	var lo, hi int64
	for _, r := range rows {
		if r.TS <= 0 {
			continue
		}
		if lo == 0 || r.TS < lo {
			lo = r.TS
		}
		hi = max(hi, r.TS+int64(r.Timing["elapsed_us"]/1000))
	}
	for _, m := range points {
		lo = min(lo, m.TimeMs)
		hi = max(hi, m.TimeMs)
	}
	labels := []string{"meter_user", "teams_model", "meter_tools_time", "chat_compact"}
	shown := []bool{false, true, false, false}
	for _, m := range points {
		shown[0] = shown[0] || !m.Tool
		shown[2] = shown[2] || m.Tool
	}
	for _, r := range rows {
		shown[3] = shown[3] || r.Purpose == "compaction"
	}
	const lane, gap = 14, 4
	lanes := 0
	for _, on := range shown {
		if on {
			lanes++
		}
	}
	// Equal width keeps an attempt's slot narrow, so a few attempts read
	// as a sequence rather than a span.
	width := max(float32(240), float32(len(rows))*9)
	slot := min(width/float32(len(rows)), 16)
	xAt := func(i int, ms int64) float32 {
		if v.real && hi > lo {
			return width * float32(ms-lo) / float32(hi-lo)
		}
		return float32(i) * slot
	}
	choose := func(r traceRow) {
		v.request, v.attempt, v.detail = r.RequestID, r.Attempt, true
		v.reveal = true
		v.preview = tracePreview{}
	}
	scale := 0
	if v.real {
		scale = 1
	}
	ui.Row(c.Key("timeline")).FillWidth().Padding(8, 12).Gap(8).AlignItems(ui.Center).Shrink(0).Children(func() {
		ui.Column(c.Key("labels")).Width(56).Shrink(0).Gap(gap).Children(func() {
			for i, label := range labels {
				if shown[i] {
					ui.Text(c.Key(i), i18n.T(c, label)).Height(lane).FontSize(11).TextColor(p.MutedForeground).SingleLine()
				}
			}
		})
		height := float32(lanes*lane + (lanes-1)*gap)
		ui.ScrollHorizontal(c.Key("lanes")).Grow(1).MinWidth(0).Height(height + 6).Children(func() {
			ui.Column(c.Key("tracks")).Width(width).Gap(gap).Children(func() {
				for kind, label := range labels {
					if !shown[kind] {
						continue
					}
					ui.Row(c.Key(kind)).Width(width).Height(lane).Radius(3).Background(p.Muted.Alpha(0.6)).Children(func() {
						if kind == 0 || kind == 2 {
							for i, m := range points {
								if m.Tool != (kind == 2) {
									continue
								}
								r := rows[m.row]
								x := xAt(m.row, m.TimeMs)
								tip := fmt.Sprintf("%s %s (#%d)", i18n.T(c, label), time.UnixMilli(m.TimeMs).Local().Format("15:04:05.000"), r.Attempt)
								dot := ui.ButtonBase(c.Key(i)).Label(tip).Absolute().Left(min(x, width-6)).Top(4).Size(6, 6).Radius(3).Background(p.Chart[kind])
								kit.Tooltip(c, dot, tip)
								dot.OnClick(func() { choose(r) })
							}
							return
						}
						for i, r := range rows {
							if v.real && r.TS <= 0 {
								continue
							}
							if (kind == 3) != (r.Purpose == "compaction") {
								continue
							}
							x, w := xAt(i, r.TS), slot
							if v.real && hi > lo {
								w = width * float32(r.Timing["elapsed_us"]) / 1000 / float32(hi-lo)
							}
							tip := fmt.Sprintf("#%d %s %s", r.Attempt, r.Provider.Model, r.elapsed())
							col := p.Primary
							if kind == 3 {
								col = p.Chart[3]
							}
							switch r.tone() {
							case kit.Negative:
								col = p.Danger
							case kit.Caution:
								col = p.Warning
							}
							bar := ui.ButtonBase(c.Key(i)).Label(tip).Absolute().Left(min(x, width-2)).Top(2).Width(max(float32(2), w-1)).Height(lane - 4).Radius(2).Background(col)
							if selected != nil && selected.RequestID == r.RequestID && selected.Attempt == r.Attempt {
								bar.InsetShadow(0, 0, 0, 2, p.Foreground)
							} else if selected != nil {
								bar.Opacity(0.65)
							}
							kit.Tooltip(c, bar, tip)
							bar.OnClick(func() { choose(r) })
						}
					})
				}
			})
		})
		kit.Segmented(c, "scale", &scale, []string{i18n.T(c, "trace_equal"), i18n.T(c, "trace_actual")}, kit.SegmentedProps{Size: kit.SizeSmall})
	})
	v.real = scale == 1
}
