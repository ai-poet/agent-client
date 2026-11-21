package cheapauth

import (
	"slices"
	"strings"
)

// Canonical CheapRouter routing lanes, matching the Rust sub2api model_routing
// lanes: Claude (anthropic), Codex (openai), Gemini, domestic Chinese models
// (DeepSeek/GLM/Kimi/MiniMax/Qwen), and Grok.
const (
	LaneClaude   = "anthropic"
	LaneCodex    = "openai"
	LaneGemini   = "gemini"
	LaneDomestic = "domestic"
	LaneGrok     = "grok"
)

// CanonicalLanes is the stable presentation order for known routing lanes.
var CanonicalLanes = []string{
	LaneClaude,
	LaneCodex,
	LaneGemini,
	LaneDomestic,
	LaneGrok,
}

var domesticNameMarkers = []string{"国模", "国产模型"}

// LaneDisplayName returns the human label shown in the account popover and settings.
func LaneDisplayName(lane string) string {
	switch NormalizeLane(lane) {
	case LaneClaude:
		return "Claude"
	case LaneCodex:
		return "Codex"
	case LaneGemini:
		return "Gemini"
	case LaneDomestic:
		return "国模"
	case LaneGrok:
		return "Grok"
	}
	trimmed := strings.TrimSpace(lane)
	if trimmed == "" {
		return "其他"
	}
	return strings.ToUpper(trimmed[:1]) + trimmed[1:]
}

// NormalizeLane maps a gateway platform/provider identifier onto its routing lane.
// Returns "" for composite/unknown/empty identifiers.
func NormalizeLane(platform string) string {
	switch strings.ToLower(strings.TrimSpace(platform)) {
	case "", "composite", "other", "default":
		return ""
	case "anthropic", "claude":
		return LaneClaude
	case "openai", "codex", "chatgpt", "azure-openai", "azure_open_ai":
		return LaneCodex
	case "gemini", "google", "google-ai", "google_ai", "google-ai-studio", "vertex", "vertex-ai":
		return LaneGemini
	case "grok", "xai", "x-ai":
		return LaneGrok
	case "domestic", "deepseek", "kimi", "moonshot", "zhipu", "glm", "bigmodel", "zai", "minimax", "qwen", "alibaba", "dashscope":
		return LaneDomestic
	default:
		return strings.ToLower(strings.TrimSpace(platform))
	}
}

// ModelFamily derives the vendor family from a model ID using the same rules as
// Rust sub2api::model_routing::model_family (plus Qwen).
func ModelFamily(model string) string {
	name := strings.ToLower(strings.TrimSpace(model))
	if _, bare, ok := strings.Cut(name, "::"); ok {
		name = strings.TrimSpace(bare)
	}
	name = strings.TrimPrefix(name, "models/")
	if provider, rest, ok := strings.Cut(name, "/"); ok {
		switch strings.TrimSpace(provider) {
		case "anthropic", "claude":
			return "anthropic"
		case "openai", "chatgpt":
			return "openai"
		case "google", "google-ai", "google_ai", "google-ai-studio", "vertex", "vertex-ai", "gemini":
			return "gemini"
		case "xai", "x-ai", "grok":
			return "grok"
		case "kimi", "moonshot":
			return "kimi"
		case "zhipu", "glm", "bigmodel", "zai":
			return "zhipu"
		case "deepseek":
			return "deepseek"
		case "minimax":
			return "minimax"
		case "qwen", "alibaba", "dashscope":
			return "qwen"
		}
		if rest = strings.TrimPrefix(strings.TrimSpace(rest), "models/"); rest != "" {
			name = rest
		}
	}
	if strings.HasPrefix(name, "claude-") || name == "claude" || strings.HasPrefix(name, "anthropic.claude-") {
		return "anthropic"
	}
	if strings.HasPrefix(name, "gpt-") || strings.HasPrefix(name, "gpt5") ||
		strings.HasPrefix(name, "chatgpt-") || strings.HasPrefix(name, "codex-") ||
		isOpenAISeries(name) {
		return "openai"
	}
	if strings.HasPrefix(name, "gemini-") || strings.HasPrefix(name, "learnlm-") {
		return "gemini"
	}
	if name == "grok" || strings.HasPrefix(name, "grok-") {
		return "grok"
	}
	if name == "k3" || name == "k3-256k" || strings.HasPrefix(name, "kimi-") || strings.HasPrefix(name, "moonshot-") {
		return "kimi"
	}
	if strings.HasPrefix(name, "glm-") {
		return "zhipu"
	}
	if strings.HasPrefix(name, "deepseek-") {
		return "deepseek"
	}
	if strings.HasPrefix(name, "minimax-") || strings.HasPrefix(name, "abab5") ||
		strings.HasPrefix(name, "abab6") || strings.HasPrefix(name, "abab7") {
		return "minimax"
	}
	if strings.HasPrefix(name, "qwen") || strings.HasPrefix(name, "qwq") {
		return "qwen"
	}
	return ""
}

func isOpenAISeries(name string) bool {
	for _, prefix := range []string{"o1", "o3", "o4", "o5"} {
		if name == prefix || strings.HasPrefix(name, prefix+"-") {
			return true
		}
	}
	return false
}

// IsDomesticModel reports whether model belongs to the domestic Chinese model lane
// (DeepSeek, GLM/Zhipu, Kimi/Moonshot, MiniMax, Qwen).
func IsDomesticModel(model string) bool {
	switch ModelFamily(model) {
	case "deepseek", "zhipu", "kimi", "minimax", "qwen":
		return true
	}
	return false
}

// NamedDomestic reports whether a group name or description marks it as a
// domestic Chinese model group.
func NamedDomestic(text string) bool {
	for _, marker := range domesticNameMarkers {
		if strings.Contains(text, marker) {
			return true
		}
	}
	return false
}

// ModelLane returns the routing lane a catalog model belongs to.
func ModelLane(m Model) string {
	switch ModelFamily(m.ID) {
	case "anthropic":
		return LaneClaude
	case "openai":
		return LaneCodex
	case "gemini":
		return LaneGemini
	case "grok":
		return LaneGrok
	case "deepseek", "zhipu", "kimi", "minimax", "qwen":
		return LaneDomestic
	}
	if lane := NormalizeLane(m.Platform); lane != "" {
		return lane
	}
	return NormalizeLane(m.Provider)
}

// GroupLane classifies a CheapRouter routing group into its lane, porting Rust
// sub2api::model_routing::group_lane. Live domestic groups frequently sit on
// the "openai" platform, so group names and catalog entries override platform.
func GroupLane(group Group, catalog []Model) string {
	platform := NormalizeLane(group.Platform)
	if platform == LaneDomestic || NamedDomestic(group.Name) || NamedDomestic(group.Description) {
		return LaneDomestic
	}
	if group.ID > 0 && len(catalog) > 0 {
		var listed []Model
		for _, item := range catalog {
			if strings.TrimSpace(item.ID) == "" {
				continue
			}
			if item.BestGroup.ID == group.ID {
				listed = append(listed, item)
			}
		}
		if len(listed) == 0 {
			for _, item := range catalog {
				if strings.TrimSpace(item.ID) != "" && item.ServesGroup(group.ID) {
					listed = append(listed, item)
				}
			}
		}
		if len(listed) > 0 {
			allDomestic := true
			singleLane := ""
			sameLane := true
			for _, item := range listed {
				lane := ModelLane(item)
				if !IsDomesticModel(item.ID) && lane != LaneDomestic {
					allDomestic = false
				}
				if lane == "" {
					sameLane = false
				} else if singleLane == "" {
					singleLane = lane
				} else if singleLane != lane {
					sameLane = false
				}
			}
			if allDomestic {
				return LaneDomestic
			}
			if platform == "" && sameLane && singleLane != "" {
				return singleLane
			}
		}
	}
	if platform != "" {
		return platform
	}
	return strings.ToLower(strings.TrimSpace(group.Platform))
}

// AvailableLanes returns the lanes present in groups or active bindings in a
// stable canonical order.
func AvailableLanes(groups []Group, catalog []Model, bindings map[string]int64) []string {
	present := map[string]bool{}
	for _, g := range groups {
		if lane := GroupLane(g, catalog); lane != "" {
			present[lane] = true
		}
	}
	for lane, gid := range bindings {
		if gid > 0 {
			if normalized := NormalizeLane(lane); normalized != "" {
				present[normalized] = true
			} else if trimmed := strings.ToLower(strings.TrimSpace(lane)); trimmed != "" {
				present[trimmed] = true
			}
		}
	}
	var lanes []string
	for _, known := range CanonicalLanes {
		if present[known] {
			lanes = append(lanes, known)
			delete(present, known)
		}
	}
	var extra []string
	for lane := range present {
		extra = append(extra, lane)
	}
	slices.Sort(extra)
	return append(lanes, extra...)
}

// GroupsForLane returns the subset of groups assigned to lane.
func GroupsForLane(lane string, groups []Group, catalog []Model) []Group {
	target := NormalizeLane(lane)
	if target == "" {
		target = strings.ToLower(strings.TrimSpace(lane))
	}
	var matched []Group
	for _, g := range groups {
		if GroupLane(g, catalog) == target {
			matched = append(matched, g)
		}
	}
	return matched
}

// GroupRoutable reports whether group is usable for lane binding (positive ID
// and active, enabled, or unspecified status).
func GroupRoutable(group Group) bool {
	if group.ID <= 0 {
		return false
	}
	status := strings.TrimSpace(group.Status)
	return status == "" || strings.EqualFold(status, "active") || strings.EqualFold(status, "enabled")
}

// FirstRoutableGroupForLane returns the first routable group assigned to lane
// in the directory order returned by /groups/available.
func FirstRoutableGroupForLane(lane string, groups []Group, catalog []Model) (Group, bool) {
	for _, g := range GroupsForLane(lane, groups, catalog) {
		if GroupRoutable(g) {
			return g, true
		}
	}
	return Group{}, false
}

// SelectCatalogForBindings builds the union of models covered by all bound
// lanes and records the winning lane for each model ID (lowercased).
// When bindings is empty, the full catalog is returned with nil modelLanes so
// the account default key handles all models.
func SelectCatalogForBindings(catalog []Model, bindings map[string]int64, groups []Group) ([]Model, map[string]string) {
	active := make(map[string]int64, len(bindings))
	for lane, gid := range bindings {
		if gid <= 0 {
			continue
		}
		key := NormalizeLane(lane)
		if key == "" {
			key = strings.ToLower(strings.TrimSpace(lane))
		}
		if key != "" {
			active[key] = gid
		}
	}
	if len(active) == 0 || len(catalog) == 0 {
		return catalog, nil
	}

	groupMeta := make(map[int64]Group, len(groups))
	for _, g := range groups {
		groupMeta[g.ID] = g
	}
	listingGroups := map[int64]bool{}
	for _, item := range catalog {
		if item.BestGroup.ID > 0 {
			listingGroups[item.BestGroup.ID] = true
		}
		for _, other := range item.OtherGroups {
			if other.Group.ID > 0 {
				listingGroups[other.Group.ID] = true
			}
		}
	}

	var order []string
	byModel := make(map[string][]Model, len(catalog))
	for _, item := range catalog {
		id := strings.TrimSpace(item.ID)
		if id == "" {
			continue
		}
		key := strings.ToLower(id)
		if _, seen := byModel[key]; !seen {
			order = append(order, key)
		}
		byModel[key] = append(byModel[key], item)
	}

	type laneCandidate struct {
		lane           string
		groupID        int64
		item           Model
		ownLane        bool
		explicit       bool
		ownPlatform    bool
		composite      bool
		rateMultiplier float64
	}
	better := func(a, b laneCandidate) bool {
		if a.ownLane != b.ownLane {
			return a.ownLane
		}
		if a.explicit != b.explicit {
			return a.explicit
		}
		if a.ownPlatform != b.ownPlatform {
			return a.ownPlatform
		}
		if a.composite != b.composite {
			return a.composite
		}
		if a.rateMultiplier != b.rateMultiplier {
			return a.rateMultiplier < b.rateMultiplier
		}
		if a.groupID != b.groupID {
			return a.groupID < b.groupID
		}
		return a.lane < b.lane
	}

	var selected []Model
	modelLanes := make(map[string]string, len(order))
	for _, key := range order {
		entries := byModel[key]
		modelLane := ModelLane(entries[0])
		modelFamily := ModelFamily(entries[0].ID)

		var best *laneCandidate
		var maxContext, maxOutput int64
		var displayName, bestPlatform string
		for _, entry := range entries {
			if entry.ContextWindow > maxContext {
				maxContext = entry.ContextWindow
			}
			if entry.MaxOutputTokens > maxOutput {
				maxOutput = entry.MaxOutputTokens
			}
			if displayName == "" && strings.TrimSpace(entry.DisplayName) != "" {
				displayName = strings.TrimSpace(entry.DisplayName)
			}
			if bestPlatform == "" {
				if p := strings.TrimSpace(entry.Platform); p != "" && !strings.EqualFold(p, "composite") {
					bestPlatform = p
				} else if p := strings.TrimSpace(entry.Provider); p != "" && !strings.EqualFold(p, "composite") {
					bestPlatform = p
				}
			}
		}

		for lane, groupID := range active {
			var matchedEntry *Model
			explicit := false
			rate := 0.0
			for i := range entries {
				if entries[i].ServesGroup(groupID) {
					matchedEntry = &entries[i]
					explicit = true
					if entries[i].BestGroup.ID == groupID {
						rate = entries[i].BestGroup.RateMultiplier
					} else {
						for _, other := range entries[i].OtherGroups {
							if other.Group.ID == groupID {
								rate = other.Group.RateMultiplier
								break
							}
						}
					}
					break
				}
			}
			if !explicit {
				// Silent bound group (no explicit catalog entries): trust it for its own lane.
				if listingGroups[groupID] || modelLane != lane {
					continue
				}
				matchedEntry = &entries[0]
				if g, ok := groupMeta[groupID]; ok {
					rate = g.RateMultiplier
				}
			}
			groupPlatform := strings.ToLower(strings.TrimSpace(matchedEntry.Platform))
			if g, ok := groupMeta[groupID]; ok && strings.TrimSpace(g.Platform) != "" {
				groupPlatform = strings.ToLower(strings.TrimSpace(g.Platform))
				if rate == 0 {
					rate = g.RateMultiplier
				}
			}
			candidate := laneCandidate{
				lane:           lane,
				groupID:        groupID,
				item:           *matchedEntry,
				ownLane:        modelLane != "" && modelLane == lane,
				explicit:       explicit,
				ownPlatform:    modelFamily != "" && groupPlatform == modelFamily,
				composite:      groupPlatform == "composite",
				rateMultiplier: rate,
			}
			if best == nil || better(candidate, *best) {
				copied := candidate
				best = &copied
			}
		}
		if best == nil {
			continue
		}
		chosen := best.item
		if displayName != "" && strings.TrimSpace(chosen.DisplayName) == "" {
			chosen.DisplayName = displayName
		}
		if bestPlatform != "" && (strings.TrimSpace(chosen.Platform) == "" || strings.EqualFold(strings.TrimSpace(chosen.Platform), "composite")) {
			chosen.Platform = bestPlatform
		}
		if maxContext > chosen.ContextWindow {
			chosen.ContextWindow = maxContext
		}
		if maxOutput > chosen.MaxOutputTokens {
			chosen.MaxOutputTokens = maxOutput
		}
		selected = append(selected, chosen)
		modelLanes[key] = best.lane
	}

	if len(selected) == 0 {
		return catalog, nil
	}
	return selected, modelLanes
}
