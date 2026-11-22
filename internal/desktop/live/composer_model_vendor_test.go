package live

import (
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestComposerModelVendor(t *testing.T) {
	gateway := protocol.ConversationProvider{ID: providers.CheapRouterProviderID, Api: protocol.ConversationModelApiChatCompletions}
	for _, tc := range []struct{ platform, model, want string }{
		{"anthropic", "deployment-a", "anthropic"},
		{"google", "deployment-b", "gemini"},
		{"openai", "deepseek-v4.1-flash", "deepseek"},
		{"openai", "glm-5", "zhipu"},
		{"openai", "kimi-k3", "kimi"},
		{"openai", "qwen3-coder", "qwen"},
		{"openai", "gpt-5", "openai"},
		{"composite", "owner/deepseek-v4", "deepseek"},
		{"", "claude-sonnet-4-6", "anthropic"},
		{"", "unrecognized-alias", "other"},
	} {
		if got := composerModelVendor(gateway, protocol.ConversationModel{ID: tc.model, Platform: tc.platform}); got != tc.want {
			t.Errorf("%q model %q vendor = %q, want %q", tc.platform, tc.model, got, tc.want)
		}
	}
}
