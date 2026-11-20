package providers

import (
	"encoding/json"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

const (
	defaultCheapRouterContext uint32 = 128000
	defaultCheapRouterOutput  uint32 = 16384
	maxCheapRouterTokens      uint32 = 2147483647
	maxCheapRouterModelIDLen         = 256
	maxCheapRouterConfigBytes        = 64 * 1024
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
	explicitContext := make(map[string]bool, len(gateway.Models))
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
		platform := strings.TrimSpace(item.Platform)
		if len(platform) > 64 {
			platform = ""
		}
		ctxWindow, hasCtx := normalizeCheapRouterContext(item.ContextWindow)
		if idx, exists := indexByID[id]; exists {
			existing := &provider.Models[idx]
			if existing.Platform == "" {
				existing.Platform = platform
			}
			if existing.DisplayName == nil && name != "" && name != id {
				display := name
				existing.DisplayName = &display
			}
			if hasCtx && (!explicitContext[id] || ctxWindow > existing.Context) {
				existing.Context = ctxWindow
				explicitContext[id] = true
				existing.Output = normalizeCheapRouterOutput(item.MaxOutputTokens, existing.Context, existing.Output)
			} else if item.MaxOutputTokens > 0 {
				existing.Output = normalizeCheapRouterOutput(item.MaxOutputTokens, existing.Context, existing.Output)
			}
			continue
		}

		model := protocol.DefaultConversationModel()
		model.ID = id
		model.Platform = platform
		if name != "" && name != id {
			display := name
			model.DisplayName = &display
		}
		model.Context = ctxWindow
		model.Output = normalizeCheapRouterOutput(item.MaxOutputTokens, model.Context, 0)
		model.Tools = true
		model.CompactionTokens = nil
		indexByID[id] = len(provider.Models)
		explicitContext[id] = hasCtx
		provider.Models = append(provider.Models, model)
	}
	if len(provider.Models) == 0 {
		return provider, nil, invalid("CheapRouter model catalog has no chat models")
	}
	provider.DefaultModel = provider.Models[0].ID
	fitCheapRouterConfigBudget(&provider)
	if err := ValidateConfiguration(provider); err != nil {
		return provider, nil, err
	}
	secret := protocol.NewSecret(gateway.APIKey)
	return provider, &secret, nil
}

func normalizeCheapRouterContext(raw int64) (uint32, bool) {
	if raw <= 0 {
		return defaultCheapRouterContext, false
	}
	if raw > int64(maxCheapRouterTokens) {
		return maxCheapRouterTokens, true
	}
	return uint32(raw), true
}

func normalizeCheapRouterOutput(raw int64, contextWindow uint32, fallback uint32) uint32 {
	var target uint32
	switch {
	case raw > int64(maxCheapRouterTokens):
		target = maxCheapRouterTokens
	case raw > 0:
		target = uint32(raw)
	case fallback > 0:
		target = fallback
	default:
		target = defaultCheapRouterOutput
	}
	if target > contextWindow {
		target = contextWindow
	}
	if target == 0 {
		target = 1
	}
	return target
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
