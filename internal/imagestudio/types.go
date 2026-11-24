// Package imagestudio implements AI image generation over CheapRouter's
// managed Images API and user-configured OpenAI-compatible image endpoints.
// Custom API keys are stored in 0600 private files and are redacted from
// string formatting, logs, and error messages.
package imagestudio

import (
	"fmt"
	"strings"
)

// Channel identifies which backend generates images.
type Channel string

const (
	ChannelCheapRouter Channel = "cheaprouter"
	ChannelCustom      Channel = "custom"
)

// CustomConfig holds the persisted custom OpenAI-compatible image endpoint.
// String and GoString redact APIKey so formatting never exposes it.
type CustomConfig struct {
	BaseURL     string `json:"base_url"`
	APIKey      string `json:"api_key,omitempty"`
	Model       string `json:"model"`
	ExtraParams string `json:"extra_params,omitempty"`
}

func (c CustomConfig) String() string {
	return fmt.Sprintf("CustomConfig{BaseURL:%q, Model:%q, HasKey:%v}", c.BaseURL, c.Model, strings.TrimSpace(c.APIKey) != "")
}

func (c CustomConfig) GoString() string { return c.String() }

// Configured reports whether the minimum required fields (BaseURL and Model) are set.
func (c CustomConfig) Configured() bool {
	return strings.TrimSpace(c.BaseURL) != "" && strings.TrimSpace(c.Model) != ""
}

// Summary returns the UI-safe projection without the API key.
func (c CustomConfig) Summary() CustomSummary {
	return CustomSummary{
		BaseURL:     strings.TrimSpace(c.BaseURL),
		Model:       strings.TrimSpace(c.Model),
		ExtraParams: strings.TrimSpace(c.ExtraParams),
		HasAPIKey:   strings.TrimSpace(c.APIKey) != "",
		Configured:  c.Configured(),
	}
}

// CustomSummary is the UI-safe view of CustomConfig. It never carries APIKey.
type CustomSummary struct {
	BaseURL     string
	Model       string
	ExtraParams string
	HasAPIKey   bool
	Configured  bool
}

// ReferenceImage is one input image supplied to /v1/images/edits.
type ReferenceImage struct {
	Filename string
	MimeType string
	Data     []byte
}

// GenerateSpec describes one image generation or editing request.
type GenerateSpec struct {
	Channel        Channel `json:"channel"`
	Model          string  `json:"model"`
	Prompt         string  `json:"prompt"`
	Size           string  `json:"size,omitempty"`
	Quality        string  `json:"quality,omitempty"`
	ResponseFormat string  `json:"response_format,omitempty"`
	Count          int     `json:"count"`
	// ReferencePaths name a studio job's stored reference images
	// (ReferenceStore) and MaskStrokes its inpainting mask over the first,
	// so the job can be retried or its parameters applied after a restart.
	// LoadInputs reads them into References and Mask for the request.
	ReferencePaths []string     `json:"reference_paths,omitempty"`
	MaskStrokes    []MaskStroke `json:"mask_strokes,omitempty"`
	// References are the edit request's input images; Mask, when set, is
	// the PNG mask over the first, transparent where it is redrawn.
	References []ReferenceImage `json:"-"`
	Mask       *ReferenceImage  `json:"-"`
}

// IsEdit reports whether this request edits one or more reference images.
func (s GenerateSpec) IsEdit() bool {
	return len(s.References) > 0
}

// IsInpaint reports whether this request redraws a masked area of its first
// reference image.
func (s GenerateSpec) IsInpaint() bool {
	return s.IsEdit() && s.Mask != nil
}

// DecodedImage holds raw image bytes decoded from base64 or downloaded from URL.
type DecodedImage struct {
	Data          []byte
	Ext           string
	MimeType      string
	RevisedPrompt string
}

// NormalizedCount clamps Count to [1, MaxCount].
func (s GenerateSpec) NormalizedCount() int {
	if s.Count < 1 {
		return 1
	}
	if s.Count > MaxCount {
		return MaxCount
	}
	return s.Count
}

// ImageSource holds either base64-encoded image bytes or a downloadable URL.
type ImageSource struct {
	Base64 string
	URL    string
}

// ImageOutput is one generated image returned by an Images endpoint.
type ImageOutput struct {
	Source        ImageSource
	RevisedPrompt string
}

// ErrorKind categorizes image generation failures for actionable UI copy.
type ErrorKind string

const (
	ErrNoImagePermission ErrorKind = "no_image_permission"
	ErrWrongPlatform     ErrorKind = "wrong_platform"
	ErrModelUnavailable  ErrorKind = "model_unavailable"
	ErrContentPolicy     ErrorKind = "content_policy"
	ErrOwnBalance        ErrorKind = "own_balance"
	ErrUpstreamBalance   ErrorKind = "upstream_balance"
	ErrBusy              ErrorKind = "busy"
	ErrTimeout           ErrorKind = "timeout"
	ErrAsyncUnavailable  ErrorKind = "async_unavailable"
	ErrTaskLost          ErrorKind = "task_lost"
	ErrInterrupted       ErrorKind = "interrupted"
	ErrBadRequest        ErrorKind = "bad_request"
	ErrUnauthorized      ErrorKind = "unauthorized"
	ErrNetwork           ErrorKind = "network"
	ErrCancelled         ErrorKind = "cancelled"
	ErrOther             ErrorKind = "other"
)

// ImageError is a categorized, secret-sanitized generation error.
type ImageError struct {
	Kind      ErrorKind `json:"kind"`
	Status    int       `json:"status,omitempty"`
	Message   string    `json:"message"`
	TaskID    string    `json:"task_id,omitempty"`
	TaskEnded bool      `json:"-"`
}

func (e *ImageError) Error() string {
	if e == nil {
		return ""
	}
	if e.Status > 0 && e.Message != "" {
		return fmt.Sprintf("HTTP %d: %s", e.Status, e.Message)
	}
	if e.Message != "" {
		return e.Message
	}
	return string(e.Kind)
}

// ShouldTryNextGroup reports whether CheapRouter should fall through to the
// next candidate group (matching Rust sub2api::images::try_next_group).
func (e *ImageError) ShouldTryNextGroup() bool {
	if e == nil {
		return false
	}
	switch e.Kind {
	case ErrNoImagePermission, ErrModelUnavailable, ErrWrongPlatform:
		return true
	default:
		return false
	}
}

// MessageKey returns the i18n key describing this failure category.
func (e *ImageError) MessageKey() string {
	if e == nil {
		return "image_studio_error_other"
	}
	switch e.Kind {
	case ErrNoImagePermission:
		return "image_studio_error_no_permission"
	case ErrWrongPlatform:
		return "image_studio_error_wrong_platform"
	case ErrModelUnavailable:
		return "image_studio_error_model_unavailable"
	case ErrContentPolicy:
		return "image_studio_error_content_policy"
	case ErrOwnBalance:
		return "image_studio_error_balance"
	case ErrUpstreamBalance:
		return "image_studio_error_upstream_balance"
	case ErrBusy:
		return "image_studio_error_busy"
	case ErrTimeout:
		return "image_studio_error_timeout"
	case ErrTaskLost:
		return "image_studio_error_task_lost"
	case ErrInterrupted:
		return "image_studio_error_interrupted"
	case ErrUnauthorized:
		return "image_studio_error_unauthorized"
	case ErrBadRequest:
		return "image_studio_error_bad_request"
	case ErrNetwork:
		return "image_studio_error_network"
	case ErrCancelled:
		return "image_studio_error_cancelled"
	default:
		return "image_studio_error_other"
	}
}

// JobStatus tracks the lifecycle of one image generation job.
type JobStatus string

const (
	JobRunning   JobStatus = "running"
	JobDone      JobStatus = "done"
	JobFailed    JobStatus = "failed"
	JobCancelled JobStatus = "cancelled"
)

// Job is one generation request and its results, persisted in the studio index.
type Job struct {
	ID          string       `json:"id"`
	CreatedAt   int64        `json:"created_at"`
	StartedAt   int64        `json:"started_at"`
	FinishedAt  int64        `json:"finished_at,omitempty"`
	Spec        GenerateSpec `json:"spec"`
	GroupID     int64        `json:"group_id,omitempty"`
	GroupName   string       `json:"group_name,omitempty"`
	Endpoint    string       `json:"endpoint,omitempty"`
	TaskID      string       `json:"task_id,omitempty"`
	EstimateUSD *float64     `json:"estimate_usd,omitempty"`
	Stem        string       `json:"stem"`
	Folder      string       `json:"folder"`
	Status      JobStatus    `json:"status"`
	Outputs     []string     `json:"outputs,omitempty"`
	Error       *ImageError  `json:"error,omitempty"`
}

// Prefs remembers the last selected studio parameters.
type Prefs struct {
	Channel Channel `json:"channel,omitempty"`
	Model   string  `json:"model,omitempty"`
	Size    string  `json:"size,omitempty"`
	Quality string  `json:"quality,omitempty"`
	Count   int     `json:"count,omitempty"`
}

// Denial records a CheapRouter group that recently refused image generation.
type Denial struct {
	Model   string `json:"model"`
	GroupID int64  `json:"group_id"`
	At      int64  `json:"at"`
}

// Index is persisted to <profile>/image-studio/index.json.
type Index struct {
	Prefs   Prefs    `json:"prefs"`
	Denials []Denial `json:"denials,omitempty"`
	Jobs    []Job    `json:"jobs,omitempty"`
}
