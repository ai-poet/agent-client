// Package anthropic carries provider options without linking a model transport.
// Derived from Fantasy v0.46.0 providers/anthropic/provider_options.go and the
// cache-control accessors in anthropic.go. Apache-2.0; see LICENSE.fantasy.
// Modified for Sailry's direct anthropic-sdk-go transport.
package anthropic

import (
	"encoding/json"

	"charm.land/fantasy"
)

const Name = "anthropic"

type Effort string

const (
	EffortLow    Effort = "low"
	EffortMedium Effort = "medium"
	EffortHigh   Effort = "high"
	EffortXHigh  Effort = "xhigh"
	EffortMax    Effort = "max"
)

type ThinkingDisplay string

const (
	ThinkingDisplaySummarized ThinkingDisplay = "summarized"
	ThinkingDisplayOmitted    ThinkingDisplay = "omitted"
)

const (
	TypeProviderOptions         = Name + ".options"
	TypeReasoningOptionMetadata = Name + ".reasoning_metadata"
	TypeProviderCacheControl    = Name + ".cache_control_options"
	TypeWebSearchResultMetadata = Name + ".web_search_result_metadata"
)

type ProviderOptions struct {
	SendReasoning          *bool                   `json:"send_reasoning"`
	Thinking               *ThinkingProviderOption `json:"thinking"`
	Effort                 *Effort                 `json:"effort"`
	ThinkingDisplay        *ThinkingDisplay        `json:"thinking_display"`
	DisableParallelToolUse *bool                   `json:"disable_parallel_tool_use"`
	ExtraBody              map[string]any          `json:"extra_body,omitempty"`
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

type ThinkingProviderOption struct {
	BudgetTokens int64 `json:"budget_tokens"`
}

type ReasoningOptionMetadata struct {
	Signature    string `json:"signature"`
	RedactedData string `json:"redacted_data"`
}

func (*ReasoningOptionMetadata) Options() {}
func (metadata ReasoningOptionMetadata) MarshalJSON() ([]byte, error) {
	type plain ReasoningOptionMetadata
	return fantasy.MarshalProviderType(TypeReasoningOptionMetadata, plain(metadata))
}
func (metadata *ReasoningOptionMetadata) UnmarshalJSON(data []byte) error {
	type plain ReasoningOptionMetadata
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*metadata = ReasoningOptionMetadata(decoded)
	return nil
}

type ProviderCacheControlOptions struct {
	CacheControl CacheControl `json:"cache_control"`
}

func (*ProviderCacheControlOptions) Options() {}
func (options ProviderCacheControlOptions) MarshalJSON() ([]byte, error) {
	type plain ProviderCacheControlOptions
	return fantasy.MarshalProviderType(TypeProviderCacheControl, plain(options))
}
func (options *ProviderCacheControlOptions) UnmarshalJSON(data []byte) error {
	type plain ProviderCacheControlOptions
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*options = ProviderCacheControlOptions(decoded)
	return nil
}

type WebSearchResultItem struct {
	URL              string `json:"url"`
	Title            string `json:"title"`
	EncryptedContent string `json:"encrypted_content"`
	PageAge          string `json:"page_age,omitempty"`
}

type WebSearchResultMetadata struct {
	Results   []WebSearchResultItem `json:"results,omitempty"`
	ErrorCode string                `json:"error_code,omitempty"`
}

func (*WebSearchResultMetadata) Options() {}
func (metadata WebSearchResultMetadata) MarshalJSON() ([]byte, error) {
	type plain WebSearchResultMetadata
	return fantasy.MarshalProviderType(TypeWebSearchResultMetadata, plain(metadata))
}
func (metadata *WebSearchResultMetadata) UnmarshalJSON(data []byte) error {
	type plain WebSearchResultMetadata
	var decoded plain
	if err := fantasy.UnmarshalProviderType(data, &decoded); err != nil {
		return err
	}
	*metadata = WebSearchResultMetadata(decoded)
	return nil
}

type CacheControl struct {
	Type string `json:"type"`
}

func NewProviderOptions(options *ProviderOptions) fantasy.ProviderOptions {
	return fantasy.ProviderOptions{Name: options}
}
func NewProviderCacheControlOptions(options *ProviderCacheControlOptions) fantasy.ProviderOptions {
	return fantasy.ProviderOptions{Name: options}
}
func GetCacheControl(options fantasy.ProviderOptions) *CacheControl {
	if value, ok := options[Name].(*ProviderCacheControlOptions); ok {
		return &value.CacheControl
	}
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
	fantasy.RegisterProviderType(TypeReasoningOptionMetadata, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var metadata ReasoningOptionMetadata
		if err := json.Unmarshal(data, &metadata); err != nil {
			return nil, err
		}
		return &metadata, nil
	})
	fantasy.RegisterProviderType(TypeProviderCacheControl, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var options ProviderCacheControlOptions
		if err := json.Unmarshal(data, &options); err != nil {
			return nil, err
		}
		return &options, nil
	})
	fantasy.RegisterProviderType(TypeWebSearchResultMetadata, func(data []byte) (fantasy.ProviderOptionsData, error) {
		var metadata WebSearchResultMetadata
		if err := json.Unmarshal(data, &metadata); err != nil {
			return nil, err
		}
		return &metadata, nil
	})
}
