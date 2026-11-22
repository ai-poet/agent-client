// Package compact owns bounded structured handoffs, never canonical history.
// Adapted from Goose 14741e9a (Apache-2.0); see ../model/LICENSE.goose.
package compact

import (
	"encoding/json"
	"fmt"
	"maps"
	"slices"
	"strings"
	"unicode"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/tokens"
)

type File struct {
	Path    string  `json:"path"`
	Summary string  `json:"summary"`
	KeyCode *string `json:"key_code"`
}
type Summary struct {
	UserIntent        []string `json:"user_intent"`
	TechnicalConcepts []string `json:"technical_concepts"`
	Files             []File   `json:"files"`
	ErrorsAndFixes    []string `json:"errors_and_fixes"`
	ProblemSolving    []string `json:"problem_solving"`
	UserMessages      []string `json:"user_messages"`
	PendingTasks      []string `json:"pending_tasks"`
	CurrentWork       *string  `json:"current_work"`
	NextStep          *string  `json:"next_step"`
}

func stringify(value any) string {
	switch value := value.(type) {
	case nil:
		return ""
	case string:
		return value
	case []any:
		var parts []string
		for _, v := range value {
			parts = append(parts, stringify(v))
		}
		return strings.Join(parts, "; ")
	case map[string]any:
		keys := make([]string, 0, len(value))
		for key := range value {
			keys = append(keys, key)
		}
		slices.Sort(keys)
		var parts []string
		for _, key := range keys {
			parts = append(parts, key+": "+stringify(value[key]))
		}
		return strings.Join(parts, "; ")
	default:
		raw, _ := json.Marshal(value)
		return string(raw)
	}
}
func texts(value any) []string {
	var values []any
	if v, ok := value.([]any); ok {
		values = v
	} else if value != nil {
		values = []any{value}
	}
	out := []string{}
	for _, v := range values {
		if text := stringify(v); strings.TrimSpace(text) != "" {
			out = append(out, text)
		}
	}
	return out
}
func optional(value any) *string {
	if value == nil {
		return nil
	}
	text := stringify(value)
	if strings.TrimSpace(text) == "" {
		return nil
	}
	return &text
}

// Parse reads the handoff of a summary response and bounds its rendered text
// to limit tokens.
func Parse(text string, limit uint64) (Summary, error) {
	candidates := extractCandidates(text)
	for _, candidate := range candidates {
		fields, ok := decodeObject(candidate)
		if !ok {
			continue
		}
		summary := buildSummary(fields)
		if summary.empty() {
			continue
		}
		summary.Bound(limit)
		if tokens.Text(summary.Render()) > limit {
			continue
		}
		return summary, nil
	}
	return Summary{}, fmt.Errorf("compaction summary was not a structured handoff")
}

func extractCandidates(text string) []string {
	var candidates []string
	seen := map[string]bool{}
	add := func(candidate string, later int, enforceTerminator bool) {
		if candidate == "" || seen[candidate] {
			return
		}
		if enforceTerminator && strings.Count(candidate, "</analysis>") != later {
			return
		}
		seen[candidate] = true
		candidates = append(candidates, candidate)
	}
	cuts := []int{}
	for offset := 0; ; {
		i := strings.Index(text[offset:], "</analysis>")
		if i < 0 {
			break
		}
		offset += i + len("</analysis>")
		cuts = append(cuts, offset)
	}
	if len(cuts) == 0 {
		cuts = []int{0}
	}
	for i := len(cuts) - 1; i >= 0; i-- {
		tail := text[cuts[i]:]
		if strings.HasPrefix(strings.TrimSpace(tail), "<analysis>") {
			continue // An unfinished analysis block is not an answer.
		}
		later := strings.Count(tail, "</analysis>")
		for _, block := range fencedBlocks(tail) {
			if candidate := object(block); candidate != "" {
				add(candidate, later, true)
			}
			for _, candidate := range embeddedObjects(block, true) {
				add(candidate, later, true)
			}
		}
		for _, block := range taggedBlocks(tail) {
			if candidate := object(block); candidate != "" {
				add(candidate, later, true)
			}
			for _, candidate := range embeddedObjects(block, true) {
				add(candidate, later, true)
			}
		}
		add(object(tail), later, true)
		for _, candidate := range embeddedObjects(tail, false) {
			add(candidate, later, true)
		}
	}
	add(object(text), 0, false)
	return candidates
}

// fencedBlocks extracts markdown code-fence bodies (``` or ~~~, with or without
// a json/JSON/jsonc language tag), ordered newest-first.
func fencedBlocks(text string) []string {
	var blocks []string
	for offset := 0; offset < len(text); {
		lineEnd := strings.IndexByte(text[offset:], '\n')
		var line string
		if lineEnd < 0 {
			line = text[offset:]
			offset = len(text)
		} else {
			line = text[offset : offset+lineEnd]
			offset += lineEnd + 1
		}
		trimmed := strings.TrimSpace(line)
		fenceChar := byte(0)
		if strings.HasPrefix(trimmed, "```") {
			fenceChar = '`'
		} else if strings.HasPrefix(trimmed, "~~~") {
			fenceChar = '~'
		}
		if fenceChar == 0 {
			continue
		}
		fenceLen := 0
		for fenceLen < len(trimmed) && trimmed[fenceLen] == fenceChar {
			fenceLen++
		}
		rest := strings.TrimSpace(trimmed[fenceLen:])
		if !supportedFenceLang(rest) {
			// A fence with inline JSON on the same line, e.g. ```json {"user_intent":...}
			if idx := strings.IndexByte(rest, '{'); idx >= 0 {
				prefix := strings.TrimSpace(rest[:idx])
				if supportedFenceLang(prefix) {
					blocks = append(blocks, rest[idx:])
				}
			}
			continue
		}
		// Collect until a matching closing fence or EOF.
		bodyStart := offset
		bodyEnd := len(text)
		closed := false
		for scan := offset; scan < len(text); {
			nextNL := strings.IndexByte(text[scan:], '\n')
			var scanLine string
			nextScan := len(text)
			if nextNL < 0 {
				scanLine = text[scan:]
			} else {
				scanLine = text[scan : scan+nextNL]
				nextScan = scan + nextNL + 1
			}
			closeTrimmed := strings.TrimSpace(scanLine)
			if isClosingFence(closeTrimmed, fenceChar, fenceLen) {
				bodyEnd = scan
				offset = nextScan
				closed = true
				break
			}
			scan = nextScan
		}
		if !closed {
			offset = len(text)
		}
		if bodyEnd > bodyStart {
			blocks = append(blocks, text[bodyStart:bodyEnd])
		}
	}
	slices.Reverse(blocks)
	return blocks
}

func supportedFenceLang(tag string) bool {
	if tag == "" {
		return true
	}
	field := strings.ToLower(strings.Fields(tag)[0])
	switch field {
	case "json", "jsonc", "json5", "javascript", "js":
		return true
	}
	return false
}

func isClosingFence(trimmed string, fenceChar byte, minLen int) bool {
	if len(trimmed) < minLen {
		return false
	}
	for i := 0; i < len(trimmed); i++ {
		if trimmed[i] != fenceChar {
			return false
		}
	}
	return true
}

// taggedBlocks extracts contents inside <summary>, <handoff>, or <json> tags,
// ordered newest-first.
func taggedBlocks(text string) []string {
	// Fold only ASCII tag names so offsets also index the original UTF-8.
	lower := strings.Map(func(r rune) rune {
		if r >= 'A' && r <= 'Z' {
			return r + 'a' - 'A'
		}
		return r
	}, text)
	var blocks []string
	for _, tag := range []string{"handoff", "summary", "json"} {
		openTag := "<" + tag + ">"
		closeTag := "</" + tag + ">"
		for offset := 0; ; {
			start := strings.Index(lower[offset:], openTag)
			if start < 0 {
				break
			}
			bodyStart := offset + start + len(openTag)
			end := strings.Index(lower[bodyStart:], closeTag)
			if end < 0 {
				blocks = append(blocks, text[bodyStart:])
				break
			}
			blocks = append(blocks, text[bodyStart:bodyStart+end])
			offset = bodyStart + end + len(closeTag)
		}
	}
	slices.Reverse(blocks)
	return blocks
}

// embeddedObjects scans text for balanced top-level JSON objects embedded
// amid explanatory prose. Unless allowInline is true, single-line snippets
// embedded mid-sentence without a leading colon or line break (such as
// `A quoted example {"user_intent":["Wrong"]} is not a handoff`) are skipped.
func embeddedObjects(text string, allowInline bool) []string {
	trimmed := strings.TrimSpace(text)
	if strings.HasPrefix(trimmed, "[") && json.Valid([]byte(trimmed)) {
		return nil // An array of examples is not a handoff object.
	}
	var found []string
	for i := 0; i < len(text); i++ {
		if text[i] == '[' {
			if array := balancedValue(text[i:], '[', ']'); array != "" && json.Valid([]byte(array)) {
				i += len(array) - 1
				continue
			}
		}
		if text[i] != '{' {
			continue
		}
		candidate := object(text[i:])
		if candidate == "" {
			continue
		}
		end := i + len(candidate)
		if allowInline || standsAloneObject(text, i, end) {
			found = append(found, candidate)
		}
		i = end - 1
	}
	return found
}

func standsAloneObject(text string, start, end int) bool {
	candidate := text[start:end]
	if strings.ContainsRune(candidate, '\n') {
		return true
	}
	lineStart := strings.LastIndexByte(text[:start], '\n') + 1
	before := strings.TrimSpace(text[lineStart:start])
	if before == "" {
		return true
	}
	// A leading colon/arrow introduces the JSON block on the same line.
	lastRune := rune(0)
	for _, r := range before {
		lastRune = r
	}
	if lastRune == ':' || lastRune == '：' {
		return true
	}
	// Otherwise require that nothing meaningful continues on the same line.
	restEnd := strings.IndexByte(text[end:], '\n')
	var after string
	if restEnd < 0 {
		after = strings.TrimSpace(text[end:])
	} else {
		after = strings.TrimSpace(text[end : end+restEnd])
	}
	return after == "" || after == "." || after == "。"
}

func decodeObject(candidate string) (map[string]any, bool) {
	decode := func(variant string) (map[string]any, bool) {
		var fields map[string]any
		decoder := json.NewDecoder(strings.NewReader(variant))
		decoder.UseNumber()
		ok := decoder.Decode(&fields) == nil && fields != nil
		return fields, ok
	}
	if fields, ok := decode(candidate); ok {
		return fields, true
	}
	repaired := stripTrailingCommas(escape(candidate))
	if fields, ok := decode(repaired); ok {
		return fields, true
	}
	return decode(repair(repaired))
}

func stripTrailingCommas(text string) string {
	var out strings.Builder
	out.Grow(len(text))
	quoted, escaped := false, false
	for i := 0; i < len(text); i++ {
		c := text[i]
		if quoted {
			out.WriteByte(c)
			if escaped {
				escaped = false
			} else if c == '\\' {
				escaped = true
			} else if c == '"' {
				quoted = false
			}
			continue
		}
		if c == '"' {
			quoted = true
			out.WriteByte(c)
			continue
		}
		if c == ',' {
			j := i + 1
			for j < len(text) && (text[j] == ' ' || text[j] == '\t' || text[j] == '\n' || text[j] == '\r') {
				j++
			}
			if j < len(text) && (text[j] == '}' || text[j] == ']') {
				continue
			}
		}
		out.WriteByte(c)
	}
	return out.String()
}

func normalizeKey(key string) string {
	var out strings.Builder
	var prev rune
	for i, r := range strings.TrimSpace(key) {
		switch {
		case r == '-' || r == ' ' || r == '\t':
			if out.Len() > 0 && prev != '_' {
				out.WriteByte('_')
				prev = '_'
			}
		case unicode.IsUpper(r):
			if i > 0 && prev != '_' && !unicode.IsUpper(prev) {
				out.WriteByte('_')
			}
			out.WriteRune(unicode.ToLower(r))
			prev = r
		default:
			out.WriteRune(r)
			prev = r
		}
	}
	return strings.Trim(out.String(), "_")
}

func normalizeFields(raw map[string]any) map[string]any {
	normalized := make(map[string]any, len(raw))
	for _, key := range slices.Sorted(maps.Keys(raw)) {
		norm := normalizeKey(key)
		if _, exists := normalized[norm]; !exists || key == norm {
			normalized[norm] = raw[key]
		}
	}
	return normalized
}

func firstField(fields map[string]any, keys ...string) any {
	for _, key := range keys {
		if value := fields[key]; value != nil {
			return value
		}
	}
	return nil
}

func (s Summary) empty() bool {
	return len(s.UserIntent)+len(s.TechnicalConcepts)+len(s.ErrorsAndFixes)+len(s.ProblemSolving)+len(s.UserMessages)+len(s.PendingTasks)+len(s.Files) == 0 && s.CurrentWork == nil && s.NextStep == nil
}

func buildSummary(raw map[string]any) Summary {
	normalized := normalizeFields(raw)
	pick := func(keys ...string) any { return firstField(normalized, keys...) }
	summary := Summary{
		UserIntent:        texts(pick("user_intent", "goal", "goals", "intent", "objective", "objectives")),
		TechnicalConcepts: texts(pick("technical_concepts", "essential_context", "context", "key_context", "references", "commands")),
		ErrorsAndFixes:    texts(pick("errors_and_fixes", "errors", "issues", "fixes", "blockers")),
		ProblemSolving:    texts(pick("problem_solving", "decisions", "key_decisions", "approach")),
		UserMessages:      texts(pick("user_messages", "constraints", "requirements", "user_constraints", "rules")),
		PendingTasks:      texts(pick("pending_tasks", "open_tasks", "remaining_tasks", "tasks", "todo", "todos")),
		CurrentWork:       optional(pick("current_work", "state", "status", "progress", "work_in_progress")),
		NextStep:          optional(pick("next_step", "next_steps", "action_items")),
		Files:             []File{},
	}
	filesVal := pick("files", "key_files", "relevant_files", "modified_files", "touched_files")
	files := []any{}
	if items, ok := filesVal.([]any); ok {
		files = items
	} else if filesVal != nil {
		files = append(files, filesVal)
	}
	for _, file := range files {
		value := File{}
		if entry, ok := file.(map[string]any); ok {
			entryNorm := normalizeFields(entry)
			pickFile := func(keys ...string) any { return firstField(entryNorm, keys...) }
			value = File{
				Path:    stringify(pickFile("path", "file", "filepath", "file_path", "filename", "name")),
				Summary: stringify(pickFile("summary", "description", "reason", "notes", "status", "changes")),
				KeyCode: optional(pickFile("key_code", "code", "snippet", "excerpt")),
			}
		} else {
			value.Path = stringify(file)
		}
		if strings.TrimSpace(value.Path) != "" || strings.TrimSpace(value.Summary) != "" || value.KeyCode != nil {
			summary.Files = append(summary.Files, value)
		}
	}
	if summary.empty() {
		for _, wrapper := range []string{"handoff", "summary", "result", "data"} {
			if inner, ok := normalized[wrapper].(map[string]any); ok {
				if nested := buildSummary(inner); !nested.empty() {
					return nested
				}
			}
		}
		if textSummary, ok := normalized["summary"].(string); ok {
			summary.CurrentWork = optional(textSummary)
		}
	}
	return summary
}

func object(text string) string {
	return balancedValue(strings.TrimSpace(text), '{', '}')
}

func balancedValue(text string, open, close rune) string {
	if len(text) == 0 || rune(text[0]) != open {
		return ""
	}
	depth := 0
	quoted, escaped := false, false
	for i, r := range text {
		if quoted {
			if escaped {
				escaped = false
			} else if r == '\\' {
				escaped = true
			} else if r == '"' {
				quoted = false
			}
			continue
		}
		switch r {
		case '"':
			quoted = true
		case open:
			depth++
		case close:
			depth--
			if depth == 0 {
				return text[:i+1]
			}
		}
	}
	return ""
}
func repair(text string) string {
	var out strings.Builder
	var closers []rune
	quoted, escaped := false, false
	for _, r := range text {
		out.WriteRune(r)
		if quoted {
			if escaped {
				escaped = false
			} else if r == '\\' {
				escaped = true
			} else if r == '"' {
				quoted = false
			}
			continue
		}
		switch r {
		case '"':
			quoted = true
		case '{':
			closers = append(closers, '}')
		case '[':
			closers = append(closers, ']')
		case '}', ']':
			if len(closers) > 0 && closers[len(closers)-1] == r {
				closers = closers[:len(closers)-1]
			}
		}
	}
	if quoted {
		if escaped {
			out.WriteByte('\\')
		}
		out.WriteByte('"')
	}
	for i := len(closers) - 1; i >= 0; i-- {
		out.WriteRune(closers[i])
	}
	return out.String()
}
func escape(text string) string {
	var out strings.Builder
	quoted, escaped := false, false
	for _, r := range text {
		if quoted {
			if escaped {
				out.WriteRune(r)
				escaped = false
				continue
			}
			if r == '\\' {
				out.WriteRune(r)
				escaped = true
				continue
			}
			if r == '"' {
				out.WriteRune(r)
				quoted = false
				continue
			}
			if r < 32 {
				switch r {
				case '\b':
					out.WriteString(`\b`)
				case '\f':
					out.WriteString(`\f`)
				case '\n':
					out.WriteString(`\n`)
				case '\r':
					out.WriteString(`\r`)
				case '\t':
					out.WriteString(`\t`)
				default:
					fmt.Fprintf(&out, `\u%04x`, r)
				}
				continue
			}
			out.WriteRune(r)
			continue
		}
		if r == '"' {
			quoted = true
		}
		out.WriteRune(r)
	}
	return out.String()
}
func (s Summary) Render() string {
	var out strings.Builder
	out.WriteString("# Handoff Summary\n\n")
	list := func(items []string) {
		for _, item := range items {
			out.WriteString("- " + item + "\n")
		}
	}
	if len(s.UserIntent)+len(s.PendingTasks) > 0 {
		out.WriteString("## Goal\n")
		list(s.UserIntent)
		if len(s.PendingTasks) > 0 {
			out.WriteString("\nOpen tasks:\n")
			list(s.PendingTasks)
		}
		out.WriteByte('\n')
	}
	if len(s.UserMessages) > 0 {
		out.WriteString("## Constraints\n")
		list(s.UserMessages)
		out.WriteByte('\n')
	}
	if s.CurrentWork != nil {
		out.WriteString("## State\n" + *s.CurrentWork + "\n\n")
	}
	if len(s.ProblemSolving) > 0 {
		out.WriteString("## Decisions\n")
		list(s.ProblemSolving)
		out.WriteByte('\n')
	}
	if s.NextStep != nil {
		out.WriteString("## Next Steps\n" + *s.NextStep + "\n\n")
	}
	if len(s.Files)+len(s.ErrorsAndFixes)+len(s.TechnicalConcepts) > 0 {
		out.WriteString("## Essential Context\n")
		for _, file := range s.Files {
			out.WriteString("- ")
			if file.Path != "" {
				out.WriteString("`" + file.Path + "`")
			}
			if file.Path != "" && file.Summary != "" {
				out.WriteString(": ")
			}
			out.WriteString(file.Summary + "\n")
			if file.KeyCode != nil {
				longest, run := 0, 0
				for _, r := range *file.KeyCode {
					if r == '`' {
						run++
						longest = max(longest, run)
					} else {
						run = 0
					}
				}
				fence := strings.Repeat("`", max(3, longest+1))
				out.WriteString(fence + "\n" + strings.TrimRight(*file.KeyCode, "\n") + "\n" + fence + "\n")
			}
		}
		list(s.ErrorsAndFixes)
		list(s.TechnicalConcepts)
	}
	return strings.TrimSpace(out.String())
}

// Bound cuts the least essential sections toward limit, retaining the last
// essential fields. Parse rejects a handoff that still cannot fit.
func (s *Summary) Bound(limit uint64) {
	fits := func() bool { return tokens.Text(s.Render()) <= limit }
	if fits() {
		return
	}
	// The droppable lists can be long, and counting the whole handoff after
	// every dropped entry is quadratic, so each is cut by bisection to the
	// same length that dropping entries one at a time arrives at.
	if cut(&s.TechnicalConcepts, fits) || cut(&s.ErrorsAndFixes, fits) || cut(&s.Files, fits) || cut(&s.ProblemSolving, fits) {
		return
	}
	for !fits() {
		if !s.trim() {
			return
		}
	}
}

// cut drops the fewest trailing entries of list that make the handoff fit
// and reports whether it then fits; a list that cannot make it fit is emptied.
func cut[T any](list *[]T, fits func() bool) bool {
	full := *list
	if len(full) == 0 {
		return false
	}
	*list = full[:0]
	if !fits() {
		return false
	}
	kept, over := 0, len(full)
	for over-kept > 1 {
		middle := (kept + over) / 2
		if *list = full[:middle]; fits() {
			kept = middle
		} else {
			over = middle
		}
	}
	*list = full[:kept]
	return true
}
func (s *Summary) trim() bool {
	for _, list := range []*[]string{&s.TechnicalConcepts, &s.ErrorsAndFixes} {
		if len(*list) > 0 {
			*list = (*list)[:len(*list)-1]
			return true
		}
	}
	if len(s.Files) > 0 {
		s.Files = s.Files[:len(s.Files)-1]
		return true
	}
	if len(s.ProblemSolving) > 0 {
		s.ProblemSolving = s.ProblemSolving[:len(s.ProblemSolving)-1]
		return true
	}
	if s.shorten(500) {
		return true
	}
	for _, list := range []*[]string{&s.PendingTasks, &s.UserMessages, &s.UserIntent} {
		if len(*list) > 1 {
			*list = (*list)[:len(*list)-1]
			return true
		}
	}
	return s.shorten(48)
}
func (s *Summary) shorten(minimum uint64) bool {
	var longest *string
	var size uint64
	for _, list := range []*[]string{&s.UserIntent, &s.UserMessages, &s.PendingTasks} {
		for i := range *list {
			if current := tokens.Text((*list)[i]); current >= size {
				size = current
				longest = &(*list)[i]
			}
		}
	}
	for _, text := range []*string{s.CurrentWork, s.NextStep} {
		if text != nil {
			if current := tokens.Text(*text); current >= size {
				size = current
				longest = text
			}
		}
	}
	if longest == nil || size <= minimum {
		return false
	}
	characters := []rune(*longest)
	keep := max(1, uint64(len(characters))*max(size/2, minimum)/size)
	*longest = strings.TrimRight(string(characters[:keep]), "…") + "…"
	return true
}
