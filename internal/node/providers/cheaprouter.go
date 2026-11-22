package providers

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

const (
	maxCheapRouterModelIDLen  = 256
	maxCheapRouterConfigBytes = 64 * 1024
)

// CheapRouterProviderID is stable across sign-ins and endpoint switches, so
// existing sessions keep their model selection when credentials rotate.
var CheapRouterProviderID = protocol.ProviderID{0xc0, 0xa4, 0x7a, 0x11, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01}

// CheapRouterLaneID identifies a logical lane across group switches and key
// rotations. The account/legacy ID is never reassigned to a different lane.
func CheapRouterLaneID(lane string) protocol.ProviderID {
	id := CheapRouterProviderID
	lane = strings.ToLower(strings.TrimSpace(lane))
	if canonical := cheapauth.NormalizeLane(lane); canonical != "" {
		lane = canonical
	}
	sum := sha256.Sum256([]byte(lane))
	id[6] = 0x80 // reserved managed-lane namespace, distinct from UUIDv4 IDs
	copy(id[7:], sum[:])
	id[8] = id[8]&0x3f | 0x80
	return id
}

func IsCheapRouterProvider(id protocol.ProviderID) bool {
	return id == CheapRouterProviderID || (id[6] == 0x80 && bytes.Equal(id[:6], CheapRouterProviderID[:6]))
}

// CheapRouterSuccessor is the lane provider a selection of the unbound route
// follows after lanes replaced that route and reconciliation disabled it.
// lanes is the account's routing table by lower-cased model ID; a model it does
// not list goes through its own lane, as the unbound route's key bundle
// resolved it. The successor is enabled and offers the model. A lane provider
// has no successor, a removed unbound route stays removed, and another lane
// never stands in for one that is unbound or signed out.
func CheapRouterSuccessor(provider protocol.ProviderID, model string, lanes map[string]string, current []protocol.ConversationProvider) (protocol.ConversationProvider, bool) {
	if provider != CheapRouterProviderID {
		return protocol.ConversationProvider{}, false
	}
	offers := func(p protocol.ConversationProvider) int {
		return slices.IndexFunc(p.Models, func(m protocol.ConversationModel) bool { return m.ID == model })
	}
	retired := slices.IndexFunc(current, func(p protocol.ConversationProvider) bool { return p.ID == provider })
	if retired < 0 || current[retired].Enabled {
		return protocol.ConversationProvider{}, false
	}
	lane := strings.TrimSpace(lanes[strings.ToLower(strings.TrimSpace(model))])
	if lane == "" {
		platform := ""
		if index := offers(current[retired]); index >= 0 {
			platform = current[retired].Models[index].Platform
		}
		lane = cheapauth.ModelLane(cheapauth.Model{ID: model, Platform: platform})
	}
	if lane == "" {
		return protocol.ConversationProvider{}, false
	}
	id := CheapRouterLaneID(lane)
	for _, p := range current {
		if p.ID == id && p.Enabled && offers(p) >= 0 {
			return p, true
		}
	}
	return protocol.ConversationProvider{}, false
}

var errCheapRouterNoChat = invalid("CheapRouter model catalog has no chat models")

type CheapRouterRoute struct {
	Provider protocol.ConversationProvider
	Secret   *protocol.Secret
}

func CheapRouterProviders(g cheapauth.Gateway) ([]CheapRouterRoute, error) {
	if len(g.Lanes) == 0 {
		p, secret, err := CheapRouterProvider(g)
		if err != nil {
			return nil, err
		}
		return []CheapRouterRoute{{p, secret}}, nil
	}
	root, err := Endpoint(g.BaseURL)
	if err != nil {
		return nil, err
	}
	root.Path = strings.TrimSuffix(strings.TrimRight(root.Path, "/"), "/v1")
	var routes []CheapRouterRoute
	for _, lane := range g.Lanes {
		if len(lane.Models) == 0 {
			continue
		}
		api, path := protocol.ConversationModelApiChatCompletions, "/v1/chat/completions"
		switch cheapauth.NormalizeLane(lane.Group.Platform) {
		case cheapauth.LaneClaude:
			api, path = protocol.ConversationModelApiAnthropic, "/v1/messages"
		case cheapauth.LaneGemini:
			api, path = protocol.ConversationModelApiGemini, "/v1beta"
		case cheapauth.LaneCodex:
			domestic := cheapauth.NormalizeLane(lane.Lane) == cheapauth.LaneDomestic ||
				cheapauth.GroupLane(lane.Group, lane.Models) == cheapauth.LaneDomestic
			if lane.Group.SupportsResponses == cheapauth.ResponsesSupportSupported ||
				(!domestic && lane.Group.SupportsResponses == cheapauth.ResponsesSupportUnset) {
				api, path = protocol.ConversationModelApiResponses, "/v1/responses"
			}
		case cheapauth.LaneGrok:
			if lane.Group.SupportsResponses != cheapauth.ResponsesSupportUnsupported {
				api, path = protocol.ConversationModelApiResponses, "/v1/responses"
			}
		}
		p, secret, err := cheapRouterProvider(cheapauth.Gateway{BaseURL: root.JoinPath(path).String(), APIKey: g.LaneKeys[lane.Lane], Models: lane.Models}, CheapRouterLaneID(lane.Lane), "CheapRouter - "+cheapauth.LaneDisplayName(lane.Lane), api)
		if err != nil {
			if err == errCheapRouterNoChat {
				continue
			}
			return nil, err
		}
		routes = append(routes, CheapRouterRoute{p, secret})
	}
	if len(routes) == 0 {
		return nil, errCheapRouterNoChat
	}
	return routes, nil
}

// CheapRouterProvider maps the account gateway into sailry's existing
// OpenAI-compatible provider and protected API-key credential. It adds no wire
// protocol variants and leaves all user-installed providers untouched.
func CheapRouterProvider(gateway cheapauth.Gateway) (protocol.ConversationProvider, *protocol.Secret, error) {
	if len(gateway.LaneKeys) > 0 || len(gateway.Lanes) > 0 {
		return protocol.ConversationProvider{}, nil, invalid("bound CheapRouter lanes require separate providers")
	}
	return cheapRouterProvider(gateway, CheapRouterProviderID, "CheapRouter", protocol.ConversationModelApiChatCompletions)
}

func cheapRouterProvider(gateway cheapauth.Gateway, id protocol.ProviderID, name string, api protocol.ConversationModelApi) (protocol.ConversationProvider, *protocol.Secret, error) {
	provider := protocol.DefaultConversationProvider()
	baseURL := strings.TrimRight(strings.TrimSpace(gateway.BaseURL), "/")
	if _, err := Endpoint(baseURL); err != nil {
		return provider, nil, err
	}
	key := strings.TrimSpace(gateway.APIKey)
	if key == "" {
		return provider, nil, invalid("CheapRouter gateway key is empty")
	}
	secret := protocol.NewSecret(key)
	provider.ID, provider.Name, provider.Api = id, name, api
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
			// A group can expose another vendor's model. Keep only choices
			// the bound wire protocol can actually represent.
			model.Efforts = slices.DeleteFunc(ladder, func(e protocol.Effort) bool {
				return validateEffort(api, e, model.Output) != nil
			})
			model.DefaultEffort = def
		}
		indexByID[key] = len(provider.Models)
		provider.Models = append(provider.Models, model)
	}
	if len(provider.Models) == 0 {
		return provider, nil, errCheapRouterNoChat
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
	return provider, &secret, nil
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
