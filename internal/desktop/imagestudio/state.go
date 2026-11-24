// Package imagestudio renders the Creative Studio workspace and the custom
// Image API settings page. All HTTP calls, disk I/O, credential storage, and
// image decoding run on background workers in internal/desktop/live.
package imagestudio

import (
	"image"
	"slices"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	core "github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
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

// Filter narrows the gallery to jobs of one outcome.
type Filter int

const (
	FilterAll Filter = iota
	FilterDone
	FilterRunning
	FilterFailed
)

// MaskEditor is the inpainting dialog's state: the first reference image
// decoded as the endpoint receives it, the strokes painted over it in its
// own pixels, and a preview of their coverage at display resolution.
type MaskEditor struct {
	Open    bool
	Path    string
	Loading bool
	Failed  bool
	// Image is the reference decoded without EXIF orientation, W×H its
	// stored pixels, which the mask must match.
	Image ui.ImageSource
	W, H  int

	Strokes []core.MaskStroke
	Erase   bool
	Width   float32

	// drawing is the stroke being dragged; cov and overlay the preview
	// (cov's pixels are scale times the image's), shown through bitmap.
	drawing bool
	scale   float32
	cov     *image.Alpha
	overlay [2]*image.RGBA
	front   int
	bitmap  *ui.Bitmap
	tint    ui.Color
	dirty   bool
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
	// References are the draft's stored reference images (ReferenceStore
	// paths) and MaskStrokes its inpainting mask over the first.
	References  []string
	MaskStrokes []core.MaskStroke
	Mask        MaskEditor

	// Persisted job history (newest first) and cached thumbnails.
	Jobs   []core.Job
	Thumbs map[string]Thumb
	Viewer kit.Viewer[ViewerEntry]
	Scroll ui.ScrollState
	Query  string
	Filter Filter

	// Callbacks wired by internal/desktop/live.
	// PickReferences opens the file picker; AddReferences imports dropped
	// or pasted files and AddReferenceData a pasted picture, each adding
	// the stored copies to the draft. OpenMask decodes the first reference
	// for the mask editor.
	PickReferences   func()
	AddReferences    func(paths []string)
	AddReferenceData func(data []byte)
	OpenMask         func()
	// UseAsReference adds a generated picture to the draft's references;
	// Inpaint makes it the only reference and opens the mask editor on it.
	UseAsReference func(path string)
	Inpaint        func(path string)
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
	return s != nil && s.ChannelReady() && strings.TrimSpace(s.Prompt) != "" &&
		len(s.References) <= s.ReferenceLimit()
}

// ReferenceLimit is how many reference images the active model accepts.
func (s *State) ReferenceLimit() int {
	return core.ReferenceLimit(s.EffectiveModel())
}

// AddReferencePaths adds stored reference images to the draft, skipping
// ones it has, up to ReferenceLimit. It reports whether some were left out
// for the limit.
func (s *State) AddReferencePaths(paths []string) bool {
	if s == nil {
		return false
	}
	limit := s.ReferenceLimit()
	for _, path := range paths {
		if path == "" || slices.Contains(s.References, path) {
			continue
		}
		if len(s.References) >= limit {
			return true
		}
		s.References = append(s.References, path)
	}
	return false
}

// RemoveReference drops the draft's i-th reference image. The mask belongs
// to the first, so removing it drops the mask too.
func (s *State) RemoveReference(i int) {
	if s == nil || i < 0 || i >= len(s.References) {
		return
	}
	s.References = slices.Delete(s.References, i, i+1)
	if i == 0 {
		s.MaskStrokes = nil
	}
}

// HasMask reports whether the draft redraws a masked area of its first
// reference image.
func (s *State) HasMask() bool {
	return s != nil && len(s.References) > 0 && core.HasMask(s.MaskStrokes)
}

// ApplyJob restores a job's prompt, parameters, reference images and mask
// into the composer. A model the channel no longer offers falls back as
// EffectiveModel does; sizes and qualities are re-validated the same way.
func (s *State) ApplyJob(job core.Job) {
	if s == nil {
		return
	}
	spec := job.Spec
	s.Prompt = spec.Prompt
	if spec.Channel == core.ChannelCustom || spec.Channel == core.ChannelCheapRouter {
		s.Channel = spec.Channel
	}
	if spec.Channel != core.ChannelCustom && spec.Model != "" {
		s.Model = spec.Model
	}
	if spec.Size != "" {
		s.Size = spec.Size
	}
	if spec.Quality != "" {
		s.Quality = spec.Quality
	}
	if spec.Count > 0 {
		s.Count = spec.Count
	}
	s.References = slices.Clone(spec.ReferencePaths)
	s.MaskStrokes = nil
	if len(s.References) > 0 {
		s.MaskStrokes = slices.Clone(spec.MaskStrokes)
	}
}

// ClearInputs empties the draft's reference images and mask.
func (s *State) ClearInputs() {
	if s == nil {
		return
	}
	s.References, s.MaskStrokes = nil, nil
}

// Visible returns the indices of the jobs the gallery shows: those of the
// selected outcome whose prompt contains the query, case-insensitively.
func (s *State) Visible() []int {
	if s == nil {
		return nil
	}
	query := strings.ToLower(strings.TrimSpace(s.Query))
	out := make([]int, 0, len(s.Jobs))
	for i := range s.Jobs {
		job := &s.Jobs[i]
		switch s.Filter {
		case FilterDone:
			if job.Status != core.JobDone {
				continue
			}
		case FilterRunning:
			if job.Status != core.JobRunning {
				continue
			}
		case FilterFailed:
			if job.Status != core.JobFailed && job.Status != core.JobCancelled {
				continue
			}
		}
		if query != "" && !strings.Contains(strings.ToLower(job.Spec.Prompt), query) {
			continue
		}
		out = append(out, i)
	}
	return out
}

// Filtered reports whether the query or outcome filter hides any jobs.
func (s *State) Filtered() bool {
	return s != nil && (s.Filter != FilterAll || strings.TrimSpace(s.Query) != "")
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

// ViewerEntries collects the completed output images of the visible jobs
// (Visible), in gallery order.
func (s *State) ViewerEntries(visible []int) []ViewerEntry {
	if s == nil {
		return nil
	}
	var out []ViewerEntry
	for _, i := range visible {
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
