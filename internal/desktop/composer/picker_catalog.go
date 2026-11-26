package composer

import (
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
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

func providerGroupKey(m Model) string {
	if m.ProviderID != "" {
		return m.ProviderID
	}
	return m.Provider
}

func hasMultipleProviders(models []Model) bool {
	seen := ""
	hasFirst := false
	for _, m := range models {
		key := providerGroupKey(m)
		if !hasFirst {
			seen, hasFirst = key, true
			continue
		}
		if key != seen {
			return true
		}
	}
	return false
}

type providerGroup struct {
	Key            string
	Name           string
	Icon           *ui.SVG
	Tag            string
	Models         []Model
	ContainsChosen bool
	ContainsRecent bool
	Unavailable    bool
}

func groupModelsByProvider(models []Model, chosenID string, recent map[string]bool) []providerGroup {
	if len(models) == 0 {
		return nil
	}
	index := make(map[string]int)
	var groups []providerGroup
	for _, m := range models {
		key := providerGroupKey(m)
		i, ok := index[key]
		if !ok {
			name := m.Provider
			if name == "" {
				name = m.ProviderID
			}
			i = len(groups)
			index[key] = i
			groups = append(groups, providerGroup{
				Key:         key,
				Name:        name,
				Icon:        m.ProviderIcon,
				Tag:         m.ProviderTag,
				Unavailable: m.Unavailable,
			})
		}
		g := &groups[i]
		if g.Icon == nil && m.ProviderIcon != nil {
			g.Icon = m.ProviderIcon
		}
		if g.Tag == "" && m.ProviderTag != "" {
			g.Tag = m.ProviderTag
		}
		if !m.Unavailable {
			g.Unavailable = false
		}
		if chosenID != "" && m.ID == chosenID {
			g.ContainsChosen = true
		}
		if recent[m.ID] {
			g.ContainsRecent = true
		}
		g.Models = append(g.Models, m)
	}
	for i := range groups {
		g := &groups[i]
		if !g.ContainsChosen || len(g.Models) <= 1 {
			continue
		}
		if idx := slices.IndexFunc(g.Models, func(m Model) bool { return m.ID == chosenID && m.Unavailable }); idx > 0 {
			chosen := g.Models[idx]
			copy(g.Models[1:idx+1], g.Models[0:idx])
			g.Models[0] = chosen
		}
	}
	rank := func(g providerGroup) int {
		switch {
		case g.ContainsChosen:
			return 0
		case g.ContainsRecent:
			return 1
		default:
			return 2
		}
	}
	slices.SortStableFunc(groups, func(a, b providerGroup) int { return rank(a) - rank(b) })
	return groups
}

func truncateProviderGroups(groups []providerGroup, limit int) ([]providerGroup, bool) {
	if limit <= 0 {
		return groups, false
	}
	total := 0
	for _, g := range groups {
		total += len(g.Models)
	}
	if total <= limit {
		return groups, false
	}
	remaining := limit
	out := make([]providerGroup, 0, len(groups))
	for _, g := range groups {
		if remaining == 0 {
			break
		}
		if len(g.Models) > remaining {
			g.Models = g.Models[:remaining]
		}
		remaining -= len(g.Models)
		out = append(out, g)
	}
	return out, true
}

func providerSearchTerms(m Model) string {
	extra := ""
	switch strings.ToLower(strings.TrimSpace(m.ProviderTag)) {
	case "bedrock":
		extra = "aws"
	case "vertex":
		extra = "vertexai vertex_ai gcp"
	case "acp":
		extra = "plugin"
	case "local":
		extra = "localhost"
	}
	return m.ProviderTag + " " + m.ProviderID + " " + extra
}

func matchingModels(models []Model, names map[string]string, query, vendor string) ([]Model, map[string]bool) {
	terms := strings.Fields(strings.ToLower(query))
	visible := make(map[string]bool)
	var shown []Model
	for _, m := range models {
		id := vendorID(m)
		if len(terms) > 0 {
			fields := strings.ToLower(m.Name + " " + m.ID + " " + m.Provider + " " + providerSearchTerms(m) + " " + id + " " + names[id])
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
