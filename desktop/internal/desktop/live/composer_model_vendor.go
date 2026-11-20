package live

import (
	"strings"

	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

// A gateway can serve many vendors over one API. Prefer the catalog platform;
// composite routes and older profiles without platform metadata use the model ID.
func composerModelVendor(p protocol.ConversationProvider, m protocol.ConversationModel) string {
	switch platform := strings.ToLower(strings.TrimSpace(m.Platform)); platform {
	case "claude":
		return "anthropic"
	case "google", "google-ai", "google_ai", "vertex", "vertex-ai":
		return "gemini"
	case "xai", "x-ai":
		return "grok"
	case "moonshot":
		return "kimi"
	case "alibaba", "dashscope":
		return "qwen"
	case "azure-openai", "azure_open_ai":
		return "openai"
	case "", "composite", "other":
		// These routes do not identify the vendor.
	default:
		return platform
	}
	id := strings.ToLower(m.ID)
	id = id[strings.LastIndex(id, "/")+1:]
	if _, bare, ok := strings.Cut(id, "::"); ok {
		id = bare
	}
	switch {
	case strings.HasPrefix(id, "claude"):
		return "anthropic"
	case strings.HasPrefix(id, "gpt-"), strings.HasPrefix(id, "o1"), strings.HasPrefix(id, "o3"), strings.HasPrefix(id, "o4"):
		return "openai"
	case strings.HasPrefix(id, "gemini"):
		return "gemini"
	case strings.HasPrefix(id, "deepseek"):
		return "deepseek"
	case strings.HasPrefix(id, "grok"):
		return "grok"
	case strings.HasPrefix(id, "glm-"):
		return "zhipu"
	case strings.HasPrefix(id, "kimi"), strings.HasPrefix(id, "moonshot"):
		return "kimi"
	case strings.HasPrefix(id, "minimax"):
		return "minimax"
	case strings.HasPrefix(id, "qwen"), strings.HasPrefix(id, "qwq"):
		return "qwen"
	}
	if p.ID == providers.CheapRouterProviderID {
		return "other"
	}
	return modelFamily(p.Api)
}
