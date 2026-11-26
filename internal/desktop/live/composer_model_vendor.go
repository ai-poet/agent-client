package live

import (
	"net/url"
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
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
	if providers.IsCheapRouterProvider(p.ID) {
		return "other"
	}
	if p.Api == protocol.ConversationModelApiChatCompletions || p.Api == protocol.ConversationModelApiResponses {
		if isOfficialOpenAIProvider(p) {
			return "openai"
		}
		return "other"
	}
	return modelFamily(p.Api)
}

func isOfficialOpenAIProvider(p protocol.ConversationProvider) bool {
	if p.Authentication == protocol.AuthenticationChatGpt || p.Authentication == protocol.AuthenticationChatGptOAuth {
		return true
	}
	if p.Preset != nil {
		switch strings.ToLower(p.Preset.ID) {
		case "openai", "chat-gpt", "chat-gpt-oauth":
			return true
		default:
			return false
		}
	}
	return isOfficialOpenAIEndpoint(p.Endpoint)
}

func isOfficialOpenAIEndpoint(endpoint string) bool {
	u, err := url.Parse(strings.TrimSpace(endpoint))
	if err != nil {
		return false
	}
	host := strings.ToLower(u.Hostname())
	return host == "api.openai.com" || strings.HasSuffix(host, ".openai.com") ||
		host == "chatgpt.com" || strings.HasSuffix(host, ".chatgpt.com")
}

func isLocalProvider(p protocol.ConversationProvider) bool {
	if p.Preset != nil {
		switch strings.ToLower(p.Preset.ID) {
		case "ollama", "lmstudio", "lm-studio", "vllm":
			return true
		}
	}
	u, err := url.Parse(strings.TrimSpace(p.Endpoint))
	if err != nil {
		return false
	}
	host := strings.ToLower(u.Hostname())
	return host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0" || strings.HasSuffix(host, ".local")
}

func providerIcon(p protocol.ConversationProvider) *ui.SVG {
	if providers.IsCheapRouterProvider(p.ID) {
		return icons.SailryMark
	}
	if p.Preset != nil {
		if own := icons.Brand(p.Preset.ID); own != nil {
			return own
		}
	}
	if mark := icons.Brand(apiBrands[p.Api]); mark != nil {
		return mark
	}
	if p.Api == protocol.ConversationModelApiPlugin {
		return icons.Plug
	}
	return nil
}

func providerTag(p protocol.ConversationProvider) string {
	switch p.Api {
	case protocol.ConversationModelApiBedrock:
		return "Bedrock"
	case protocol.ConversationModelApiVertexAi:
		return "Vertex"
	case protocol.ConversationModelApiAzureOpenAi, protocol.ConversationModelApiAzureAi:
		return "Azure"
	case protocol.ConversationModelApiPlugin:
		return "ACP"
	}
	if isLocalProvider(p) {
		return "Local"
	}
	return ""
}

func isDedicatedMediaModelID(modelID string) bool {
	name := strings.ToLower(strings.TrimSpace(modelID))
	name = name[strings.LastIndex(name, "/")+1:]
	for _, prefix := range []string{
		"gpt-image", "dall-e", "sora", "tts-", "gpt-4o-mini-tts",
		"whisper", "grok-image", "grok-imagine", "grok-2-image",
		"imagen-", "flux-",
	} {
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

func isPureMediaModel(m protocol.ConversationModel) bool {
	if slices.Contains(m.Generates, protocol.MediaGenerationImageTool) {
		return false
	}
	if len(m.Generates) > 0 {
		if m.Reasoning {
			return false
		}
		if slices.Contains(m.Generates, protocol.MediaGenerationVideo) ||
			slices.Contains(m.Generates, protocol.MediaGenerationSpeech) {
			return true
		}
		return !m.Tools || isDedicatedMediaModelID(m.ID)
	}
	return !m.Reasoning && isDedicatedMediaModelID(m.ID)
}
