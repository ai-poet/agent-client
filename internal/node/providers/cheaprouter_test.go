package providers

import (
	"encoding/json"
	"errors"
	"math"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestCheapRouterProviderDeduplicatesAndNormalizesCatalog(t *testing.T) {
	provider, secret, err := CheapRouterProvider(cheapauth.Gateway{
		BaseURL: "https://cheaprouter.cc/v1/",
		APIKey:  "sk-test-key",
		Models: []cheapauth.Model{
			{ID: "claude-sonnet-4-6", DisplayName: "", BillingMode: "token", ContextWindow: 0},
			{ID: "claude-sonnet-4-6", DisplayName: "Claude Sonnet 4.6", Platform: "anthropic", BillingMode: "token", ContextWindow: 200000, MaxOutputTokens: 32768},
			{ID: "claude-sonnet-4-6", DisplayName: "Ignored Duplicate", Platform: "composite", BillingMode: "token", ContextWindow: 64000},
			{ID: "gpt-image-1", DisplayName: "Image Gen", BillingMode: "image", ContextWindow: 0},
			{ID: "tts-1", DisplayName: "Audio", BillingMode: "per_request", ContextWindow: 0},
			{ID: "small-ctx-model", BillingMode: "", ContextWindow: 4096},
			{ID: "overflow-model", BillingMode: "TOKEN", ContextWindow: math.MaxInt64, MaxOutputTokens: math.MaxInt64},
			{ID: "negative-model", BillingMode: "token", ContextWindow: -100, MaxOutputTokens: -50},
			{ID: strings.Repeat("x", 300), BillingMode: "token", ContextWindow: 64000},
			{ID: "   ", BillingMode: "token", ContextWindow: 64000},
		},
	})
	if err != nil {
		t.Fatalf("CheapRouterProvider failed: %v", err)
	}
	if secret == nil || secret.Expose() != "sk-test-key" {
		t.Fatal("missing or wrong secret")
	}
	if err := ValidateConfiguration(provider); err != nil {
		t.Fatalf("ValidateConfiguration rejected normalized provider: %v", err)
	}
	if len(provider.Models) != 4 {
		t.Fatalf("model count = %d, want 4: %+v", len(provider.Models), provider.Models)
	}
	if provider.DefaultModel != "claude-sonnet-4-6" {
		t.Fatalf("default model = %q, want claude-sonnet-4-6", provider.DefaultModel)
	}

	first := provider.Models[0]
	if first.ID != "claude-sonnet-4-6" || first.DisplayName == nil || *first.DisplayName != "Claude Sonnet 4.6" ||
		first.Platform != "anthropic" || first.Context != 200000 || first.Output != 32768 || !first.Tools {
		t.Fatalf("merged duplicate model = %+v", first)
	}
	raw, err := json.Marshal(provider)
	if err != nil {
		t.Fatal(err)
	}
	var saved protocol.ConversationProvider
	if err := json.Unmarshal(raw, &saved); err != nil {
		t.Fatal(err)
	}
	if len(saved.Models) != len(provider.Models) || saved.Models[0].Platform != first.Platform || saved.Models[1].Platform != "" {
		t.Fatalf("catalog platform lost in provider JSON: %+v", saved.Models)
	}

	small := provider.Models[1]
	if small.ID != "small-ctx-model" || small.Context != 0 || small.Output != 0 {
		t.Fatalf("small context model = %+v", small)
	}

	overflow := provider.Models[2]
	if overflow.ID != "overflow-model" || overflow.Context != MaxModelOutput || overflow.Output != MaxModelOutput {
		t.Fatalf("overflow model = %+v", overflow)
	}

	negative := provider.Models[3]
	if negative.ID != "negative-model" || negative.Context != 0 || negative.Output != 0 {
		t.Fatalf("negative model = %+v", negative)
	}
}

func TestCheapRouterProviderRespectsConfigSizeLimit(t *testing.T) {
	models := make([]cheapauth.Model, 0, 500)
	for i := range 500 {
		models = append(models, cheapauth.Model{
			ID:            "model-" + strings.Repeat("a", 80) + "-" + string(rune('0'+i%10)) + string(rune('a'+(i/10)%26)) + string(rune('A'+(i/260)%26)),
			DisplayName:   "Display " + strings.Repeat("b", 60),
			BillingMode:   "token",
			ContextWindow: 128000,
		})
	}
	provider, _, err := CheapRouterProvider(cheapauth.Gateway{
		BaseURL: "https://cheaprouter.cc/v1",
		APIKey:  "sk-test-key",
		Models:  models,
	})
	if err != nil {
		t.Fatalf("large catalog failed: %v", err)
	}
	if err := ValidateConfiguration(provider); err != nil {
		t.Fatalf("ValidateConfiguration rejected trimmed large catalog: %v", err)
	}
	if len(provider.Models) == 0 || len(provider.Models) >= len(models) {
		t.Fatalf("expected catalog to be trimmed to fit 64KiB budget, got %d models", len(provider.Models))
	}
}

func TestCheapRouterProviderStructuredErrors(t *testing.T) {
	cases := []struct {
		name        string
		gateway     cheapauth.Gateway
		wantContain string
	}{
		{
			name:        "missing api key",
			gateway:     cheapauth.Gateway{BaseURL: "https://cheaprouter.cc/v1", APIKey: "   ", Models: []cheapauth.Model{{ID: "gpt-5"}}},
			wantContain: "gateway key is empty",
		},
		{
			name:        "only non-chat models",
			gateway:     cheapauth.Gateway{BaseURL: "https://cheaprouter.cc/v1", APIKey: "sk-key", Models: []cheapauth.Model{{ID: "img-1", BillingMode: "image"}, {ID: "vid-1", BillingMode: "per_request"}}},
			wantContain: "no chat models",
		},
		{
			name:        "invalid endpoint",
			gateway:     cheapauth.Gateway{BaseURL: "ftp://bad.example/v1", APIKey: "sk-key", Models: []cheapauth.Model{{ID: "gpt-5"}}},
			wantContain: "provider endpoint",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, _, err := CheapRouterProvider(tc.gateway)
			if err == nil {
				t.Fatalf("expected error containing %q, got nil", tc.wantContain)
			}
			var fault *protocol.Fault
			if !errors.As(err, &fault) {
				t.Fatalf("expected *protocol.Fault, got %T: %v", err, err)
			}
			if fault.Code != protocol.ErrorCodeInvalidRequest || !strings.Contains(fault.Message, tc.wantContain) {
				t.Fatalf("fault = %+v, want InvalidRequest containing %q", fault, tc.wantContain)
			}
		})
	}
}

func TestCheapRouterProvidersDefaultProtocolRouting(t *testing.T) {
	cases := []struct {
		name     string
		lane     string
		group    cheapauth.Group
		model    string
		wantAPI  protocol.ConversationModelApi
		wantPath string
	}{
		{
			name:     "claude stays anthropic",
			lane:     cheapauth.LaneClaude,
			group:    cheapauth.Group{ID: 1, Platform: "anthropic"},
			model:    "claude-sonnet-4-6",
			wantAPI:  protocol.ConversationModelApiAnthropic,
			wantPath: "/v1/messages",
		},
		{
			name:     "gemini stays gemini",
			lane:     cheapauth.LaneGemini,
			group:    cheapauth.Group{ID: 2, Platform: "gemini"},
			model:    "gemini-2.5-pro",
			wantAPI:  protocol.ConversationModelApiGemini,
			wantPath: "/v1beta",
		},
		{
			name:     "vertex alias stays gemini",
			lane:     cheapauth.LaneGemini,
			group:    cheapauth.Group{ID: 3, Platform: "vertex"},
			model:    "gemini-2.5-pro",
			wantAPI:  protocol.ConversationModelApiGemini,
			wantPath: "/v1beta",
		},
		{
			name:     "openai unset defaults to responses",
			lane:     cheapauth.LaneCodex,
			group:    cheapauth.Group{ID: 4, Platform: "openai"},
			model:    "gpt-5",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "codex alias unset defaults to responses",
			lane:     cheapauth.LaneCodex,
			group:    cheapauth.Group{ID: 5, Platform: "codex"},
			model:    "gpt-5-codex",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "azure-openai unset defaults to responses",
			lane:     cheapauth.LaneCodex,
			group:    cheapauth.Group{ID: 6, Platform: "azure-openai"},
			model:    "gpt-5",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "openai explicit false falls back to chat completions",
			lane:     cheapauth.LaneCodex,
			group:    cheapauth.Group{ID: 7, Platform: "openai", SupportsResponses: cheapauth.ResponsesSupportUnsupported},
			model:    "gpt-5",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
		{
			name:     "openai explicit true uses responses",
			lane:     cheapauth.LaneCodex,
			group:    cheapauth.Group{ID: 8, Platform: "openai", SupportsResponses: cheapauth.ResponsesSupportSupported},
			model:    "gpt-5",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "grok unset defaults to responses",
			lane:     cheapauth.LaneGrok,
			group:    cheapauth.Group{ID: 9, Platform: "grok"},
			model:    "grok-4",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "xai alias unset defaults to responses",
			lane:     cheapauth.LaneGrok,
			group:    cheapauth.Group{ID: 10, Platform: "xai"},
			model:    "grok-4",
			wantAPI:  protocol.ConversationModelApiResponses,
			wantPath: "/v1/responses",
		},
		{
			name:     "grok explicit false falls back to chat completions",
			lane:     cheapauth.LaneGrok,
			group:    cheapauth.Group{ID: 11, Platform: "grok", SupportsResponses: cheapauth.ResponsesSupportUnsupported},
			model:    "grok-4",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
		{
			name:     "domestic platform uses chat completions",
			lane:     cheapauth.LaneDomestic,
			group:    cheapauth.Group{ID: 12, Platform: "deepseek"},
			model:    "deepseek-r1",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
		{
			name:     "domestic lane on openai platform defaults to chat completions",
			lane:     cheapauth.LaneDomestic,
			group:    cheapauth.Group{ID: 13, Name: "国模高速组", Platform: "openai"},
			model:    "deepseek-r1",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
		{
			name:     "unknown platform falls back to chat completions",
			lane:     "future",
			group:    cheapauth.Group{ID: 14, Platform: "unknown-vendor"},
			model:    "future-model",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
		{
			name:     "composite platform falls back to chat completions",
			lane:     cheapauth.LaneClaude,
			group:    cheapauth.Group{ID: 15, Platform: "composite"},
			model:    "claude-alias",
			wantAPI:  protocol.ConversationModelApiChatCompletions,
			wantPath: "/v1/chat/completions",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			routes, err := CheapRouterProviders(cheapauth.Gateway{
				BaseURL:  "https://cheaprouter.cc/proxy/v1/",
				LaneKeys: map[string]string{tc.lane: "sk-lane-key"},
				Lanes: []cheapauth.GatewayLane{{
					Lane:   tc.lane,
					Group:  tc.group,
					Models: []cheapauth.Model{{ID: tc.model, ContextWindow: 128000}},
				}},
			})
			if err != nil {
				t.Fatalf("CheapRouterProviders failed: %v", err)
			}
			if len(routes) != 1 {
				t.Fatalf("routes = %d, want 1", len(routes))
			}
			got := routes[0].Provider
			wantEndpoint := "https://cheaprouter.cc/proxy" + tc.wantPath
			if got.Api != tc.wantAPI || got.Endpoint != wantEndpoint {
				t.Fatalf("got (api=%s, endpoint=%s), want (api=%s, endpoint=%s)", got.Api, got.Endpoint, tc.wantAPI, wantEndpoint)
			}
		})
	}
}
