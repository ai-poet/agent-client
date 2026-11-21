package providers

import (
	"encoding/json"
	"strings"

	"github.com/ai-poet/agent-client/internal/cheapauth"
	"github.com/ai-poet/agent-client/internal/protocol"
)

const cheapRouterKeyBundleVersion = 1

type cheapRouterKeyBundle struct {
	Version    int               `json:"cheaprouter_key_bundle"`
	Default    string            `json:"default,omitempty"`
	LaneKeys   map[string]string `json:"lane_keys,omitempty"`
	ModelLanes map[string]string `json:"model_lanes,omitempty"`
}

// EncodeCheapRouterSecret packs per-lane gateway keys into the protected
// provider Secret when lane routing is active, or returns the plain API key
// for single-key gateways.
func EncodeCheapRouterSecret(gateway cheapauth.Gateway) (*protocol.Secret, error) {
	defaultKey := strings.TrimSpace(gateway.APIKey)
	laneKeys := map[string]string{}
	for lane, key := range gateway.LaneKeys {
		key = strings.TrimSpace(key)
		lane = strings.ToLower(strings.TrimSpace(lane))
		if lane != "" && key != "" {
			laneKeys[lane] = key
		}
	}
	if len(laneKeys) == 0 {
		if defaultKey == "" {
			return nil, invalid("CheapRouter gateway key is empty")
		}
		secret := protocol.NewSecret(defaultKey)
		return &secret, nil
	}
	modelLanes := map[string]string{}
	for modelID, lane := range gateway.ModelLanes {
		modelID = strings.ToLower(strings.TrimSpace(modelID))
		lane = strings.ToLower(strings.TrimSpace(lane))
		if modelID == "" || lane == "" || laneKeys[lane] == "" {
			continue
		}
		// Only record overrides where inferred lane differs, keeping the bundle compact.
		if cheapauth.ModelLane(cheapauth.Model{ID: modelID}) != lane {
			modelLanes[modelID] = lane
		}
	}
	bundle := cheapRouterKeyBundle{
		Version:    cheapRouterKeyBundleVersion,
		Default:    defaultKey,
		LaneKeys:   laneKeys,
		ModelLanes: modelLanes,
	}
	encoded, err := json.Marshal(bundle)
	if err != nil {
		return nil, invalid("CheapRouter gateway key bundle is invalid")
	}
	secret := protocol.NewSecret(string(encoded))
	return &secret, nil
}

// ResolveCheapRouterKey selects the gateway key for modelID/platform from a
// plain API key or an encoded multi-lane bundle, and returns all keys in the
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
