// Package imagestudio renders the Creative Studio workspace and the custom
// Image API settings page. All HTTP calls, disk I/O, credential storage, and
// image decoding run on background workers in internal/desktop/live.
package imagestudio

import (
	"strings"

	"github.com/ai-poet/agent-client/internal/cheapauth"
	core "github.com/ai-poet/agent-client/internal/imagestudio"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

// SettingsState is UI-thread presentation state for Settings -> Image API.
// It never stores the persisted API key; APIKeyInput only holds unsaved input
// while the user edits it and is cleared immediately after save.
type SettingsState struct {
	BaseURL     string
	APIKeyInput string
	APIKeyShown bool
	HasSavedKey bool
	Model       string
	ExtraParams string
	Configured  bool

	Saving   bool
	ErrorKey string

	Save       func()
	ClearKey   func()
	ClearAll   func()
	OpenStudio func()
}

// NewSettingsState returns an empty UI settings state.
func NewSettingsState() *SettingsState { return &SettingsState{} }

// ApplySummary updates UI fields from a UI-safe CustomSummary without touching
// unsaved key input unless resetInput is true.
func (s *SettingsState) ApplySummary(summary core.CustomSummary, resetInput bool) {
	if s == nil {
		return
	}
	s.BaseURL = summary.BaseURL
	s.Model = summary.Model
	s.ExtraParams = summary.ExtraParams
	s.HasSavedKey = summary.HasAPIKey
	s.Configured = summary.Configured
	if resetInput {
		s.APIKeyInput = ""
		s.APIKeyShown = false
	}
}

// ThumbStatus tracks the asynchronous decode state of a local output image.
type ThumbStatus uint8

const (
	ThumbIdle ThumbStatus = iota
	ThumbLoading
	ThumbReady
	ThumbMissing
)

// Thumb holds one cached decoded thumbnail on the UI thread.
type Thumb struct {
	Status ThumbStatus
	Image  ui.ImageSource
}

// ViewerEntry identifies one generated picture in the fullscreen kit.ImageViewer.
type ViewerEntry struct {
	JobID  string
	Path   string
	Prompt string
}

// State is UI-thread presentation state for the Creative Studio page.
// It never stores API keys or OAuth tokens.
type State struct {
	Loaded bool

	// CheapRouter channel state mirrored from cheapauth.
	CheapSignedIn      bool
	CheapLoading       bool
	CheapModels        []cheapauth.Model
	CheapCatalog       []cheapauth.Model

	// Custom API channel summary (no secret).
	Custom core.CustomSummary

	// Active controls / composer draft.
	Channel core.Channel
	Model   string
	Size    string
	Quality string
	Count   int
	Prompt  string

	// Persisted job history (newest first) and cached thumbnails.
	Jobs    []core.Job
	Thumbs  map[string]Thumb
	Viewer  kit.Viewer[ViewerEntry]
	Scroll  ui.ScrollState

	// Callbacks wired by internal/desktop/live.
	Submit           func()
	CancelJob        func(jobID string)
	RetryJob         func(jobID string)
	RemoveJob        func(jobID string)
	SaveImageAs      func(path string)
	OpenImage        func(path string)
	RevealImage      func(path string)
	CopyImage        func(path string)
	OpenOutputFolder func()
	RefreshModels    func()
	OpenSettings     func()
	OpenCheapAccount func()
	OnPrefsChanged   func()
	RequestThumb     func(path string)
}

// NewState initializes default Creative Studio UI state.
func NewState() *State {
	return &State{
		Channel: core.ChannelCheapRouter,
		Size:    "1024x1024",
		Quality: "auto",
		Count:   1,
		Thumbs:  map[string]Thumb{},
	}
}

// EffectiveChannel returns the currently selected channel, defaulting to CheapRouter.
func (s *State) EffectiveChannel() core.Channel {
	if s == nil {
		return core.ChannelCheapRouter
	}
	if s.Channel == core.ChannelCustom {
		return core.ChannelCustom
	}
	return core.ChannelCheapRouter
}

// EffectiveModel resolves the active model ID for the selected channel.
func (s *State) EffectiveModel() string {
	if s == nil {
		return ""
	}
	if s.EffectiveChannel() == core.ChannelCustom {
		return strings.TrimSpace(s.Custom.Model)
	}
	if len(s.CheapModels) == 0 {
		return ""
	}
	for _, m := range s.CheapModels {
		if strings.EqualFold(m.ID, strings.TrimSpace(s.Model)) {
			return m.ID
		}
	}
	return s.CheapModels[0].ID
}

// EffectiveSize resolves a valid size for the active model.
func (s *State) EffectiveSize() string {
	options := core.SizeOptions(s.EffectiveModel())
	if s != nil {
		for _, opt := range options {
			if opt == s.Size {
				return opt
			}
		}
	}
	return options[0]
}

// EffectiveQuality resolves a valid quality for the active model.
func (s *State) EffectiveQuality() string {
	model := s.EffectiveModel()
	if !core.SupportsQuality(model) {
		return "auto"
	}
	options := core.QualityOptions(model)
	if s != nil {
		for _, opt := range options {
			if opt == s.Quality {
				return opt
			}
		}
	}
	return options[0]
}

// EffectiveCount returns the clamped image count (1..4).
func (s *State) EffectiveCount() int {
	if s == nil || s.Count < 1 {
		return 1
	}
	if s.Count > core.MaxCount {
		return core.MaxCount
	}
	return s.Count
}

// ChannelReady reports whether the active channel can submit a generation job.
func (s *State) ChannelReady() bool {
	if s == nil {
		return false
	}
	if s.EffectiveChannel() == core.ChannelCustom {
		return s.Custom.Configured
	}
	return s.CheapSignedIn && !s.CheapLoading && len(s.CheapModels) > 0
}

// CanSubmit reports whether Generate is enabled.
func (s *State) CanSubmit() bool {
	return s != nil && s.ChannelReady() && strings.TrimSpace(s.Prompt) != ""
}

// EstimateUSD computes an estimated cost for CheapRouter when pricing metadata exists.
func (s *State) EstimateUSD() (float64, bool) {
	if s == nil || s.EffectiveChannel() != core.ChannelCheapRouter {
		return 0, false
	}
	model := s.EffectiveModel()
	if model == "" {
		return 0, false
	}
	for _, m := range s.CheapModels {
		if strings.EqualFold(m.ID, model) {
			return cheapauth.EstimateImageUSD(m, s.EffectiveSize(), s.EffectiveCount())
		}
	}
	return 0, false
}

// HasRunningJobs reports whether any job is currently executing.
func (s *State) HasRunningJobs() bool {
	if s == nil {
		return false
	}
	for i := range s.Jobs {
		if s.Jobs[i].Status == core.JobRunning {
			return true
		}
	}
	return false
}

// PictureCount returns total completed image outputs across jobs.
func (s *State) PictureCount() int {
	if s == nil {
		return 0
	}
	total := 0
	for i := range s.Jobs {
		if s.Jobs[i].Status == core.JobDone {
			total += len(s.Jobs[i].Outputs)
		}
	}
	return total
}

// AllViewerEntries collects all completed output images in gallery order.
func (s *State) AllViewerEntries() []ViewerEntry {
	if s == nil {
		return nil
	}
	var out []ViewerEntry
	for i := range s.Jobs {
		job := &s.Jobs[i]
		if job.Status != core.JobDone {
			continue
		}
		for _, path := range job.Outputs {
			out = append(out, ViewerEntry{
				JobID:  job.ID,
				Path:   path,
				Prompt: job.Spec.Prompt,
			})
		}
	}
	return out
}
