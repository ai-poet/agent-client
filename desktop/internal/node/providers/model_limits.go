package providers

import (
	"encoding/json"
	"strings"
	"sync"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
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
		id := strings.ToLower(entry.ID)
		if index.suppliers[entry.Provider] == nil {
			index.suppliers[entry.Provider] = map[string]ModelLimits{}
		}
		index.suppliers[entry.Provider][id] = limits
		for _, name := range []string{id, modelLeaf(id)} {
			previous := index.names[name]
			index.names[name] = ModelLimits{Context: max(previous.Context, limits.Context), Output: max(previous.Output, limits.Output)}
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

// CatalogModelLimits uses the models.dev snapshot, separate from the pinned
// request-policy catalog. Prefer an identified supplier, then the largest
// declaration for the same exact ID or bare name across suppliers.
func CatalogModelLimits(api protocol.ConversationModelApi, id string) ModelLimits {
	var supplier string
	switch api {
	case protocol.ConversationModelApiResponses, protocol.ConversationModelApiChatCompletions:
		supplier = "openai"
	case protocol.ConversationModelApiAnthropic:
		supplier = "anthropic"
	case protocol.ConversationModelApiGemini:
		supplier = "google"
	case protocol.ConversationModelApiAzureAi, protocol.ConversationModelApiAzureOpenAi:
		supplier = "azure"
	case protocol.ConversationModelApiDeepSeek:
		supplier = "deepseek"
	case protocol.ConversationModelApiOpenCodeGo:
		supplier = "opencode-go"
	case protocol.ConversationModelApiOpenCodeZen:
		supplier = "opencode"
	}
	return bundledLimits().lookup(supplier, id)
}

func (index catalogLimits) lookup(supplier, id string) ModelLimits {
	id = strings.ToLower(strings.TrimSpace(id))
	var resolved ModelLimits
	fill := func(candidate ModelLimits) {
		if resolved.Context == 0 {
			resolved.Context = candidate.Context
		}
		if resolved.Output == 0 {
			resolved.Output = candidate.Output
		}
	}
	if owner, bare, ok := strings.Cut(id, "/"); ok {
		fill(index.suppliers[owner][bare])
	}
	fill(index.suppliers[supplier][id])
	fill(index.names[id])
	fill(index.names[modelLeaf(id)])
	return resolved
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
