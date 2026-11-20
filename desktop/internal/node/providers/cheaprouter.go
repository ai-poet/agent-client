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
	if strings.TrimSpace(gateway.APIKey) == "" {
		return provider, nil, invalid("CheapRouter gateway key is empty")
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
		if id == "" || len(id) > maxCheapRouterModelIDLen {
			continue
		}
		mode := strings.ToLower(strings.TrimSpace(item.BillingMode))
		if mode != "" && mode != "token" {
			continue
		}
		name := strings.TrimSpace(item.DisplayName)
		var window, output uint32
		if item.ContextWindow >= int64(MinCatalogContext) {
			window = uint32(min(item.ContextWindow, int64(MaxModelOutput)))
		}
		if item.MaxOutputTokens > 0 {
			output = uint32(min(item.MaxOutputTokens, int64(MaxModelOutput)))
		}
		if idx, exists := indexByID[id]; exists {
			existing := &provider.Models[idx]
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
		if name != "" && name != id {
			display := name
			model.DisplayName = &display
		}
		model.Context, model.Output = window, output
		model.Tools = true
		model.CompactionTokens = nil
		indexByID[id] = len(provider.Models)
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
	provider.DefaultModel = provider.Models[0].ID
	fitCheapRouterConfigBudget(&provider)
	if err := ValidateConfiguration(provider); err != nil {
		return provider, nil, err
	}
	secret := protocol.NewSecret(gateway.APIKey)
	return provider, &secret, nil
}

func fitCheapRouterConfigBudget(provider *protocol.ConversationProvider) {
	raw, err := json.Marshal(provider)
	if err != nil || len(raw) <= maxCheapRouterConfigBytes {
		return
	}
	for len(provider.Models) > 1 {
		provider.Models = provider.Models[:len(provider.Models)-1]
		if raw, err := json.Marshal(provider); err == nil && len(raw) <= maxCheapRouterConfigBytes {
			return
		}
	}
}
