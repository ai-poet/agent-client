package panel

import (
	"bytes"
	"encoding/json"
	"fmt"
	"maps"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type traceSide struct {
	Body        string
	Headers     map[string][]string
	End         string
	Gateway     string `json:"gateway_request_id"`
	ContentType string `json:"content_type"`
}

type traceRow struct {
	RequestID                      string `json:"request_id"`
	Attempt                        int
	TS                             int64
	Turn, Record, Purpose, Outcome string
	Status                         int
	Provider                       struct{ Model, ID string }
	Failure                        struct{ Kind, Code string }
	Retry                          *struct{ Index int }
	Usage                          map[string]json.RawMessage
	Timing                         map[string]uint64
	Request, Response              traceSide
	Bytes                          struct {
		ReqTruncated bool `json:"req_truncated"`
		ResTruncated bool `json:"res_truncated"`
	}
}

type tracePresentation struct {
	request, filter, query, tab                  string
	attempt                                      int
	detail, real, split, export, confirm, reveal bool
	scope, mode, exportTurn                      string
	page                                         []json.RawMessage
	projected                                    []traceRow
	appliedFilter, appliedQuery                  string
	preview                                      tracePreview
}

func (s *State) ResetTrace() {
	s.TraceReadBody, s.TraceJump, s.TraceExport = nil, nil, nil
	s.TracePage = nil
	s.TraceContent = protocol.TracePage{}
	s.TraceError = ""
	s.trace = tracePresentation{}
}

func (s *State) SelectTrace(request string, attempt int) {
	s.trace.request, s.trace.attempt = request, attempt
	s.trace.query, s.trace.filter, s.trace.tab = "", "", ""
	s.trace.detail = true
	s.trace.reveal = true
	s.trace.preview = tracePreview{}
}

func traceRows(page []json.RawMessage, filter, query string) []traceRow {
	var out []traceRow
	groups := map[string][]traceRow{}
	var turns []string
	query = strings.ToLower(strings.TrimSpace(query))
	for _, raw := range page {
		var r traceRow
		if json.Unmarshal(raw, &r) != nil || r.RequestID == "" {
			continue
		}
		if filter == "failed" && r.Outcome != "failed" || filter == "retry" && r.Attempt <= 1 && r.Retry == nil || filter == "miss" && string(r.Usage["cache_read"]) != "0" {
			continue
		}
		if query != "" && !strings.Contains(strings.ToLower(strings.Join([]string{r.status(), r.Failure.Kind, r.Failure.Code, r.Provider.Model, r.Response.Gateway}, " ")), query) {
			continue
		}
		if _, ok := groups[r.Turn]; !ok {
			turns = append(turns, r.Turn)
		}
		groups[r.Turn] = append(groups[r.Turn], r)
	}
	for _, turn := range turns {
		out = append(out, groups[turn]...)
	}
	return out
}
func (r traceRow) status() string {
	if r.Status == 0 {
		return "-"
	}
	return strconv.Itoa(r.Status)
}
func (r traceRow) metric(k string) string {
	if v, ok := r.Usage[k]; ok {
		return string(v)
	}
	return "-"
}
func (r traceRow) elapsed() string {
	if v, ok := r.Timing["elapsed_us"]; ok {
		return (time.Duration(v) * time.Microsecond).String()
	}
	return "-"
}
func (r traceRow) badge(c *ui.Context) {
	key, tone := "unavailable", kit.Neutral
	switch r.Outcome {
	case "failed":
		key, tone = "meter_failed", kit.Negative
	case "succeeded":
		key, tone = "meter_succeeded", kit.Positive
	case "cancelled":
		key = "background_outcome_cancelled"
	}
	kit.Badge(c, i18n.T(c, key), tone)
}

func (s *State) traceView(c *ui.Context) {
	v := &s.trace
	box := ui.Column(c.Key("trace")).Grow(1).MinHeight(0)
	narrow := box.LayoutBounds().W < 480
	box.Children(func() {
		ui.Row(c.Key("actions")).Gap(4).PaddingX(6).Children(func() {
			if narrow && v.detail {
				kit.IconButton(c.Key("back"), icons.ArrowLeft, i18n.T(c, "provider_step_back"), 28, false).OnClick(func() { v.detail = false })
			}
			ui.Text(c, i18n.T(c, "trace_title")).Grow(1).FontWeight(600)
			kit.Ghost(c.Key("export"), i18n.T(c, "trace_export"), false).Disabled(s.TraceExport == nil).OnClick(func() { v.scope, v.mode, v.export = "all", "shareable", true })
		})
		ui.Text(c, i18n.T(c, "trace_private")).FontSize(12).TextColor(theme.Of(c).MutedForeground).PaddingX(6)
		if s.TraceError != "" {
			ui.Text(c, s.TraceError).TextColor(theme.Of(c).Danger)
		}
		// Read results are immutable. Repeated frames and timeline hover do not
		// decode headers or provider metadata again unless the index changes.
		if v.appliedFilter != v.filter || v.appliedQuery != v.query || !slices.EqualFunc(v.page, s.TracePage, func(a, b json.RawMessage) bool { return bytes.Equal(a, b) }) {
			v.page = slices.Clone(s.TracePage)
			v.appliedQuery, v.appliedFilter = v.query, v.filter
			v.projected = traceRows(v.page, v.filter, v.query)
		}
		rows := v.projected
		s.traceTimeline(c, rows)
		var selected *traceRow
		for i := range rows {
			if rows[i].RequestID == v.request && rows[i].Attempt == v.attempt {
				selected = &rows[i]
				break
			}
		}
		if selected == nil && len(rows) > 0 && (!v.detail || v.request == "") {
			selected = &rows[0]
		}
		ui.Row(c.Key("columns")).Grow(1).MinHeight(0).Children(func() {
			if !narrow || !v.detail {
				list := ui.Column(c.Key("list")).MinHeight(0)
				if narrow {
					list.Grow(1)
				} else {
					list.Width(200).Shrink(0)
				}
				list.Children(func() {
					ui.TextInput(c.Key("search"), &v.query).Label(i18n.T(c, "search")).Placeholder(i18n.T(c, "search")).FillWidth()
					ui.ScrollHorizontal(c.Key("filters")).Height(30).Scrollbars(false).Children(func() {
						ui.Row(c).Gap(3).Children(func() {
							for i, id := range []string{"", "failed", "retry", "miss"} {
								key := []string{"market_all", "meter_failed", "trace_retries", "trace_miss"}[i]
								kit.Ghost(c.Key(i), i18n.T(c, key), v.filter == id).PaddingX(5).OnClick(func() { v.filter = id; v.detail = false })
							}
						})
					})
					ui.Scroll(c.Key("attempts")).Grow(1).MinHeight(0).Children(func() {
						prev := ""
						for i, r := range rows {
							if i == 0 || prev != r.Turn {
								ui.Text(c.Key(i).Key("turn"), r.Turn).FontSize(12).SingleLine()
								prev = r.Turn
							}
							active := selected != nil && selected.RequestID == r.RequestID && selected.Attempt == r.Attempt
							row := kit.GhostLook(c, ui.ButtonBase(c.Key(r.RequestID).Key(r.Attempt)).Label(fmt.Sprintf("#%d %s %s", r.Attempt, r.Provider.Model, r.Outcome)), active)
							if active && v.reveal {
								row.ScrollIntoView()
								v.reveal = false
							}
							row.FillWidth().Children(func() {
								ui.Column(c).FillWidth().Padding(4).Children(func() {
									ui.Row(c).Gap(4).Children(func() { ui.Text(c, fmt.Sprintf("#%d  %s", r.Attempt, r.status())); r.badge(c) })
									ui.Text(c, r.elapsed()+"   "+r.metric("input")+"/"+r.metric("output")).FontSize(12)
									ui.Text(c, r.Provider.Model).FontSize(12).SingleLine()
								})
							}).OnClick(func() {
								v.request, v.attempt, v.detail, v.tab = r.RequestID, r.Attempt, true, ""
								v.preview = tracePreview{}
							})
						}
						if len(rows) == 0 {
							ui.Text(c, i18n.T(c, "file_search_empty"))
						}
					})
				})
			}
			if !narrow || v.detail {
				ui.Column(c.Key("detail")).Grow(1).MinWidth(0).MinHeight(0).Children(func() {
					if selected == nil {
						ui.Text(c, i18n.T(c, "unavailable"))
						return
					}
					s.traceDetail(c, *selected)
				})
			}
		})
		turn := ""
		if selected != nil {
			turn = selected.Turn
		}
		s.traceExportForm(c, turn)
	})
}

func (s *State) traceDetail(c *ui.Context, r traceRow) {
	v := &s.trace
	ui.ScrollHorizontal(c.Key("tabs")).Height(30).Scrollbars(false).Children(func() {
		ui.Row(c).Children(func() {
			for i, id := range []string{"", "request", "response", "metrics"} {
				key := []string{"market_details_overview", "trace_request", "trace_response", "trace_metrics"}[i]
				kit.Ghost(c.Key(i), i18n.T(c, key), v.tab == id).PaddingX(6).OnClick(func() {
					v.tab = id
					v.preview = tracePreview{}
					var ref string
					if id == "request" {
						ref = r.Request.Body
					} else if id == "response" {
						ref = r.Response.Body
					}
					if ref != "" && ref != s.TraceContent.Body && s.TraceReadBody != nil {
						s.TraceReadBody(ref)
					}
				})
			}
		})
	})
	ui.Scroll(c.Key(r.RequestID).Key(r.Attempt).Key(v.tab)).Grow(1).MinHeight(0).Padding(8).Children(func() {
		switch v.tab {
		case "request", "response":
			s.traceBody(c, r)
		case "metrics":
			for i, key := range []string{"input", "output", "cache_read", "cache_write", "reasoning"} {
				label := []string{"meter_input", "meter_output", "meter_cache_read", "trace_cache_write", "meter_reasoning"}[i]
				ui.Text(c, i18n.T(c, label)+"   "+r.metric(key))
			}
			if len(r.Usage) > 0 {
				raw, _ := json.Marshal(r.Usage)
				ui.Text(c, string(raw)).FontSize(12)
			}
			traceTiming(c, r)
		default:
			r.badge(c)
			if r.TS != 0 {
				ui.Text(c, time.UnixMilli(r.TS).Local().Format("2006-01-02 15:04:05.000 MST -07:00")).FontSize(12)
			}
			ui.Text(c, r.Provider.Model+"   "+r.status())
			if r.Failure.Kind != "" {
				ui.Text(c, r.Failure.Kind+" "+r.Failure.Code)
			}
			if r.Response.Gateway != "" {
				ui.Text(c, r.Response.Gateway)
				kit.Ghost(c.Key("gateway"), i18n.T(c, "connections_copy_id"), false).OnClick(func() { c.WriteClipboard(r.Response.Gateway) })
			}
			ui.Text(c, i18n.T(c, "meter_input")+" "+r.metric("input")+"   "+i18n.T(c, "meter_output")+" "+r.metric("output"))
			for _, pair := range [][3]string{{"cache_read", "input", "meter_cache_read"}, {"reasoning", "output", "meter_reasoning"}} {
				n, e1 := strconv.ParseUint(r.metric(pair[0]), 10, 64)
				total, e2 := strconv.ParseUint(r.metric(pair[1]), 10, 64)
				if e1 == nil && e2 == nil && total > 0 && n <= total {
					ui.Text(c, i18n.T(c, pair[2])+fmt.Sprintf("   %.1f%%", 100*float64(n)/float64(total))).FontSize(12)
					ui.Box(c).Height(4).Width(140 * float32(n) / float32(total)).Background(theme.Of(c).Primary)
				}
			}
			traceTiming(c, r)
			kit.Secondary(c.Key("jump"), i18n.T(c, "trace_locate")).Disabled(s.TraceJump == nil || (r.Record == "" && r.Turn == "")).OnClick(func() { s.TraceJump(r.Record, r.Turn) })
		}
	})
}
func traceTiming(c *ui.Context, r traceRow) {
	p := theme.Of(c)
	for i, key := range []string{"dispatch_us", "sent_us", "headers_us", "first_token_us", "elapsed_us", "max_gap_us"} {
		if t, ok := r.Timing[key]; ok {
			label := []string{"trace_dispatch", "trace_sent", "mcp_headers", "meter_first_response", "meter_request_time", "trace_gap"}[i]
			ui.Text(c, i18n.T(c, label)+"   "+(time.Duration(t)*time.Microsecond).String()).FontSize(12)
			if total := r.Timing["elapsed_us"]; total > 0 {
				ui.Box(c.Key(key)).Height(3).Width(140 * float32(min(t, total)) / float32(total)).Background(p.Primary)
			}
		}
	}
}

func (s *State) traceTimeline(c *ui.Context, rows []traceRow) {
	if len(rows) == 0 {
		return
	}
	v := &s.trace
	// The index records model calls (including compaction), not tool or input spans.
	lo, hi := rows[0].TS, rows[0].TS
	for _, r := range rows {
		lo = min(lo, r.TS)
		hi = max(hi, r.TS+int64(r.Timing["elapsed_us"]/1000))
	}
	ui.Row(c.Key("ruler")).Gap(6).AlignItems(ui.Center).Children(func() {
		key := "trace_equal"
		if v.real {
			key = "trace_actual"
		}
		kit.Ghost(c.Key("scale"), i18n.T(c, key), v.real).OnClick(func() { v.real = !v.real })
		ui.ScrollHorizontal(c.Key("lanes")).Grow(1).Height(35).Children(func() {
			width := max(float32(240), float32(len(rows))*9)
			ui.Column(c.Key("tracks")).Width(width).Children(func() {
				for i, label := range []string{"teams_model", "meter_compacting"} {
					ui.Row(c.Key(i)).Width(width).Height(17).Children(func() {
						for n, r := range rows {
							if (r.Purpose == "compaction") != (i == 1) {
								continue
							}
							x, w := width*float32(n)/float32(len(rows)), width/float32(len(rows))
							if v.real && hi > lo {
								x = width * float32(r.TS-lo) / float32(hi-lo)
								w = width * float32(r.Timing["elapsed_us"]) / 1000 / float32(hi-lo)
							}
							tip := fmt.Sprintf("%s #%d %s", i18n.T(c, label), r.Attempt, r.Provider.Model)
							col := theme.Of(c).Primary
							if i == 1 {
								col = theme.Of(c).Warning
							}
							if r.Outcome == "failed" {
								col = theme.Of(c).Danger
							}
							bar := ui.ButtonBase(c.Key(n)).Label(tip).Absolute().Left(x).Top(3).Width(max(float32(2), w-1)).Height(10).Background(col)
							kit.Tooltip(c, bar, tip)
							bar.OnClick(func() {
								v.request, v.attempt, v.detail, v.tab = r.RequestID, r.Attempt, true, ""
								v.preview = tracePreview{}
							})
						}
					})
				}
			})
		})
	})
}
func (s *State) traceExportForm(c *ui.Context, turn string) {
	v := &s.trace
	title := i18n.T(c, "trace_export")
	if v.export {
		kit.Dialog(c.Key("trace-export"), &v.export, func() {
			ui.Text(c, title).FontWeight(600)
			kit.Select(c.Key("scope"), "trace-scope", i18n.T(c, "trace_scope"), &v.scope, []string{i18n.T(c, "market_all"), i18n.T(c, "meter_failed"), i18n.T(c, "trace_turn")}, false, []string{"all", "failed", "turn"})
			kit.Select(c.Key("mode"), "trace-mode", i18n.T(c, "trace_mode"), &v.mode, []string{i18n.T(c, "trace_shareable"), i18n.T(c, "trace_raw")}, false, []string{"shareable", "raw"})
			kit.Primary(c.Key("export-go"), title, s.TraceExport == nil || v.scope == "turn" && turn == "").OnClick(func() {
				v.export = false
				v.exportTurn = ""
				if v.scope == "turn" {
					v.exportTurn = turn
				}
				if v.mode == "raw" {
					v.confirm = true
					return
				}
				s.TraceExport(v.scope, v.mode, v.exportTurn)
			})
		}).Width(360).Label(title)
	}
	if v.confirm {
		if kit.Confirm(c.Key("trace-raw"), &v.confirm, kit.Confirmation{Title: title, Message: i18n.T(c, "trace_raw_confirm"), Buttons: []string{i18n.T(c, "settings_cancel"), title}, Default: 0}) == 1 && s.TraceExport != nil {
			v.confirm = false
			s.TraceExport(v.scope, "raw", v.exportTurn)
		}
	}
}

func traceHeaders(c *ui.Context, h map[string][]string) {
	for _, name := range slices.Sorted(maps.Keys(h)) {
		ui.Text(c, name+": "+strings.Join(h[name], ", ")).FontSize(12)
	}
}
