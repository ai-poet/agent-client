package providers

import (
	"encoding/json"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

const cheapRouterKeyBundleVersion = 1

type cheapRouterKeyBundle struct {
	Version    int               `json:"cheaprouter_key_bundle"`
	Default    string            `json:"default,omitempty"`
	LaneKeys   map[string]string `json:"lane_keys,omitempty"`
	ModelLanes map[string]string `json:"model_lanes,omitempty"`
}

// ResolveCheapRouterKey selects the gateway key for modelID/platform from a
// plain API key or a legacy stored multi-lane bundle, and returns all keys in the
// bundle for error redaction.
func ResolveCheapRouterKey(rawSecret, modelID, platform string) (string, []protocol.Secret) {
	trimmed := strings.TrimSpace(rawSecret)
	if !strings.HasPrefix(trimmed, "{") {
		return rawSecret, nil
	}
	var bundle cheapRouterKeyBundle
	if json.Unmarshal([]byte(trimmed), &bundle) != nil || bundle.Version != cheapRouterKeyBundleVersion {
		return rawSecret, nil
	}
	seen := map[string]bool{}
	var protection []protocol.Secret
	addSecret := func(key string) {
		key = strings.TrimSpace(key)
		if key != "" && !seen[key] {
			seen[key] = true
			protection = append(protection, protocol.NewSecret(key))
		}
	}
	addSecret(bundle.Default)
	for _, key := range bundle.LaneKeys {
		addSecret(key)
	}
	modelKey := strings.ToLower(strings.TrimSpace(modelID))
	if lane, ok := bundle.ModelLanes[modelKey]; ok {
		if key := strings.TrimSpace(bundle.LaneKeys[lane]); key != "" {
			return key, protection
		}
	}
	if lane := cheapauth.ModelLane(cheapauth.Model{ID: modelID, Platform: platform}); lane != "" {
		if key := strings.TrimSpace(bundle.LaneKeys[lane]); key != "" {
			return key, protection
		}
	}
	if key := strings.TrimSpace(bundle.Default); key != "" {
		return key, protection
	}
	if len(bundle.LaneKeys) == 1 {
		for _, key := range bundle.LaneKeys {
			return strings.TrimSpace(key), protection
		}
	}
	return "", protection
}
