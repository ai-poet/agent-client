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

func (s *State) traceTimeline(c *ui.Context, rows []traceRow) {
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
	key := "trace_equal"
	if v.real {
		key = "trace_actual"
	}
	labels := []string{"meter_input", "teams_model", "meter_tools_time", "meter_compacting"}
	width := max(float32(240), float32(len(rows))*9)
	slot := width / float32(len(rows))
	xAt := func(i int, ms int64) float32 {
		if v.real && hi > lo {
			return width * float32(ms-lo) / float32(hi-lo)
		}
		return float32(i) * slot
	}
	choose := func(r traceRow) {
		v.request, v.attempt, v.detail, v.tab = r.RequestID, r.Attempt, true, ""
		v.reveal = true
		v.preview = tracePreview{}
	}
	ui.Row(c.Key("ruler")).Gap(6).AlignItems(ui.Center).Children(func() {
		kit.Ghost(c.Key("scale"), i18n.T(c, key), v.real).OnClick(func() { v.real = !v.real })
		ui.Column(c.Key("labels")).Children(func() {
			for i, label := range labels {
				ui.Text(c.Key(i), i18n.T(c, label)).Height(16).FontSize(11).TextColor(p.MutedForeground).SingleLine()
			}
		})
		ui.ScrollHorizontal(c.Key("lanes")).Grow(1).MinWidth(0).Height(68).Children(func() {
			ui.Column(c.Key("tracks")).Width(width).Children(func() {
				for lane, label := range labels {
					ui.Row(c.Key(lane)).Width(width).Height(16).Children(func() {
						if lane == 0 || lane == 2 {
							for i, m := range points {
								if m.Tool != (lane == 2) {
									continue
								}
								r := rows[m.row]
								x := xAt(m.row, m.TimeMs)
								tip := fmt.Sprintf("%s %s (#%d)", i18n.T(c, label), time.UnixMilli(m.TimeMs).Local().Format("15:04:05.000 MST"), r.Attempt)
								dot := ui.ButtonBase(c.Key(i)).Label(tip).Absolute().Left(min(x, width-6)).Top(5).Size(6, 6).Radius(3).Background(p.Chart[lane])
								kit.Tooltip(c, dot, tip)
								dot.OnClick(func() { choose(r) })
							}
							return
						}
						for i, r := range rows {
							if v.real && r.TS <= 0 {
								continue
							}
							if (lane == 3) != (r.Purpose == "compaction") {
								continue
							}
							x, w := xAt(i, r.TS), slot
							if v.real && hi > lo {
								w = width * float32(r.Timing["elapsed_us"]) / 1000 / float32(hi-lo)
							}
							tip := fmt.Sprintf("%s #%d %s", i18n.T(c, label), r.Attempt, r.Provider.Model)
							col := p.Primary
							if lane == 3 {
								col = p.Warning
							}
							if r.Outcome == "failed" {
								col = p.Danger
							}
							bar := ui.ButtonBase(c.Key(i)).Label(tip).Absolute().Left(min(x, width-2)).Top(3).Width(max(float32(2), w-1)).Height(10).Background(col)
							kit.Tooltip(c, bar, tip)
							bar.OnClick(func() { choose(r) })
						}
					})
				}
			})
		})
	})
}
