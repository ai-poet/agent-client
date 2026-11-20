package providers

import (
	"errors"
	"math"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func TestCheapRouterProviderDeduplicatesAndNormalizesCatalog(t *testing.T) {
	provider, secret, err := CheapRouterProvider(cheapauth.Gateway{
		BaseURL: "https://cheaprouter.cc/v1/",
		APIKey:  "sk-test-key",
		Models: []cheapauth.Model{
			{ID: "claude-sonnet-4-6", BillingMode: "token", ContextWindow: 8192, MaxOutputTokens: 32768},
			{ID: "claude-sonnet-4-6", DisplayName: "Claude Sonnet 4.6", BillingMode: "token", ContextWindow: 200000},
			{ID: "claude-sonnet-4-6", DisplayName: "Ignored Duplicate", BillingMode: "token", ContextWindow: 64000, MaxOutputTokens: 2048},
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
		first.Context != 200000 || first.Output != 32768 || !first.Tools {
		t.Fatalf("merged duplicate model = %+v", first)
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
