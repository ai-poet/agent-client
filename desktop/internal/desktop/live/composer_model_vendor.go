package live

import (
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

// A gateway can serve many vendors over one API (for example DeepSeek, GLM,
// Kimi, MiniMax and Qwen over an "openai" group). Match Rust native_platform:
// infer vendor from the model ID first, falling back to catalog platform only
// when the name gives nothing away.
func composerModelVendor(p protocol.ConversationProvider, m protocol.ConversationModel) string {
	if family := cheapauth.ModelFamily(m.ID); family != "" {
		return family
	}
	switch platform := strings.ToLower(strings.TrimSpace(m.Platform)); platform {
	case "claude":
		return "anthropic"
	case "google", "google-ai", "google_ai", "google-ai-studio", "vertex", "vertex-ai":
		return "gemini"
	case "xai", "x-ai":
		return "grok"
	case "moonshot":
		return "kimi"
	case "glm", "bigmodel", "zai":
		return "zhipu"
	case "alibaba", "dashscope":
		return "qwen"
	case "azure-openai", "azure_open_ai", "codex", "chatgpt":
		return "openai"
	case "", "composite", "other", "default":
		// These routes do not identify the vendor.
	default:
		return platform
	}
	if p.ID == providers.CheapRouterProviderID {
		return "other"
	}
	return modelFamily(p.Api)
}
