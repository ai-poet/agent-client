package panel

import (
	"encoding/json"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
	"github.com/egoist/mygo/ui/editor"
)

const tracePreviewLimit = protocol.TracePreviewLimit

type tracePreview struct {
	ref, raw, shown string
	// json is whether shown is (a prefix of) JSON, laid out a value per line.
	json      bool
	events    []string
	event     int
	truncated bool
}

func prepareTraceBody(ref string, body []byte, truncated, response, sse bool) tracePreview {
	p := tracePreview{ref: ref, truncated: truncated || len(body) > tracePreviewLimit}
	body = body[:min(len(body), tracePreviewLimit)]
	if len(body) > 0 && !utf8.FullRune(body[lastRuneStart(body):]) {
		body = body[:lastRuneStart(body)]
	}
	p.raw = string(body)
	p.shown = p.raw
	if response && (sse || strings.HasPrefix(p.raw, "data:") || strings.Contains(p.raw, "\ndata:")) {
		normalized := strings.ReplaceAll(strings.ReplaceAll(p.raw, "\r\n", "\n"), "\r", "\n")
		for part := range strings.SplitSeq(normalized, "\n\n") {
			if strings.TrimSpace(part) != "" {
				p.events = append(p.events, part)
			}
		}
		return p
	}
	if shown, ok := indentJSON(p.raw); ok {
		p.shown, p.json = shown, true
	}
	return p
}
func lastRuneStart(p []byte) int {
	i := len(p) - 1
	for i > 0 && !utf8.RuneStart(p[i]) {
		i--
	}
	return i
}

// indentJSON lays out JSON, or the prefix of it a capped preview keeps, a
// value per line. It does not validate: a truncated body still reads, and
// one long line never reaches the editor's soft wrap. It refuses text that
// is not JSON-shaped and layouts that would outgrow the preview several
// times over.
func indentJSON(s string) (string, bool) {
	t := strings.TrimLeft(s, " \t\r\n")
	if t == "" || t[0] != '{' && t[0] != '[' {
		return "", false
	}
	var b strings.Builder
	b.Grow(len(t) + len(t)/4)
	depth := 0
	newline := func() {
		b.WriteByte('\n')
		for range min(depth, 32) {
			b.WriteString("  ")
		}
	}
	quoted, escaped := false, false
	for i := 0; i < len(t); i++ {
		ch := t[i]
		if quoted {
			b.WriteByte(ch)
			switch {
			case escaped:
				escaped = false
			case ch == '\\':
				escaped = true
			case ch == '"':
				quoted = false
			}
			continue
		}
		switch ch {
		case ' ', '\t', '\r', '\n':
			continue
		case '"':
			quoted = true
			b.WriteByte(ch)
		case '{', '[':
			// An empty object or array stays on its line.
			j := i + 1
			for j < len(t) && strings.IndexByte(" \t\r\n", t[j]) >= 0 {
				j++
			}
			if j < len(t) && (ch == '{' && t[j] == '}' || ch == '[' && t[j] == ']') {
				b.WriteByte(ch)
				b.WriteByte(t[j])
				i = j
				continue
			}
			b.WriteByte(ch)
			depth++
			newline()
		case '}', ']':
			depth = max(depth-1, 0)
			newline()
			b.WriteByte(ch)
		case ',':
			b.WriteByte(ch)
			newline()
		case ':':
			b.WriteString(": ")
		default:
			b.WriteByte(ch)
		}
		if b.Len() > 3*tracePreviewLimit {
			return "", false
		}
	}
	return b.String(), true
}

func eventLabel(event string) string {
	var v struct{ Type string }
	_ = json.Unmarshal([]byte(eventData(event)), &v)
	for line := range strings.SplitSeq(event, "\n") {
		if label, ok := strings.CutPrefix(line, "event:"); ok && strings.TrimSpace(label) != "" {
			return strings.TrimSpace(label)
		}
	}
	return v.Type
}

// eventData is the event's data lines joined, as an SSE client reads them.
func eventData(event string) string {
	var data []string
	for line := range strings.SplitSeq(event, "\n") {
		if value, ok := strings.CutPrefix(line, "data:"); ok {
			data = append(data, strings.TrimPrefix(value, " "))
		}
	}
	return strings.Join(data, "\n")
}

func (s *State) traceBody(c *ui.Context, r traceRow) {
	v := &s.trace
	p := theme.Of(c)
	response := v.tab == "response"
	side, partial := r.Request, r.Bytes.ReqTruncated
	headers := "mcp_headers"
	if response {
		side, partial = r.Response, r.Bytes.ResTruncated
		headers = "trace_response_headers"
	}
	loaded := side.Body != "" && side.Body == s.TraceContent.Body
	if side.Body != "" && !loaded && v.requested != side.Body && s.TraceReadBody != nil {
		v.requested = side.Body
		s.TraceReadBody(side.Body)
		loaded = side.Body == s.TraceContent.Body
	}
	pv := &v.preview
	if loaded && pv.ref != side.Body {
		*pv = prepareTraceBody(side.Body, s.TraceContent.Content, s.TraceContent.Truncated, response, strings.HasPrefix(strings.ToLower(side.ContentType), "text/event-stream"))
	}
	ui.Row(c.Key("meta")).FillWidth().Padding(8, 12).Gap(6).AlignItems(ui.Center).Shrink(0).Children(func() {
		if n := len(side.Headers); n > 0 {
			label := fmt.Sprintf("%s %d", i18n.T(c, headers), n)
			chevron := icons.ChevronRight
			if v.headers {
				chevron = icons.ChevronDown
			}
			kit.GhostLook(c, ui.ButtonBase(c.Key("headers")).Label(label), v.headers).Height(24).PaddingX(6).Gap(4).Children(func() {
				ui.Icon(c, chevron).FontSize(12).TextColor(p.MutedForeground)
				ui.Text(c, label).FontSize(12).SingleLine()
			}).OnClick(func() { v.headers = !v.headers })
		}
		if response && side.End != "" {
			ui.Text(c, side.End).FontSize(12).Font("monospace").TextColor(p.MutedForeground).SingleLine()
		}
		if partial || loaded && (s.TraceContent.Truncated || pv.truncated) {
			kit.Badge(c.Key("partial"), i18n.T(c, "trace_partial"), kit.Caution)
		}
		ui.Spacer(c)
		if loaded && len(pv.events) > 0 {
			ui.Checkbox(c.Key("split"), &v.split, fmt.Sprintf("%s %d", i18n.T(c, "trace_events"), len(pv.events))).FontSize(12)
		}
		if loaded {
			kit.IconButton(c.Key("copy"), icons.Copy, i18n.T(c, "content_copy"), 24, false).TextColor(p.MutedForeground).OnClick(func() {
				c.WriteClipboard(pv.raw)
				kit.Copied(c)
			})
		}
	})
	if v.headers && len(side.Headers) > 0 {
		ui.Scroll(c.Key("header-list")).MaxHeight(160).Shrink(0).Children(func() {
			ui.Column(c).FillWidth().Padding(0, 12, 8, 12).Gap(2).Children(func() {
				traceHeaders(c, side.Headers)
				if len(side.Dropped) > 0 {
					ui.Text(c, i18n.T(c, "trace_dropped_headers")+": "+strings.Join(side.Dropped, ", ")).FontSize(12).TextColor(p.MutedForeground).Padding(4, 0, 0, 0)
				}
			})
		})
	}
	ui.Box(c.Key("rule")).FillWidth().Height(1).Shrink(0).Background(p.Border)
	switch {
	case side.Body == "":
		hint := "trace_body_capture_hint"
		if r.captured {
			hint = "trace_body_not_saved"
		}
		kit.Result(c, kit.ResultProps{Icon: icons.FileText, Title: i18n.T(c, "trace_body_missing"), SubTitle: i18n.T(c, hint)}).Grow(1).Justify(ui.Center)
		return
	case !loaded && s.TraceBodyFailed:
		kit.Result(c, kit.ResultProps{Status: kit.ResultError, Title: i18n.T(c, "trace_body_failed"), Extra: func() {
			kit.Secondary(c.Key("retry"), i18n.T(c, "chat_retry")).Disabled(s.TraceReadBody == nil).OnClick(func() { v.requested = "" })
		}}).Grow(1).Justify(ui.Center)
		return
	case !loaded:
		ui.Column(c.Key("loading")).Grow(1).Center().Children(func() { kit.Loader(c, 20).Label(i18n.T(c, "trace_load")) })
		return
	}
	shown, json := pv.shown, pv.json
	if v.split && len(pv.events) > 0 {
		pv.event = min(pv.event, len(pv.events)-1)
		ui.Row(c.Key("event")).FillWidth().Padding(6, 12).Gap(6).AlignItems(ui.Center).Shrink(0).Children(func() {
			kit.Action(c.Key("prev"), icons.ChevronLeft, i18n.T(c, "knowledge_previous"), pv.event == 0).OnClick(func() { pv.event-- })
			ui.Text(c, fmt.Sprintf("%d/%d", pv.event+1, len(pv.events))).FontSize(12).FontFeatures("tnum").Shrink(0)
			ui.Text(c, eventLabel(pv.events[pv.event])).FontSize(12).Font("monospace").TextColor(p.MutedForeground).SingleLine().Grow(1).MinWidth(0)
			kit.Action(c.Key("next"), icons.ChevronRight, i18n.T(c, "knowledge_next"), pv.event+1 >= len(pv.events)).OnClick(func() { pv.event++ })
		})
		shown, json = pv.events[pv.event], false
		if data, ok := indentJSON(eventData(shown)); ok {
			shown, json = data, true
		}
	}
	o := kit.CodeOptions{Numbers: true, Wrap: true, Fill: true}
	if json {
		o.Language = editor.LanguageNamed("json")
	}
	key := fmt.Sprintf("%s:%t:%d", pv.ref, v.split, pv.event)
	kit.Code(c, ui.Column(c.Key("code").Key(key)).Grow(1).MinHeight(0).PaddingY(4), shown, o)
}
