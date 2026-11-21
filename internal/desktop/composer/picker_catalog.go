package composer

import (
	"slices"
	"strings"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/icons"
	"github.com/egoist/mygo/ui"
)

const recentVendor = "#recent"

// DefaultModel picks Sonnet 5.5, then another Sonnet 5, then any Sonnet,
// and otherwise leaves the catalog order intact.
func DefaultModel(models []Model) Model {
	var best Model
	rank := 4
	for _, m := range models {
		id := strings.ReplaceAll(strings.ToLower(m.ID), ".", "-")
		id = id[strings.LastIndex(id, "/")+1:]
		if _, bare, ok := strings.Cut(id, "::"); ok {
			id = bare
		}
		name := strings.ToLower(m.Name)
		preference := 3
		switch {
		case id == "claude-sonnet-5-5" || strings.HasPrefix(id, "claude-sonnet-5-5-"):
			preference = 0
		case strings.Contains(id, "sonnet-5"):
			preference = 1
		case strings.Contains(id, "sonnet") || strings.Contains(name, "claude sonnet"):
			preference = 2
		}
		if preference < rank {
			best, rank = m, preference
			if rank == 0 {
				break
			}
		}
	}
	return best
}

// DefaultModelForFamily prefers a model in preferredFamily (still preferring
// Sonnet within the Anthropic family, else first available in that family),
// falling back to DefaultModel across all models when none matches.
func DefaultModelForFamily(models []Model, preferredFamily string) Model {
	if preferredFamily != "" {
		var same []Model
		for _, m := range models {
			if vendorID(m) == preferredFamily {
				same = append(same, m)
			}
		}
		if len(same) > 0 {
			return DefaultModel(same)
		}
	}
	return DefaultModel(models)
}

func vendorID(m Model) string {
	if m.Family == "" {
		return "other"
	}
	return m.Family
}

var vendorOrder = []string{"anthropic", "openai", "gemini", "grok", "deepseek", "zhipu", "kimi", "minimax", "qwen"}

func pickerVendors(c *ui.Context, models []Model, overrides []Family) ([]Family, map[string]string) {
	var vendors []Family
	names := make(map[string]string)
	for _, m := range models {
		id := vendorID(m)
		if _, ok := names[id]; ok {
			continue
		}
		f := Family{ID: id, Label: "provider_" + id}
		if i := slices.IndexFunc(overrides, func(f Family) bool { return f.ID == id }); i >= 0 {
			f = overrides[i]
		}
		if f.Icon == nil && id != "other" {
			f.Icon = icons.Brand(id)
		}
		if f.Icon == nil {
			f.Icon = icons.Settings2
		}
		name := i18n.T(c, f.Label)
		switch {
		case id == "gemini":
			name = "Google"
		case id == "grok":
			name = "xAI"
		case id == "deepseek":
			name = "DeepSeek"
		case id == "zhipu":
			name = "Zhipu"
		case id == "kimi":
			name = "Kimi"
		case id == "minimax":
			name = "MiniMax"
		case id == "qwen":
			name = "Qwen"
		case name == f.Label:
			name = id
		}
		names[id] = name
		vendors = append(vendors, f)
	}
	order := func(id string) int {
		if i := slices.Index(vendorOrder, id); i >= 0 {
			return i
		}
		if id == "other" {
			return len(vendorOrder) + 1
		}
		return len(vendorOrder)
	}
	slices.SortStableFunc(vendors, func(a, b Family) int { return order(a.ID) - order(b.ID) })
	return vendors, names
}

func matchingModels(models []Model, names map[string]string, query, vendor string) ([]Model, map[string]bool) {
	terms := strings.Fields(strings.ToLower(query))
	visible := make(map[string]bool)
	var shown []Model
	for _, m := range models {
		id := vendorID(m)
		if len(terms) > 0 {
			fields := strings.ToLower(m.Name + " " + m.ID + " " + m.Provider + " " + id + " " + names[id])
			if slices.ContainsFunc(terms, func(term string) bool { return !strings.Contains(fields, term) }) {
				continue
			}
		}
		visible[id] = true
		if len(terms) > 0 || vendor == id {
			shown = append(shown, m)
		}
	}
	return shown, visible
}
