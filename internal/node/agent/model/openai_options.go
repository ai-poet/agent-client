package model

import (
	"encoding/json"

	"charm.land/fantasy"
)

// The OpenAI transport is implemented directly with openai-go/v3. These small
// option carriers preserve the existing provider metadata wire format without
// linking Fantasy's second, charmbracelet/openai-go-based OpenAI transport.
const (
	openAIName                     = "openai"
	typeResponsesProviderOptions   = openAIName + ".responses.options"
	typeResponsesReasoningMetadata = openAIName + ".responses.reasoning_metadata"
)

type openAIResponsesReasoningMetadata struct {
	ItemID           string   `json:"item_id"`
	EncryptedContent *string  `json:"encrypted_content"`
	Summary          []string `json:"summary"`
}

func (*openAIResponsesReasoningMetadata) Options() {}

func (metadata openAIResponsesReasoningMetadata) MarshalJSON() ([]byte, error) {
	type plain openAIResponsesReasoningMetadata
	return fantasy.MarshalProviderType(typeResponsesReasoningMetadata, plain(metadata))
}

func (metadata *openAIResponsesReasoningMetadata) UnmarshalJSON(data []byte) error {
	type plain openAIResponsesReasoningMetadata
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*metadata = openAIResponsesReasoningMetadata(decoded)
	return nil
}

type openAIIncludeType string
type openAIServiceTier string
type openAIReasoningEffort string
type openAITextVerbosity string

type openAIResponsesProviderOptions struct {
	Include            []openAIIncludeType    `json:"include"`
	Instructions       *string                `json:"instructions"`
	Logprobs           any                    `json:"logprobs"`
	MaxToolCalls       *int64                 `json:"max_tool_calls"`
	Metadata           map[string]any         `json:"metadata"`
	ParallelToolCalls  *bool                  `json:"parallel_tool_calls"`
	PreviousResponseID *string                `json:"previous_response_id"`
	PromptCacheKey     *string                `json:"prompt_cache_key"`
	ReasoningEffort    *openAIReasoningEffort `json:"reasoning_effort"`
	ReasoningSummary   *string                `json:"reasoning_summary"`
	SafetyIdentifier   *string                `json:"safety_identifier"`
	ServiceTier        *openAIServiceTier     `json:"service_tier"`
	Store              *bool                  `json:"store"`
	StrictJSONSchema   *bool                  `json:"strict_json_schema"`
	TextVerbosity      *openAITextVerbosity   `json:"text_verbosity"`
	User               *string                `json:"user"`
}

func (*openAIResponsesProviderOptions) Options() {}

func (options openAIResponsesProviderOptions) MarshalJSON() ([]byte, error) {
	type plain openAIResponsesProviderOptions
	return fantasy.MarshalProviderType(typeResponsesProviderOptions, plain(options))
}

func (options *openAIResponsesProviderOptions) UnmarshalJSON(data []byte) error {
	type plain openAIResponsesProviderOptions
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*options = openAIResponsesProviderOptions(decoded)
	return nil
}

func newOpenAIResponsesProviderOptions(options *openAIResponsesProviderOptions) fantasy.ProviderOptions {
	return fantasy.ProviderOptions{openAIName: options}
}

func init() {
	fantasy.RegisterProviderType(typeResponsesProviderOptions, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var options openAIResponsesProviderOptions
		if err := json.Unmarshal(data, &options); err != nil {
			return nil, err
		}
		return &options, nil
	})
	fantasy.RegisterProviderType(typeResponsesReasoningMetadata, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var metadata openAIResponsesReasoningMetadata
		if err := json.Unmarshal(data, &metadata); err != nil {
			return nil, err
		}
		return &metadata, nil
	})
}
