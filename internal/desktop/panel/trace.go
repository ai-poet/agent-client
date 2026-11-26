package panel

import (
	"bytes"
	"cmp"
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
	Dropped     []string `json:"dropped_headers"`
	End         string
	Gateway     string `json:"gateway_request_id"`
	ContentType string `json:"content_type"`
}

type traceRow struct {
	Verdict                        *protocol.TraceVerdict
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
	Categories                     []protocol.ConversationObservationCategory
	CacheReported                  *bool `json:"cache_reported"`
	observedCache                  *uint64
	CompleteContext                bool `json:"complete_context"`
	turnNumber                     int
	// captured is a row of the session's private index, as opposed to an
	// observation or Insights fallback without sidecars.
	captured          bool
	Request, Response traceSide
	Bytes             struct {
		ReqTruncated bool `json:"req_truncated"`
		ResTruncated bool `json:"res_truncated"`
	}
}

type tracePresentation struct {
	// indexed is a metadata-only fallback for the deep-linked attempt,
	// not a captured record or part of the conversation watch.
	indexed                                               *traceRow
	request, filter, query, tab                           string
	attempt, mode                                         int
	detail, real, split, export, confirm, reveal, headers bool
	scope, exportTurn                                     string
	page                                                  []json.RawMessage
	projected                                             []traceRow
	// requested is the body last asked for, so a frame never repeats a
	// read that is in flight or failed.
	requested string

	preview tracePreview
}

// The detail tabs, in order: their ids and labels.
var traceTabs = []struct{ id, label string }{
	{"", "market_details_overview"}, {"request", "trace_request"}, {"response", "trace_response"}, {"metrics", "trace_metrics"},
}

// The list filters, in order: their ids and labels.
var traceFilters = []struct{ id, label string }{
	{"", "market_all"}, {"failed", "meter_failed"}, {"retry", "trace_retries"}, {"miss", "trace_miss"},
}

// traceNarrow is the width below which the list and the detail take turns.
const traceNarrow = 520

func (s *State) ResetTrace() {
	s.TraceReadBody, s.TraceJump, s.TraceExport = nil, nil, nil
	s.TracePage, s.TraceRecent, s.TraceTurnNumbers, s.TraceMarkers = nil, nil, nil, nil
	s.TraceContent = protocol.TracePage{}
	s.TraceError, s.TraceBodyFailed = "", false
	s.trace = tracePresentation{}
}

func (s *State) SelectTrace(request string, attempt int) {
	s.trace.indexed = nil
	s.trace.request, s.trace.attempt = request, attempt
	s.trace.query, s.trace.filter, s.trace.tab = "", "", ""
	s.trace.detail = true
	s.trace.reveal = true
	s.trace.preview = tracePreview{}
}

func decodeTraceRows(page []json.RawMessage) []traceRow {
	var out []traceRow

	for _, raw := range page {
		var r traceRow
		if json.Unmarshal(raw, &r) != nil || r.RequestID == "" {
			continue
		}
		r.captured = true
		out = append(out, r)
	}
	return out
}

func (s *State) traceRows(decoded []traceRow, filter, query string) []traceRow {
	requests := make(map[string]*protocol.ConversationObservationRequest, len(s.TraceRecent))
	numbers := maps.Clone(s.TraceTurnNumbers)
	if numbers == nil {
		numbers = map[string]int{}
	}
	for i := range s.TraceRecent {
		req := &s.TraceRecent[i]
		requests[req.ID] = req
		if req.TurnIndex != nil && *req.TurnIndex > 0 && numbers[req.Turn.String()] <= 0 {
			numbers[req.Turn.String()] = int(*req.TurnIndex)
		}
	}
	groups := map[string][]traceRow{}
	var turns []string
	query = strings.ToLower(strings.TrimSpace(query))
	decoded = withObservedDiagnoses(decoded, s.TraceRecent)
	if seed := s.trace.indexed; seed != nil && !slices.ContainsFunc(decoded, func(r traceRow) bool {
		return r.Turn == seed.Turn && r.RequestID == seed.RequestID && r.Attempt == seed.Attempt
	}) {
		decoded = append(slices.Clip(decoded), *seed)
	}
	for _, r := range decoded {
		r.turnNumber = numbers[r.Turn]
		if req := requests[r.RequestID]; req != nil && req.Turn.String() == r.Turn {
			for i := range req.Attempts {
				attempt := &req.Attempts[i]
				if attempt.Index != uint64(r.Attempt) {
					continue
				}
				if len(r.Categories) == 0 {
					r.Categories = attempt.Categories
				}
				if r.Verdict == nil {
					r.Verdict = attempt.Verdict
				}
				r.CompleteContext = attempt.CompleteContext
				if r.CacheReported == nil {
					r.CacheReported = &attempt.CacheReported
				}
				if attempt.CacheReported && attempt.Usage != nil {
					r.observedCache = &attempt.Usage.CachedInput
				}
				break
			}
		}
		if !r.matches(filter) {
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
	// Captures precede the observation fallback in decoded, so a session
	// recorded only after its first turns would list those turns last.
	// Order turns, and each turn's requests, by their earliest start.
	first := func(rows []traceRow, request string) int64 {
		var at int64
		for _, r := range rows {
			if r.TS > 0 && (request == "" || r.RequestID == request) && (at == 0 || r.TS < at) {
				at = r.TS
			}
		}
		if at == 0 {
			return 1<<63 - 1
		}
		return at
	}
	slices.SortStableFunc(turns, func(a, b string) int { return cmp.Compare(first(groups[a], ""), first(groups[b], "")) })
	var out []traceRow
	for _, turn := range turns {
		rows := groups[turn]
		slices.SortStableFunc(rows, func(a, b traceRow) int {
			if a.RequestID == b.RequestID {
				return cmp.Compare(a.Attempt, b.Attempt)
			}
			return cmp.Compare(first(rows, a.RequestID), first(rows, b.RequestID))
		})
		out = append(out, rows...)
	}
	return out
}

func (r traceRow) matches(filter string) bool {
	switch filter {
	case "failed":
		return r.Outcome == "failed"
	case "retry":
		return r.Attempt > 1 || r.Retry != nil
	case "miss":
		return r.metric("cache_read") == "0"
	}
	return true
}

func (r traceRow) status() string {
	if r.Status == 0 {
		return "-"
	}
	return strconv.Itoa(r.Status)
}
func (r traceRow) metric(k string) string {
	if k == "cache_read" {
		if r.CacheReported != nil && !*r.CacheReported {
			return "-"
		}
		if r.observedCache != nil {
			return strconv.FormatUint(*r.observedCache, 10)
		}
	}
	if v, ok := r.Usage[k]; ok && string(v) != "null" {
		return string(v)
	}
	return "-"
}
func (r traceRow) elapsed() string {
	if v, ok := r.Timing["elapsed_us"]; ok {
		return traceDuration(v)
	}
	return "-"
}

// traceDuration is a microsecond span at the precision a reader compares:
// whole milliseconds under a second, then seconds to two places.
func traceDuration(us uint64) string {
	d := time.Duration(us) * time.Microsecond
	switch {
	case d < time.Millisecond:
		return fmt.Sprintf("%.1fms", float64(d)/float64(time.Millisecond))
	case d < time.Second:
		return fmt.Sprintf("%dms", d.Milliseconds())
	case d < time.Minute:
		return fmt.Sprintf("%.2fs", d.Seconds())
	}
	return d.Round(time.Second).String()
}

// tone is the attempt's state at a glance: its outcome, unless a warning
// qualifies a success.
func (r traceRow) tone() kit.Tone {
	switch {
	case r.Outcome == "failed":
		return kit.Negative
	case r.Verdict != nil && r.Verdict.Severity == "warning":
		return kit.Caution
	case r.Outcome == "succeeded":
		return kit.Positive
	}
	return kit.Neutral
}

func toneColor(p *theme.Palette, t kit.Tone) ui.Color {
	switch t {
	case kit.Positive:
		return p.Success
	case kit.Caution:
		return p.Warning
	case kit.Negative:
		return p.Danger
	}
	return p.MutedForeground
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

// finding is the attempt's diagnosis unless it found nothing.
func (r traceRow) finding() *protocol.TraceVerdict {
	if r.Verdict == nil || r.Verdict.Code == "normal" || r.Verdict.Summary == "" {
		return nil
	}
	return r.Verdict
}

func (r traceRow) turnLabel(c *ui.Context) string {
	if r.turnNumber > 0 {
		return i18n.T(c, "trace_turn_number", "count", r.turnNumber)
	}
	id := r.Turn
	if len(id) > 8 {
		id = id[:8]
	}
	return i18n.T(c, "trace_turn_label") + " " + id
}

func (s *State) traceView(c *ui.Context) {
	v := &s.trace
	p := theme.Of(c)
	box := ui.Column(c.Key("trace")).Grow(1).MinHeight(0)
	narrow := box.LayoutBounds().W < traceNarrow
	box.Children(func() {
		// Read results are immutable. Repeated frames and timeline hover do not
		// decode headers or provider metadata again unless the index changes.
		if !slices.EqualFunc(v.page, s.TracePage, func(a, b json.RawMessage) bool { return bytes.Equal(a, b) }) {
			v.page = slices.Clone(s.TracePage)
			v.projected = decodeTraceRows(v.page)
			// A changed index may have dropped the body being read.
			v.requested = ""
		}
		all := s.traceRows(v.projected, "", v.query)
		rows := slices.DeleteFunc(slices.Clone(all), func(r traceRow) bool { return !r.matches(v.filter) })
		var selected *traceRow
		for i := range rows {
			if rows[i].RequestID == v.request && rows[i].Attempt == v.attempt {
				selected = &rows[i]
				break
			}
		}
		if selected == nil && len(rows) > 0 && (!v.detail || v.request == "") {
			// The latest attempt is the one a reader opens the trace for.
			selected = &rows[len(rows)-1]
			v.request, v.attempt, v.reveal = selected.RequestID, selected.Attempt, true
			v.preview = tracePreview{}
		}
		toolbar(c, "toolbar", func() {
			if narrow && v.detail {
				kit.Action(c.Key("back"), icons.ArrowLeft, i18n.T(c, "composer_back"), false).OnClick(func() { v.detail = false })
			}
			search := i18n.T(c, "search")
			ui.TextInput(c.Key("search"), &v.query).Label(search).Placeholder(search).Grow(1).MinWidth(0).Height(28)
			kit.TextAction(c.Key("export"), i18n.T(c, "trace_export"), s.TraceExport == nil || len(v.projected) == 0).OnClick(func() {
				v.scope, v.mode, v.export = "all", 0, true
			})
		})
		if s.TraceError != "" {
			ui.Column(c.Key("error")).Padding(8, 8, 0, 8).Children(func() { kit.Alert(c, "alert", kit.Negative, s.TraceError, nil) })
		}
		if len(all) == 0 && v.query == "" {
			kit.Result(c, kit.ResultProps{Icon: icons.ListTree, Title: i18n.T(c, "trace_empty"), SubTitle: i18n.T(c, "trace_empty_hint")}).Grow(1).Justify(ui.Center)
			return
		}
		s.traceTimeline(c, rows, selected)
		ui.Row(c.Key("columns")).Grow(1).MinHeight(0).AlignItems(ui.Stretch).Clip().BorderWidth(1, 0, 0, 0).BorderColor(p.Border).Children(func() {
			if !narrow || !v.detail {
				list := ui.Column(c.Key("list")).MinHeight(0)
				if narrow {
					list.Grow(1)
				} else {
					list.Width(232).Shrink(0).BorderWidth(0, 1, 0, 0).BorderColor(p.Border)
				}
				list.Children(func() { s.traceList(c, all, rows, selected) })
			}
			if !narrow || v.detail {
				ui.Column(c.Key("detail")).Grow(1).MinWidth(0).MinHeight(0).Children(func() {
					if selected == nil {
						empty(c, icons.ListTree, "unavailable")
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
		s.traceExportForm(c, turn, selected)
	})
}

func (s *State) traceList(c *ui.Context, all, rows []traceRow, selected *traceRow) {
	v := &s.trace
	p := theme.Of(c)
	ui.Row(c.Key("filters")).Wrap().Gap(4).Padding(8, 8, 4, 8).Children(func() {
		for i, f := range traceFilters {
			label := i18n.T(c, f.label)
			if f.id != "" {
				n := 0
				for _, r := range all {
					if r.matches(f.id) {
						n++
					}
				}
				// A filter that matches nothing is not offered, unless chosen.
				if n == 0 && v.filter != f.id {
					continue
				}
				label += " " + strconv.Itoa(n)
			}
			kit.GhostLook(c, ui.ButtonBase(c.Key(i)).Label(label), v.filter == f.id).Height(24).PaddingX(8).Children(func() {
				ui.Text(c, label).FontSize(12).SingleLine()
			}).OnClick(func() { v.filter = f.id; v.detail = false })
		}
	})
	ui.Scroll(c.Key("attempts")).Grow(1).MinHeight(0).Children(func() {
		ui.Column(c).FillWidth().Padding(0, 4, 8, 4).Children(func() {
			prev := ""
			for i, r := range rows {
				if i == 0 || prev != r.Turn {
					ui.Text(c.Key("turn:"+r.Turn), r.turnLabel(c)).FontSize(12).FontWeight(500).TextColor(p.MutedForeground).
						SingleLine().Padding(10, 6, 4, 6)
					prev = r.Turn
				}
				active := selected != nil && selected.RequestID == r.RequestID && selected.Attempt == r.Attempt
				s.traceItem(c, r, active)
			}
			if len(rows) == 0 {
				ui.Text(c, i18n.T(c, "file_search_empty")).FontSize(13).TextColor(p.MutedForeground).Padding(12, 6)
			}
		})
	})
}

func (s *State) traceItem(c *ui.Context, r traceRow, active bool) {
	v := &s.trace
	p := theme.Of(c)
	muted := p.MutedForeground
	label := fmt.Sprintf("#%d %s %s", r.Attempt, r.Provider.Model, r.Outcome)
	row := kit.GhostLook(c, ui.ButtonBase(c.Key(fmt.Sprintf("attempt:%s:%d", r.RequestID, r.Attempt))).Label(label), active)
	if active && v.reveal {
		row.ScrollIntoView()
		v.reveal = false
	}
	row.FillWidth().Children(func() {
		ui.Column(c).FillWidth().MinWidth(0).Padding(6, 8).Gap(2).Children(func() {
			ui.Row(c).FillWidth().Gap(6).AlignItems(ui.Center).Children(func() {
				kit.StatusDot(c, r.tone(), "")
				ui.Text(c, "#"+strconv.Itoa(r.Attempt)).FontSize(13).FontWeight(500).FontFeatures("tnum")
				if r.Status != 0 {
					ui.Text(c, strconv.Itoa(r.Status)).FontSize(12).TextColor(muted).FontFeatures("tnum")
				}
				if r.Purpose == "compaction" {
					kit.Tag(c, i18n.T(c, "chat_compact"))
				}
				ui.Spacer(c)
				ui.Text(c, r.elapsed()).FontSize(12).TextColor(muted).FontFeatures("tnum").SingleLine()
			})
			if r.Provider.Model != "" {
				ui.Text(c, r.Provider.Model).FontSize(12).TextColor(muted).SingleLine().PaddingStart(14)
			}
			if f := r.finding(); f != nil {
				ui.Text(c, i18n.T(c, f.Summary)).FontSize(12).TextColor(toneColor(p, r.tone())).SingleLine().PaddingStart(14)
			} else if in, out := r.metric("input"), r.metric("output"); in != "-" || out != "-" {
				ui.Text(c, i18n.T(c, "meter_input")+" "+in+" · "+i18n.T(c, "meter_output")+" "+out).
					FontSize(12).TextColor(muted).FontFeatures("tnum").SingleLine().PaddingStart(14)
			}
		})
	}).OnClick(func() {
		// The tab stays, so attempts compare side by side.
		v.request, v.attempt, v.detail = r.RequestID, r.Attempt, true
		v.preview = tracePreview{}
	})
}

func (s *State) traceDetail(c *ui.Context, r traceRow) {
	v := &s.trace
	p := theme.Of(c)
	ui.Column(c.Key("summary")).Padding(10, 12, 6, 12).Gap(6).Children(func() {
		ui.Row(c).FillWidth().Gap(8).AlignItems(ui.Center).Children(func() {
			ui.Text(c, "#"+strconv.Itoa(r.Attempt)).FontSize(15).FontWeight(600).FontFeatures("tnum")
			ui.Text(c, r.Provider.Model).FontSize(15).FontWeight(600).SingleLine().Shrink(1).MinWidth(0)
			ui.Spacer(c)
			kit.TextAction(c.Key("jump"), i18n.T(c, "trace_locate"), s.TraceJump == nil || (r.Record == "" && r.Turn == "")).
				OnClick(func() { s.TraceJump(r.Record, r.Turn) })
		})
		ui.Row(c).FillWidth().Gap(6).AlignItems(ui.Center).Wrap().Children(func() {
			r.badge(c)
			if f := r.finding(); f != nil {
				kit.Badge(c.Key("finding"), i18n.T(c, f.Summary), r.tone())
			}
			if r.Purpose == "compaction" {
				kit.Tag(c.Key("purpose"), i18n.T(c, "chat_compact"))
			}
			var meta []string
			if r.Status != 0 {
				meta = append(meta, "HTTP "+strconv.Itoa(r.Status))
			}
			if e := r.elapsed(); e != "-" {
				meta = append(meta, e)
			}
			if r.TS != 0 {
				meta = append(meta, time.UnixMilli(r.TS).Local().Format("15:04:05"))
			}
			if len(meta) > 0 {
				ui.Text(c, strings.Join(meta, " · ")).FontSize(12).TextColor(p.MutedForeground).FontFeatures("tnum").SingleLine()
			}
		})
	})
	tab := slices.IndexFunc(traceTabs, func(t struct{ id, label string }) bool { return t.id == v.tab })
	chosen := max(tab, 0)
	labels := make([]string, len(traceTabs))
	for i, t := range traceTabs {
		labels[i] = i18n.T(c, t.label)
	}
	ui.ScrollHorizontal(c.Key("tabs")).Scrollbars(false).Shrink(0).Children(func() {
		ui.Row(c).Padding(0, 12, 8, 12).Children(func() {
			kit.Segmented(c, "segments", &chosen, labels, kit.SegmentedProps{Size: kit.SizeSmall})
		})
	})
	if chosen != tab {
		v.tab = traceTabs[chosen].id
		v.preview = tracePreview{}
	}
	ui.Box(c.Key("rule")).FillWidth().Height(1).Shrink(0).Background(p.Border)
	switch v.tab {
	case "request", "response":
		// The body's editor scrolls itself, so its find bar stays in view.
		ui.Column(c.Key(r.RequestID).Key(r.Attempt).Key(v.tab)).Grow(1).MinHeight(0).Children(func() { s.traceBody(c, r) })
		return
	}
	ui.Scroll(c.Key(r.RequestID).Key(r.Attempt).Key(v.tab)).Grow(1).MinHeight(0).Children(func() {
		ui.Column(c).FillWidth().Padding(12).Gap(16).Children(func() {
			if v.tab == "metrics" {
				traceUsage(c, r)
				traceTiming(c, r)
				return
			}
			r.diagnosis(c)
			traceFacts(c, r)
			traceCategories(c, r)
		})
	})
}

// traceSection is a group of the detail's facts, under its title unless
// the tab already names it.
func traceSection(c *ui.Context, key any, title string, body func()) {
	ui.Column(c.Key(key)).FillWidth().Gap(4).Children(func() {
		if title != "" {
			ui.Text(c, title).FontSize(13).FontWeight(600).Padding(0, 0, 2, 0)
		}
		body()
	})
}

// traceFact is one label and value row of a section.
func traceFact(c *ui.Context, key any, label string, value func()) {
	ui.Row(c.Key(key)).FillWidth().Gap(12).AlignItems(ui.Center).MinHeight(24).Children(func() {
		ui.Text(c, label).Width(96).Shrink(0).FontSize(13).TextColor(theme.Of(c).MutedForeground).SingleLine()
		ui.Row(c).Grow(1).MinWidth(0).Gap(6).AlignItems(ui.Center).Children(value)
	})
}

func traceFactText(c *ui.Context, key any, label, value string) {
	traceFact(c, key, label, func() { ui.Text(c, value).FontSize(13).Selectable().Shrink(1).MinWidth(0) })
}

// traceFactID is an identifier a reader quotes to a provider or gateway.
func traceFactID(c *ui.Context, key any, label, value string) {
	traceFact(c, key, label, func() {
		ui.Text(c, value).FontSize(12).Font("monospace").Selectable().SingleLine().TruncateMiddle().Shrink(1).MinWidth(0)
		kit.IconButton(c.Key("copy"), icons.Copy, i18n.T(c, "connections_copy_id"), 20, false).
			TextColor(theme.Of(c).MutedForeground).OnClick(func() {
			c.WriteClipboard(value)
			kit.Copied(c)
		})
	})
}

func traceFacts(c *ui.Context, r traceRow) {
	t := func(k string) string { return i18n.T(c, k) }
	traceSection(c, "facts", "", func() {
		if r.TS != 0 {
			traceFactText(c, "started", t("trace_started"), time.UnixMilli(r.TS).Local().Format("2006-01-02 15:04:05.000 MST"))
		}
		if r.Provider.Model != "" {
			traceFactText(c, "model", t("teams_model"), r.Provider.Model)
		}
		if r.Provider.ID != "" {
			traceFactID(c, "provider", t("insights_provider_id"), r.Provider.ID)
		}
		traceFactText(c, "status", t("insights_status"), r.status())
		if r.Response.End != "" {
			traceFactText(c, "end", t("insights_end"), r.Response.End)
		}
		if r.Failure.Kind != "" {
			traceFactText(c, "failure", t("insights_failure"), strings.TrimSpace(r.Failure.Kind+" "+r.Failure.Code))
		}
		traceFactID(c, "request", t("trace_request_id"), r.RequestID)
		if r.Response.Gateway != "" {
			traceFactID(c, "gateway", t("insights_gateway_id"), r.Response.Gateway)
		}
	})
}

// traceUsage lists the reported usage, the shares of cached input and
// reasoning output, then any other field the provider reported.
func traceUsage(c *ui.Context, r traceRow) {
	p := theme.Of(c)
	known := []struct{ key, label, of string }{
		{"input", "meter_input", ""}, {"output", "meter_output", ""}, {"cache_read", "meter_cache_read", "input"},
		{"cache_write", "trace_cache_write", ""}, {"reasoning", "meter_reasoning", "output"},
	}
	traceSection(c, "usage", i18n.T(c, "usage"), func() {
		for _, k := range known {
			value := r.metric(k.key)
			traceFact(c, k.key, i18n.T(c, k.label), func() {
				ui.Text(c, value).FontSize(13).FontFeatures("tnum").Width(72).Shrink(0)
				n, e1 := strconv.ParseUint(value, 10, 64)
				total, e2 := strconv.ParseUint(r.metric(k.of), 10, 64)
				if k.of == "" || e1 != nil || e2 != nil || total == 0 || n > total {
					return
				}
				share := float32(n) / float32(total)
				traceBar(c, 0, share, p.Primary)
				ui.Text(c, fmt.Sprintf("%.1f%%", 100*share)).FontSize(12).TextColor(p.MutedForeground).FontFeatures("tnum").Width(48).Shrink(0)
			})
		}
		for _, key := range slices.Sorted(maps.Keys(r.Usage)) {
			if slices.ContainsFunc(known, func(k struct{ key, label, of string }) bool { return k.key == key }) || string(r.Usage[key]) == "null" {
				continue
			}
			traceFact(c, "extra:"+key, key, func() {
				ui.Text(c, string(r.Usage[key])).FontSize(12).Font("monospace").Shrink(1).MinWidth(0)
			})
		}
	})
}

// traceBar is a span of a track as wide as the space left in its row:
// from and to are shares of the track.
func traceBar(c *ui.Context, from, to float32, color ui.Color) {
	p := theme.Of(c)
	from, to = min(max(from, 0), 1), min(max(to, 0), 1)
	ui.Row(c).Grow(1).MinWidth(40).Height(6).Radius(3).Background(p.Muted).Children(func() {
		if from > 0 {
			ui.Box(c.Key("lead")).Basis(0).Grow(from).Height(6)
		}
		ui.Box(c.Key("span")).Basis(0).Grow(max(to-from, 0.004)).MinWidth(2).Height(6).Radius(3).Background(color)
		if to < 1 {
			ui.Box(c.Key("rest")).Basis(0).Grow(1 - to).Height(6)
		}
	})
}

// traceTiming is the attempt's milestones over its elapsed time: each
// from the attempt's start, the longest gap where it began.
func traceTiming(c *ui.Context, r traceRow) {
	p := theme.Of(c)
	total := r.Timing["elapsed_us"]
	var shown bool
	for key := range r.Timing {
		shown = shown || slices.Contains([]string{"dispatch_us", "sent_us", "headers_us", "first_token_us", "elapsed_us", "max_gap_us"}, key)
	}
	if !shown {
		return
	}
	traceSection(c, "timing", i18n.T(c, "trace_timing"), func() {
		for i, key := range []string{"dispatch_us", "sent_us", "headers_us", "first_token_us", "elapsed_us", "max_gap_us"} {
			t, ok := r.Timing[key]
			if !ok {
				continue
			}
			label := []string{"trace_dispatch", "trace_sent", "trace_response_headers", "meter_first_response", "meter_request_time", "trace_gap"}[i]
			traceFact(c, key, i18n.T(c, label), func() {
				if total > 0 {
					from := float32(0)
					color := p.Primary
					if key == "max_gap_us" {
						color = p.Warning
						if v := r.Verdict; v != nil && v.GapStartUs != nil {
							from = float32(*v.GapStartUs) / float32(total)
						}
						traceBar(c, from, from+float32(t)/float32(total), color)
					} else {
						traceBar(c, 0, float32(min(t, total))/float32(total), color)
					}
				}
				ui.Text(c, traceDuration(t)).FontSize(12).FontFeatures("tnum").Width(64).Shrink(0).TextAlign(ui.End)
			})
		}
	})
}

// traceExportCount is how many captured attempts an export would contain.
func (s *State) traceExportCount(scope, turn string) int {
	n := 0
	for _, r := range s.trace.projected {
		if scope == "failed" && r.Outcome != "failed" || scope == "turn" && r.Turn != turn {
			continue
		}
		n++
	}
	return n
}

func (s *State) traceExportForm(c *ui.Context, turn string, selected *traceRow) {
	v := &s.trace
	p := theme.Of(c)
	title := i18n.T(c, "trace_export")
	if v.export {
		kit.Dialog(c.Key("trace-export"), &v.export, func() {
			ui.Text(c, title).FontSize(16).FontWeight(600).PaddingEnd(32).TextColor(p.PopoverForeground)
			ui.Text(c, i18n.T(c, "trace_private")).FontSize(13).TextColor(p.MutedForeground)
			turnLabel := i18n.T(c, "trace_turn")
			if selected != nil && selected.turnNumber > 0 {
				turnLabel = i18n.T(c, "trace_turn_number", "count", selected.turnNumber)
			}
			if turn == "" && v.scope == "turn" {
				v.scope = "all"
			}
			count := s.traceExportCount(v.scope, turn)
			hint := i18n.T(c, "trace_export_count", "count", count)
			if count == 0 {
				hint = i18n.T(c, "trace_export_none")
			}
			scope := i18n.T(c, "trace_scope")
			kit.Field(c, "scope", scope, hint, func() {
				names, values := []string{i18n.T(c, "market_all"), i18n.T(c, "meter_failed")}, []string{"all", "failed"}
				if turn != "" {
					names, values = append(names, turnLabel), append(values, "turn")
				}
				kit.Select(c.Key("scope"), "trace-scope", scope, &v.scope, names, false, values)
			})
			hint = i18n.T(c, "trace_export_shareable_hint")
			if v.mode == 1 {
				hint = i18n.T(c, "trace_export_raw_hint")
			}
			kit.Field(c, "mode", i18n.T(c, "trace_mode"), hint, func() {
				kit.Segmented(c, "trace-mode", &v.mode, []string{i18n.T(c, "trace_shareable"), i18n.T(c, "trace_raw")}, kit.SegmentedProps{Block: true})
			})
			ui.Row(c.Key("footer")).Gap(8).Justify(ui.End).Children(func() {
				kit.Secondary(c.Key("cancel"), i18n.T(c, "settings_cancel")).OnClick(func() { v.export = false })
				kit.Primary(c.Key("export-go"), title, s.TraceExport == nil || count == 0).OnClick(func() {
					v.export = false
					v.exportTurn = ""
					if v.scope == "turn" {
						v.exportTurn = turn
					}
					if v.mode == 1 {
						v.confirm = true
						return
					}
					s.TraceExport(v.scope, "shareable", v.exportTurn)
				})
			})
		}).Width(400).Label(title)
	}
	if v.confirm {
		if kit.Confirm(c.Key("trace-raw"), &v.confirm, kit.Confirmation{Title: title, Message: i18n.T(c, "trace_raw_confirm"), Buttons: []string{i18n.T(c, "settings_cancel"), title}, Default: 0}) == 1 && s.TraceExport != nil {
			v.confirm = false
			s.TraceExport(v.scope, "raw", v.exportTurn)
		}
	}
}

// traceHeaders lists a side's retained headers, a name per line.
func traceHeaders(c *ui.Context, h map[string][]string) {
	p := theme.Of(c)
	for _, name := range slices.Sorted(maps.Keys(h)) {
		ui.Row(c.Key(name)).FillWidth().Gap(8).Children(func() {
			ui.Text(c, name).FontSize(12).Font("monospace").TextColor(p.MutedForeground).Shrink(0).MaxWidth(180)
			ui.Text(c, strings.Join(h[name], ", ")).FontSize(12).Font("monospace").Grow(1).Shrink(1).MinWidth(0)
		})
	}
}
