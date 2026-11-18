package providers

import (
	"errors"
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
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
	for _, item := range gateway.Models {
		id := strings.TrimSpace(item.ID)
		if id == "" || item.BillingMode != "" && item.BillingMode != "token" {
			continue
		}
		model := protocol.DefaultConversationModel()
		model.ID = id
		if name := strings.TrimSpace(item.DisplayName); name != "" {
			model.DisplayName = &name
		}
		model.Context = item.ContextWindow
		if model.Context == 0 {
			model.Context = 128000
		}
		model.Output = min(uint32(16384), model.Context)
		model.Tools = true
		provider.Models = append(provider.Models, model)
	}
	if len(provider.Models) == 0 {
		return provider, nil, errors.New("CheapRouter model catalog has no chat models")
	}
	provider.DefaultModel = provider.Models[0].ID
	secret := protocol.NewSecret(gateway.APIKey)
	return provider, &secret, nil
}
