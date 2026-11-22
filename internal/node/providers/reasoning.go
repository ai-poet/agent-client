package providers

import (
	"encoding/json"
	"strconv"
	"strings"
	"sync"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func bareModelName(modelID string) string {
	name := strings.ToLower(strings.TrimSpace(modelID))
	if _, bare, ok := strings.Cut(name, "::"); ok {
		name = strings.TrimSpace(bare)
	}
	name = strings.TrimPrefix(name, "models/")
	if idx := strings.LastIndexByte(name, '/'); idx >= 0 {
		name = name[idx+1:]
	}
	return name
}

// IsDeepSeekModel mirrors Rust provider_options::is_deepseek_model.
func IsDeepSeekModel(modelID string) bool {
	return strings.HasPrefix(bareModelName(modelID), "deepseek-")
}

// IsGLMThinkingModel mirrors Rust provider_options::is_glm_thinking_model:
// GLM 4.5 and later support the thinking switch + high/max reasoning_effort.
func IsGLMThinkingModel(modelID string) bool {
	name := bareModelName(modelID)
	rest, ok := strings.CutPrefix(name, "glm-")
	if !ok {
		return false
	}
	end := 0
	for end < len(rest) && ((rest[end] >= '0' && rest[end] <= '9') || rest[end] == '.') {
		end++
	}
	number := strings.TrimRight(rest[:end], ".")
	version, err := strconv.ParseFloat(number, 64)
	return err == nil && version >= 4.5
}

// IsKimiK3Model mirrors Rust provider_options::is_kimi_k3_model.
func IsKimiK3Model(modelID string) bool {
	name := bareModelName(modelID)
	return name == "k3" || name == "k3-256k" || strings.HasPrefix(name, "kimi-k3")
}

// HasThreeStepEffort mirrors Rust sub2api::model_routing::has_three_step_effort.
func HasThreeStepEffort(modelID string) bool {
	return IsDeepSeekModel(modelID) || IsGLMThinkingModel(modelID) || IsKimiK3Model(modelID)
}

// IsGeminiThinkingModel reports whether a Gemini model supports thinkingLevel /
// thinkingBudget reasoning effort selection.
func IsGeminiThinkingModel(modelID string) bool {
	name := bareModelName(modelID)
	return strings.HasPrefix(name, "gemini-2.5") ||
		strings.HasPrefix(name, "gemini-3") ||
		strings.HasPrefix(name, "gemini-4")
}

// IsOpenAIReasoningModel mirrors Rust provider_options::is_openai_reasoning_model.
func IsOpenAIReasoningModel(modelID string) bool {
	name := bareModelName(modelID)
	if strings.HasPrefix(name, "gpt-5") || strings.HasPrefix(name, "gpt5") ||
		gptMajorVersion(name) >= 5 ||
		strings.HasPrefix(name, "codex") ||
		name == "grok" || strings.HasPrefix(name, "grok-") {
		return true
	}
	for _, prefix := range []string{"o1", "o3", "o4", "o5"} {
		if name == prefix || strings.HasPrefix(name, prefix+"-") {
			return true
		}
	}
	return false
}

func gptMajorVersion(name string) int {
	rest, ok := strings.CutPrefix(name, "gpt-")
	if !ok {
		return 0
	}
	end := 0
	for end < len(rest) && rest[end] >= '0' && rest[end] <= '9' {
		end++
	}
	if end == 0 {
		return 0
	}
	n, err := strconv.Atoi(rest[:end])
	if err != nil {
		return 0
	}
	return n
}

// IsClaudeReasoningModel reports whether a Claude model accepts reasoning effort.
func IsClaudeReasoningModel(modelID string) bool {
	name := bareModelName(modelID)
	name = strings.TrimPrefix(name, "anthropic.")
	if !strings.HasPrefix(name, "claude-") {
		return false
	}
	for _, legacy := range []string{"claude-2", "claude-instant", "claude-3-haiku", "claude-3-opus", "claude-3-sonnet", "claude-3-5", "claude-3.5"} {
		if strings.HasPrefix(name, legacy) {
			return false
		}
	}
	return true
}

var bundledReasoning = sync.OnceValue(func() map[string]bool {
	catalog, err := BundledCatalog()
	if err != nil {
		return nil
	}
	out := make(map[string]bool, len(catalog.Entries))
	for _, entry := range catalog.Entries {
		var meta struct {
			Reasoning *bool `json:"reasoning"`
		}
		if json.Unmarshal(entry.Body, &meta) != nil || meta.Reasoning == nil {
			continue
		}
		id := strings.ToLower(strings.TrimSpace(entry.ID))
		for _, key := range []string{id, modelLeaf(id)} {
			if key == "" {
				continue
			}
			// Prefer true if any supplier marks the model reasoning-capable.
			if prev, exists := out[key]; !exists || *meta.Reasoning && !prev {
				out[key] = *meta.Reasoning
			}
		}
	}
	return out
})

// CatalogModelReasoning reports whether the bundled models.dev registry declares
// reasoning support for modelID.
func CatalogModelReasoning(modelID string) (reasoning bool, known bool) {
	index := bundledReasoning()
	if len(index) == 0 {
		return false, false
	}
	id := strings.ToLower(strings.TrimSpace(modelID))
	if v, ok := index[id]; ok {
		return v, true
	}
	if v, ok := index[bareModelName(id)]; ok {
		return v, true
	}
	return false, false
}

// CheapRouterReasoningLadder returns the supported reasoning efforts and default
// effort for a CheapRouter catalog model, matching Rust native_agent::reasoning_ladder.
func CheapRouterReasoningLadder(platform, modelID string) ([]protocol.Effort, protocol.Effort, bool) {
	name := bareModelName(modelID)
	// Kimi K2 and MiniMax expose no reasoning depth choice on the gateway.
	if strings.HasPrefix(name, "kimi-k2") || strings.HasPrefix(name, "minimax-") {
		return nil, protocol.Effort{Kind: "default"}, false
	}
	if HasThreeStepEffort(modelID) {
		return effortsOf("low", "high", "max"), protocol.Effort{Kind: "high"}, true
	}
	if IsGeminiThinkingModel(modelID) {
		return effortsOf("low", "medium", "high"), protocol.Effort{Kind: "high"}, true
	}
	if IsClaudeReasoningModel(modelID) || IsOpenAIReasoningModel(modelID) {
		return effortsOf("low", "medium", "high", "xhigh", "max"), protocol.Effort{Kind: "high"}, true
	}
	if reasoning, known := CatalogModelReasoning(modelID); known {
		if reasoning {
			return effortsOf("low", "medium", "high", "xhigh", "max"), protocol.Effort{Kind: "high"}, true
		}
		return nil, protocol.Effort{Kind: "default"}, false
	}
	switch strings.ToLower(strings.TrimSpace(platform)) {
	case "anthropic", "grok":
		return effortsOf("low", "medium", "high", "xhigh", "max"), protocol.Effort{Kind: "high"}, true
	}
	return nil, protocol.Effort{Kind: "default"}, false
}

func effortsOf(kinds ...string) []protocol.Effort {
	out := make([]protocol.Effort, 0, len(kinds))
	for _, k := range kinds {
		out = append(out, protocol.Effort{Kind: k})
	}
	return out
}

// GoogleThinkingLevelForEffort mirrors Rust provider_options::google_thinking_level_for_effort.
func GoogleThinkingLevelForEffort(level string) string {
	switch strings.ToLower(strings.TrimSpace(level)) {
	case "none", "low":
		return "low"
	case "minimal":
		return "minimal"
	case "medium":
		return "medium"
	default:
		return "high"
	}
}

// OpenAIReasoningEffortForLevel mirrors Rust provider_options::reasoning_effort_for_level.
func OpenAIReasoningEffortForLevel(level string, allowXHigh bool) string {
	switch strings.ToLower(strings.TrimSpace(level)) {
	case "none":
		return "none"
	case "minimal":
		return "minimal"
	case "low":
		return "low"
	case "medium":
		return "medium"
	case "xhigh", "max", "ultracode":
		if allowXHigh {
			return "xhigh"
		}
		return "high"
	default:
		return "high"
	}
}
