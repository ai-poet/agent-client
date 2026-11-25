package panel

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

const tracePreviewLimit = protocol.TracePreviewLimit

type tracePreview struct {
	ref, raw, shown string
	events          []string
	event, offset   int
	truncated       bool
}

func prepareTraceBody(ref string, body []byte, truncated, response, sse bool) tracePreview {
	p := tracePreview{ref: ref, truncated: truncated || len(body) > tracePreviewLimit}
	body = body[:min(len(body), tracePreviewLimit)]
	if len(body) > 0 && !utf8.FullRune(body[lastRuneStart(body):]) {
		body = body[:lastRuneStart(body)]
	}
	p.raw = string(body)
	p.shown = p.raw
	if !response && !p.truncated && canIndent(body) {
		var b bytes.Buffer
		if json.Indent(&b, body, "", "  ") == nil && b.Len() <= tracePreviewLimit {
			p.shown = b.String()
		}
	}
	if response && (sse || strings.HasPrefix(p.raw, "data:") || strings.Contains(p.raw, "\ndata:")) {
		normalized := strings.ReplaceAll(strings.ReplaceAll(p.raw, "\r\n", "\n"), "\r", "\n")
		for part := range strings.SplitSeq(normalized, "\n\n") {
			if strings.TrimSpace(part) != "" {
				p.events = append(p.events, part)
			}
		}
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

// Bound indentation growth before formatting arbitrary provider JSON.
func canIndent(p []byte) bool {
	depth, extra := 0, 0
	quoted, escaped := false, false
	for _, b := range p {
		if quoted {
			if escaped {
				escaped = false
			} else if b == '\\' {
				escaped = true
			} else if b == '"' {
				quoted = false
			}
			continue
		}
		switch b {
		case '"':
			quoted = true
		case '{', '[':
			depth++
			extra += 2*depth + 1
		case '}', ']':
			depth--
			extra += 2*depth + 1
		case ',':
			extra += 2*depth + 1
		case ':':
			extra++
		}
		if depth < 0 || depth > 100 || extra+len(p) > tracePreviewLimit {
			return false
		}
	}
	return true
}

func eventLabel(event string) string {
	var data []string
	for line := range strings.SplitSeq(event, "\n") {
		if label, ok := strings.CutPrefix(line, "event:"); ok && strings.TrimSpace(label) != "" {
			return strings.TrimSpace(label)
		}
		if value, ok := strings.CutPrefix(line, "data:"); ok {
			data = append(data, strings.TrimPrefix(value, " "))
		}
	}
	var v struct{ Type string }
	_ = json.Unmarshal([]byte(strings.Join(data, "\n")), &v)
	return v.Type
}

func (s *State) traceBody(c *ui.Context, r traceRow) {
	v := &s.trace
	response := v.tab == "response"
	side, partial := r.Request, r.Bytes.ReqTruncated
	if response {
		side, partial = r.Response, r.Bytes.ResTruncated
		if side.End != "" {
			ui.Text(c, side.End)
		}
	}
	traceHeaders(c, side.Headers)
	if partial || side.Body != "" && s.TraceContent.Body == side.Body && s.TraceContent.Truncated {
		kit.Badge(c, i18n.T(c, "trace_partial"), kit.Caution)
	}
	if side.Body == "" {
		ui.Text(c, i18n.T(c, "unavailable"))
		return
	}
	if side.Body != s.TraceContent.Body {
		kit.Secondary(c.Key("load"), i18n.T(c, "trace_load")).Disabled(s.TraceReadBody == nil).OnClick(func() { s.TraceReadBody(side.Body) })
		return
	}
	p := &v.preview
	if p.ref != side.Body {
		*p = prepareTraceBody(side.Body, s.TraceContent.Content, s.TraceContent.Truncated, response, strings.HasPrefix(strings.ToLower(side.ContentType), "text/event-stream"))
	}
	if p.truncated && !s.TraceContent.Truncated {
		kit.Badge(c, i18n.T(c, "trace_partial"), kit.Caution)
	}
	kit.Ghost(c.Key("copy"), i18n.T(c, "content_copy"), false).OnClick(func() { c.WriteClipboard(p.raw) })
	shown := p.shown
	if len(p.events) > 0 {
		ui.Checkbox(c.Key("split"), &v.split, i18n.T(c, "trace_events")+" ("+fmt.Sprint(len(p.events))+")")
		if v.split {
			ui.Row(c.Key("event")).Gap(6).Children(func() {
				kit.Ghost(c.Key("prev"), "<", false).Label(i18n.T(c, "knowledge_previous")).Disabled(p.event == 0).OnClick(func() { p.event--; p.offset = 0 })
				ui.Text(c, fmt.Sprintf("%d/%d %s", p.event+1, len(p.events), eventLabel(p.events[p.event]))).Grow(1).SingleLine()
				kit.Ghost(c.Key("next"), ">", false).Label(i18n.T(c, "knowledge_next")).Disabled(p.event+1 >= len(p.events)).OnClick(func() { p.event++; p.offset = 0 })
			})
			shown = p.events[p.event]
		}
	}
	// Bound text shaping per frame, even for a single multi-megabyte JSON string.
	const chunk = 4096
	if p.offset >= len(shown) {
		p.offset = 0
	}
	start := p.offset
	for start > 0 && !utf8.RuneStart(shown[start]) {
		start--
	}
	end := min(len(shown), p.offset+chunk)
	for end < len(shown) && !utf8.RuneStart(shown[end]) {
		end--
	}
	if len(shown) > chunk {
		ui.Row(c.Key("pages")).Gap(6).Children(func() {
			kit.Ghost(c.Key("prev"), i18n.T(c, "knowledge_previous"), false).Disabled(p.offset == 0).OnClick(func() { p.offset -= chunk })
			ui.Text(c, fmt.Sprintf("%d/%d", 1+p.offset/chunk, (len(shown)+chunk-1)/chunk))
			kit.Ghost(c.Key("next"), i18n.T(c, "knowledge_next"), false).Disabled(end == len(shown)).OnClick(func() { p.offset += chunk })
		})
	}
	ui.Text(c, shown[start:end]).FontSize(12)
}
