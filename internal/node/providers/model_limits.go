package providers

import (
	"encoding/json"
	"strings"
	"sync"

	"github.com/ai-poet/agent-client/internal/protocol"
)

const (
	MinCatalogContext uint32 = 8192
	MaxModelOutput    uint32 = 2147483647
)

// ModelLimits contains declarations only; zero means the source did not
// provide a usable value. It does not include runtime estimates.
type ModelLimits struct {
	Context uint32 `json:"context"`
	Output  uint32 `json:"output"`
}

type catalogLimits struct {
	suppliers map[string]map[string]ModelLimits
	names     map[string]ModelLimits
}

func modelLeaf(id string) string { return id[strings.LastIndexByte(id, '/')+1:] }

func maxLimits(a, b ModelLimits) ModelLimits {
	return ModelLimits{Context: max(a.Context, b.Context), Output: max(a.Output, b.Output)}
}

func modelNameVariants(id string) []string {
	id = strings.ToLower(strings.TrimSpace(id))
	if id == "" {
		return nil
	}
	if _, bare, ok := strings.Cut(id, "::"); ok {
		id = strings.TrimSpace(bare)
	}
	id = strings.TrimPrefix(id, "models/")
	leaf := modelLeaf(id)
	seen := map[string]bool{}
	var out []string
	add := func(v string) {
		if v != "" && !seen[v] {
			seen[v] = true
			out = append(out, v)
		}
	}
	add(id)
	add(leaf)
	if normalized := strings.ReplaceAll(leaf, ".", "-"); normalized != leaf {
		add(normalized)
	}
	if stripped := stripDateSuffix(leaf); stripped != leaf {
		add(stripped)
		if normalized := strings.ReplaceAll(stripped, ".", "-"); normalized != stripped {
			add(normalized)
		}
	}
	return out
}

func stripDateSuffix(name string) string {
	for at := len(name) - 9; at >= 0; at-- {
		if strings.HasPrefix(name[at:], "-20") && at+9 == len(name) && isAllDigits(name[at+1:]) {
			return name[:at]
		}
	}
	return name
}

func isAllDigits(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

func indexCatalogLimits(entries []CatalogEntry) catalogLimits {
	index := catalogLimits{map[string]map[string]ModelLimits{}, map[string]ModelLimits{}}
	for _, entry := range entries {
		var model struct {
			Limit ModelLimits `json:"limit"`
		}
		if json.Unmarshal(entry.Body, &model) != nil {
			continue
		}
		limits := model.Limit
		if limits.Context < MinCatalogContext {
			limits.Context = 0
		}
		limits.Output = min(limits.Output, MaxModelOutput)
		id := strings.ToLower(strings.TrimSpace(entry.ID))
		if index.suppliers[entry.Provider] == nil {
			index.suppliers[entry.Provider] = map[string]ModelLimits{}
		}
		index.suppliers[entry.Provider][id] = maxLimits(index.suppliers[entry.Provider][id], limits)
		for _, name := range modelNameVariants(id) {
			index.names[name] = maxLimits(index.names[name], limits)
		}
	}
	return index
}

var bundledLimits = sync.OnceValue(func() catalogLimits {
	catalog, err := BundledCatalog()
	if err != nil {
		return catalogLimits{}
	}
	return indexCatalogLimits(catalog.Entries)
})

func supplierForPlatform(platform string) string {
	switch strings.ToLower(strings.TrimSpace(platform)) {
	case "anthropic", "claude":
		return "anthropic"
	case "openai", "codex", "chatgpt":
		return "openai"
	case "gemini", "google", "google-ai", "google_ai", "google-ai-studio", "vertex", "vertex-ai":
		return "google"
	case "grok", "xai", "x-ai":
		return "xai"
	case "deepseek":
		return "deepseek"
	case "zhipu", "glm", "bigmodel", "zai":
		return "zhipu"
	case "kimi", "moonshot":
		return "moonshotai"
	case "minimax":
		return "minimax"
	case "qwen", "alibaba", "dashscope":
		return "alibaba"
	}
	return ""
}

func supplierForAPI(api protocol.ConversationModelApi) string {
	switch api {
	case protocol.ConversationModelApiResponses, protocol.ConversationModelApiChatCompletions:
		return "openai"
	case protocol.ConversationModelApiAnthropic:
		return "anthropic"
	case protocol.ConversationModelApiGemini:
		return "google"
	case protocol.ConversationModelApiAzureAi, protocol.ConversationModelApiAzureOpenAi:
		return "azure"
	case protocol.ConversationModelApiDeepSeek:
		return "deepseek"
	case protocol.ConversationModelApiOpenCodeGo:
		return "opencode-go"
	case protocol.ConversationModelApiOpenCodeZen:
		return "opencode"
	}
	return ""
}

// CatalogModelLimits uses the models.dev snapshot, separate from the pinned
// request-policy catalog. Same-name entries across suppliers take the maximum,
// and only contexts >= MinCatalogContext (8192) are accepted.
func CatalogModelLimits(api protocol.ConversationModelApi, id string) ModelLimits {
	return CatalogModelLimitsForPlatform(api, "", id)
}

// CatalogModelLimitsForPlatform resolves limits from the models.dev snapshot
// using platform when known (e.g. multi-vendor gateways) and taking the
// maximum across same-name entries.
func CatalogModelLimitsForPlatform(api protocol.ConversationModelApi, platform, id string) ModelLimits {
	supplier := supplierForPlatform(platform)
	if supplier == "" {
		supplier = supplierForAPI(api)
	}
	return bundledLimits().lookup(supplier, id)
}

func (index catalogLimits) lookup(supplier, id string) ModelLimits {
	id = strings.ToLower(strings.TrimSpace(id))
	var resolved ModelLimits
	merge := func(candidate ModelLimits) {
		if candidate.Context < MinCatalogContext {
			candidate.Context = 0
		}
		candidate.Output = min(candidate.Output, MaxModelOutput)
		resolved = maxLimits(resolved, candidate)
	}
	if owner, bare, ok := strings.Cut(id, "/"); ok {
		merge(index.suppliers[owner][bare])
	}
	if supplier != "" {
		merge(index.suppliers[supplier][id])
		merge(index.suppliers[supplier][modelLeaf(id)])
	}
	for _, variant := range modelNameVariants(id) {
		merge(index.names[variant])
	}
	return resolved
}

// ResolveModelLimits is the single source of truth for context and output
// limits shared by compaction thresholds, UI displays and request parameters.
// Priority: catalog/declared (>= 8192) -> models.dev registry (max by name, >= 8192)
// -> probed result (>= 8192). Never substitutes 128k for 0 or small values;
// returns estimated=true when no credible context >= 8192 is available.
func ResolveModelLimits(api protocol.ConversationModelApi, model protocol.ConversationModel, probed ModelLimits) (limits ModelLimits, estimated bool) {
	if model.Context >= MinCatalogContext {
		limits.Context = min(model.Context, MaxModelOutput)
	}
	if model.Output > 0 {
		limits.Output = min(model.Output, MaxModelOutput)
	}
	if limits.Context == 0 || limits.Output == 0 {
		registered := CatalogModelLimitsForPlatform(api, model.Platform, model.ID)
		if limits.Context == 0 && registered.Context >= MinCatalogContext {
			limits.Context = registered.Context
		}
		if limits.Output == 0 && registered.Output > 0 {
			limits.Output = registered.Output
		}
	}
	if limits.Context == 0 && probed.Context >= MinCatalogContext {
		limits.Context = min(probed.Context, MaxModelOutput)
	}
	if limits.Output == 0 && probed.Output > 0 {
		limits.Output = min(probed.Output, MaxModelOutput)
	}
	estimated = limits.Context < MinCatalogContext
	if estimated {
		limits.Context = 0
	} else {
		limits.Output = min(limits.Output, limits.Context, MaxModelOutput)
	}
	return limits, estimated
}

// ApplyDiscoveredLimits keeps missing or implausible server declarations
// absent so the execution path can use the registry or mark an estimate.
func ApplyDiscoveredLimits(model *protocol.ConversationModel, found protocol.ConversationDiscoveryModel) {
	if found.Context != nil && *found.Context >= MinCatalogContext {
		model.Context = *found.Context
	}
	if found.Output != nil {
		model.Output = min(*found.Output, MaxModelOutput)
	}
	if model.Context > 0 {
		model.Output = min(model.Output, model.Context)
	}
}
