package providers

import (
	"encoding/json"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

const (
	maxCheapRouterModelIDLen  = 256
	maxCheapRouterConfigBytes = 64 * 1024
)

// CheapRouterProviderID is stable across sign-ins and endpoint switches, so
// existing sessions keep their model selection when credentials rotate.
var CheapRouterProviderID = protocol.ProviderID{0xc0, 0xa4, 0x7a, 0x11, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01}

// CheapRouterProvider maps the account gateway into sailry's existing
// OpenAI-compatible provider and protected API-key credential. It adds no wire
// protocol variants and leaves all user-installed providers untouched.
func CheapRouterProvider(gateway cheapauth.Gateway) (protocol.ConversationProvider, *protocol.Secret, error) {
	provider := protocol.DefaultConversationProvider()
	baseURL := strings.TrimRight(strings.TrimSpace(gateway.BaseURL), "/")
	if _, err := Endpoint(baseURL); err != nil {
		return provider, nil, err
	}
	secret, err := EncodeCheapRouterSecret(gateway)
	if err != nil {
		return provider, nil, err
	}
	provider.ID = CheapRouterProviderID
	provider.Name = "CheapRouter"
	provider.Api = protocol.ConversationModelApiChatCompletions
	provider.Authentication = protocol.AuthenticationApiKey
	provider.Endpoint = baseURL
	provider.Enabled = true

	indexByID := make(map[string]int, len(gateway.Models))
	for _, item := range gateway.Models {
		id := strings.TrimSpace(item.ID)
		if id == "" || len(id) > maxCheapRouterModelIDLen || isNonConversationalCheapRouterModel(id) || cheapauth.IsImageCatalogModel(item) {
			continue
		}
		mode := strings.ToLower(strings.TrimSpace(item.BillingMode))
		if mode != "" && mode != "token" {
			continue
		}
		name := strings.TrimSpace(item.DisplayName)
		platform := normalizeCheapRouterPlatform(item)
		var window, output uint32
		if item.ContextWindow >= int64(MinCatalogContext) {
			window = uint32(min(item.ContextWindow, int64(MaxModelOutput)))
		}
		if item.MaxOutputTokens > 0 {
			output = uint32(min(item.MaxOutputTokens, int64(MaxModelOutput)))
		}
		key := strings.ToLower(id)
		if idx, exists := indexByID[key]; exists {
			existing := &provider.Models[idx]
			if existing.Platform == "" || strings.EqualFold(existing.Platform, "composite") {
				if platform != "" && !strings.EqualFold(platform, "composite") {
					existing.Platform = platform
				} else if existing.Platform == "" {
					existing.Platform = platform
				}
			}
			if existing.DisplayName == nil && name != "" && name != id {
				display := name
				existing.DisplayName = &display
			}
			existing.Context = max(existing.Context, window)
			existing.Output = max(existing.Output, output)
			continue
		}

		model := protocol.DefaultConversationModel()
		model.ID = id
		model.Platform = platform
		if name != "" && name != id {
			display := name
			model.DisplayName = &display
		}
		model.Context, model.Output = window, output
		model.Tools = true
		model.CompactionTokens = nil
		if ladder, def, ok := CheapRouterReasoningLadder(platform, id); ok {
			model.Reasoning = true
			model.Efforts = ladder
			model.DefaultEffort = def
		}
		indexByID[key] = len(provider.Models)
		provider.Models = append(provider.Models, model)
	}
	if len(provider.Models) == 0 {
		return provider, nil, invalid("CheapRouter model catalog has no chat models")
	}
	for i := range provider.Models {
		model := &provider.Models[i]
		if model.Context > 0 {
			model.Output = min(model.Output, model.Context)
		}
	}
	provider.DefaultModel = defaultCheapRouterModel(provider.Models)
	fitCheapRouterConfigBudget(&provider)
	if err := ValidateConfiguration(provider); err != nil {
		return provider, nil, err
	}
	return provider, secret, nil
}

func normalizeCheapRouterPlatform(item cheapauth.Model) string {
	if family := cheapauth.ModelFamily(item.ID); family != "" {
		return family
	}
	for _, candidate := range []string{item.Platform, item.Provider} {
		value := strings.TrimSpace(candidate)
		if value != "" && len(value) <= 64 {
			return value
		}
	}
	return ""
}

// isNonConversationalCheapRouterModel mirrors Rust native_agent::is_non_conversational_name.
func isNonConversationalCheapRouterModel(modelID string) bool {
	name := strings.ToLower(strings.TrimSpace(modelID))
	name = name[strings.LastIndex(name, "/")+1:]
	for _, prefix := range []string{"gpt-image", "dall-e", "sora", "tts-", "gpt-4o-mini-tts", "whisper", "grok-image", "grok-imagine", "grok-2-image", "imagen-", "flux-"} {
		if strings.HasPrefix(name, prefix) {
			return true
		}
	}
	for _, word := range strings.FieldsFunc(name, func(r rune) bool {
		return r == '-' || r == '.' || r == '/' || r == ':'
	}) {
		switch word {
		case "embedding", "embeddings", "moderation", "rerank":
			return true
		}
	}
	return false
}

// defaultCheapRouterModel follows Rust native_agent::default_row: Sonnet 5.5,
// then Sonnet 5, then any Sonnet, else the first available model.
func defaultCheapRouterModel(models []protocol.ConversationModel) string {
	if len(models) == 0 {
		return ""
	}
	best := models[0].ID
	rank := 4
	for _, m := range models {
		id := strings.ReplaceAll(strings.ToLower(strings.TrimSpace(m.ID)), ".", "-")
		id = id[strings.LastIndex(id, "/")+1:]
		if _, bare, ok := strings.Cut(id, "::"); ok {
			id = bare
		}
		display := ""
		if m.DisplayName != nil {
			display = strings.ToLower(*m.DisplayName)
		}
		preference := 3
		switch {
		case id == "claude-sonnet-5-5" || strings.HasPrefix(id, "claude-sonnet-5-5-"):
			preference = 0
		case strings.Contains(id, "sonnet-5"):
			preference = 1
		case strings.Contains(id, "sonnet") || strings.Contains(display, "claude sonnet"):
			preference = 2
		}
		if preference < rank {
			best, rank = m.ID, preference
			if rank == 0 {
				break
			}
		}
	}
	return best
}

func fitCheapRouterConfigBudget(provider *protocol.ConversationProvider) {
	raw, err := json.Marshal(provider)
	if err != nil || len(raw) <= maxCheapRouterConfigBytes {
		return
	}
	for len(provider.Models) > 1 {
		drop := len(provider.Models) - 1
		if provider.Models[drop].ID == provider.DefaultModel && drop > 0 {
			provider.Models[drop-1], provider.Models[drop] = provider.Models[drop], provider.Models[drop-1]
		}
		provider.Models = provider.Models[:len(provider.Models)-1]
		if raw, err := json.Marshal(provider); err == nil && len(raw) <= maxCheapRouterConfigBytes {
			return
		}
	}
	if len(provider.Models) > 0 {
		provider.DefaultModel = provider.Models[0].ID
	}
}
