// Package google carries provider options without linking a model transport.
// Derived from Fantasy v0.46.0 providers/google/provider_options.go.
// Apache-2.0; see LICENSE.fantasy. Modified for Sailry's direct GenAI transport.
package google

import (
	"encoding/json"

	"charm.land/fantasy"
)

const Name = "google"

const (
	TypeProviderOptions   = Name + ".options"
	TypeReasoningMetadata = Name + ".reasoning_metadata"
)

type ThinkingLevel = string

const (
	ThinkingLevelLow     ThinkingLevel = "LOW"
	ThinkingLevelMedium  ThinkingLevel = "MEDIUM"
	ThinkingLevelHigh    ThinkingLevel = "HIGH"
	ThinkingLevelMinimal ThinkingLevel = "MINIMAL"
)

type ThinkingConfig struct {
	ThinkingBudget  *int64  `json:"thinking_budget,omitempty"`
	IncludeThoughts *bool   `json:"include_thoughts,omitempty"`
	ThinkingLevel   *string `json:"thinking_level,omitempty"`
}

type ReasoningMetadata struct {
	Signature string `json:"signature"`
	ToolID    string `json:"tool_id"`
}

func (*ReasoningMetadata) Options() {}
func (metadata ReasoningMetadata) MarshalJSON() ([]byte, error) {
	type plain ReasoningMetadata
	return fantasy.MarshalProviderType(TypeReasoningMetadata, plain(metadata))
}
func (metadata *ReasoningMetadata) UnmarshalJSON(data []byte) error {
	type plain ReasoningMetadata
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*metadata = ReasoningMetadata(decoded)
	return nil
}

type SafetySetting struct {
	Category  string `json:"category"`
	Threshold string `json:"threshold"`
}

type ProviderOptions struct {
	ThinkingConfig *ThinkingConfig `json:"thinking_config"`
	CachedContent  string          `json:"cached_content"`
	SafetySettings []SafetySetting `json:"safety_settings"`
	Threshold      string          `json:"threshold"`
}

func (*ProviderOptions) Options() {}
func (options ProviderOptions) MarshalJSON() ([]byte, error) {
	type plain ProviderOptions
	return fantasy.MarshalProviderType(TypeProviderOptions, plain(options))
}
func (options *ProviderOptions) UnmarshalJSON(data []byte) error {
	type plain ProviderOptions
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*options = ProviderOptions(decoded)
	return nil
}

func init() {
	fantasy.RegisterProviderType(TypeProviderOptions, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var options ProviderOptions
		if err := json.Unmarshal(data, &options); err != nil {
			return nil, err
		}
		return &options, nil
	})
	fantasy.RegisterProviderType(TypeReasoningMetadata, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var metadata ReasoningMetadata
		if err := json.Unmarshal(data, &metadata); err != nil {
			return nil, err
		}
		return &metadata, nil
	})
}
