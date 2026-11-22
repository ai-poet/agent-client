package live

import (
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	imagestudioui "github.com/ai-poet/cheaprouter-harness/internal/desktop/imagestudio"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	core "github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/egoist/mygo/ui"
	"github.com/google/uuid"
)

type imageStudioRuntime struct {
	profileDir       string
	customStore      *core.CustomStore
	indexStore       *core.IndexStore
	client           *core.Client
	state            *imagestudioui.State
	settingsState    *imagestudioui.SettingsState
	denials          []core.Denial
	running          map[string]context.CancelFunc
	asyncUnavailable bool

	saveMu  sync.Mutex
	saveSeq uint64
}

func (r *Runtime) initImageStudio() {
	profile := filepath.Dir(filepath.Dir(r.Preferences.Path))
	customStore, _ := core.NewCustomStore(profile)
	indexStore, _ := core.NewIndexStore(profile)

	state := imagestudioui.NewState()
	settingsState := imagestudioui.NewSettingsState()
	studio := &imageStudioRuntime{
		profileDir:    profile,
		customStore:   customStore,
		indexStore:    indexStore,
		client:        core.NewClient(),
		state:         state,
		settingsState: settingsState,
		running:       map[string]context.CancelFunc{},
	}
	r.imageStudio = studio
	r.App.ImageStudio = state
	r.App.Settings.ImageAPI = settingsState

	// Wire settings callbacks.
	settingsState.Save = r.saveCustomImageAPI
	settingsState.ClearKey = r.clearCustomImageAPIKey
	settingsState.ClearAll = r.clearCustomImageAPI
	settingsState.OpenStudio = func() { r.App.Navigate(shell.ImageStudioPage) }

	// Wire Creative Studio callbacks.
	state.Submit = r.submitImageStudioJob
	state.CancelJob = r.cancelImageStudioJob
	state.RetryJob = r.retryImageStudioJob
	state.RemoveJob = r.removeImageStudioJob
	state.SaveImageAs = r.saveStudioImageAs
	state.OpenImage = r.openStudioImage
	state.RevealImage = r.revealStudioImage
	state.CopyImage = r.copyStudioImage
	state.OpenOutputFolder = r.openStudioOutputFolder
	state.RefreshModels = r.refreshCheapImageCatalog
	state.OpenSettings = func() {
		r.App.Navigate(shell.SettingsPage)
		r.App.Settings.Open(settings.ImageAPI)
	}
	state.OpenCheapAccount = func() {
		r.App.Navigate(shell.SettingsPage)
		r.App.Settings.Open(settings.CheapAccount)
	}
	state.OnPrefsChanged = r.persistImageStudioIndex
	state.RequestThumb = r.loadStudioThumb

	r.syncImageStudioCatalog()

	// Load persisted custom API summary and job history off the UI thread.
	r.async(func() {
		var summary core.CustomSummary
		if customStore != nil {
			if loaded, err := customStore.Summary(); err == nil {
				summary = loaded
			}
		}
		var idx core.Index
		if indexStore != nil {
			if loaded, err := indexStore.Load(time.Now()); err == nil {
				idx = loaded
			}
		}
		r.enqueue(func(*ui.Context) {
			studio.settingsState.ApplySummary(summary, true)
			studio.state.Custom = summary
			studio.denials = idx.Denials
			if idx.Prefs.Channel == core.ChannelCustom || idx.Prefs.Channel == core.ChannelCheapRouter {
				studio.state.Channel = idx.Prefs.Channel
			}
			if idx.Prefs.Model != "" {
				studio.state.Model = idx.Prefs.Model
			}
			if idx.Prefs.Size != "" {
				studio.state.Size = idx.Prefs.Size
			}
			if idx.Prefs.Quality != "" {
				studio.state.Quality = idx.Prefs.Quality
			}
			if idx.Prefs.Count >= 1 && idx.Prefs.Count <= core.MaxCount {
				studio.state.Count = idx.Prefs.Count
			}
			if len(idx.Jobs) > 0 {
				studio.state.Jobs = append(studio.state.Jobs, idx.Jobs...)
			}
			studio.state.Loaded = true
			r.syncImageStudioCatalog()
		})
	})
}

func (r *Runtime) closeImageStudio() {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	for id, cancel := range studio.running {
		cancel()
		delete(studio.running, id)
	}
}

func (r *Runtime) syncImageStudioCatalog() {
	studio := r.imageStudio
	if studio == nil || studio.state == nil {
		return
	}
	if r.cheapAuth == nil || r.cheapAuth.state == nil {
		studio.state.CheapSignedIn = false
		studio.state.CheapLoading = false
		studio.state.CheapModels = nil
		studio.state.CheapCatalog = nil
		return
	}
	authState := r.cheapAuth.state
	studio.state.CheapSignedIn = authState.SignedIn()
	studio.state.CheapLoading = authState.Busy()
	studio.state.CheapCatalog = append([]cheapauth.Model(nil), authState.Catalog...)
	models := cheapauth.ImageCatalogModels(authState.Catalog)
	studio.state.CheapModels = models
	if len(models) > 0 {
		matched := false
		for _, m := range models {
			if strings.EqualFold(m.ID, studio.state.Model) {
				matched = true
				break
			}
		}
		if !matched {
			studio.state.Model = pickDefaultImageModel(models)
		}
	}
}

func pickDefaultImageModel(models []cheapauth.Model) string {
	if len(models) == 0 {
		return ""
	}
	for _, preferred := range []string{"gpt-image-2", "gpt-image-1.5", "gpt-image-1"} {
		for _, m := range models {
			if strings.EqualFold(m.ID, preferred) {
				return m.ID
			}
		}
	}
	return models[0].ID
}

func (r *Runtime) refreshCheapImageCatalog() {
	account := r.cheapAuth
	if account == nil || account.service == nil || !account.state.SignedIn() || account.state.Busy() || account.state.LoggingOut {
		return
	}
	generation := account.generation
	if generation == nil {
		generation = account.advance(r.ctx)
	}
	r.startCheapSync(account, generation)
}

func (r *Runtime) saveCustomImageAPI() {
	studio := r.imageStudio
	if studio == nil || studio.customStore == nil || studio.settingsState.Saving {
		return
	}
	s := studio.settingsState
	baseURL := strings.TrimSpace(s.BaseURL)
	apiKeyInput := s.APIKeyInput
	keepKey := s.HasSavedKey
	model := strings.TrimSpace(s.Model)
	extra := strings.TrimSpace(s.ExtraParams)

	if _, err := core.NormalizeBaseURL(baseURL); err != nil {
		s.ErrorKey = "settings_image_api_invalid_url"
		return
	}
	if model == "" {
		s.ErrorKey = "settings_image_api_invalid_model"
		return
	}
	if _, err := core.ParseExtraParams(extra); err != nil {
		s.ErrorKey = "settings_image_api_invalid_extra"
		return
	}

	s.Saving = true
	s.ErrorKey = ""
	r.async(func() {
		summary, err := studio.customStore.Save(baseURL, apiKeyInput, keepKey, model, extra)
		r.enqueue(func(c *ui.Context) {
			s.Saving = false
			if err != nil {
				s.ErrorKey = "settings_image_api_save_failed"
				kit.Toast(c, kit.Failed, i18n.T(c, "settings_image_api_save_failed"))
				return
			}
			s.ErrorKey = ""
			s.ApplySummary(summary, true)
			studio.state.Custom = summary
			kit.Toast(c, kit.Done, i18n.T(c, "settings_image_api_saved"))
		})
	})
}

func (r *Runtime) clearCustomImageAPIKey() {
	studio := r.imageStudio
	if studio == nil || studio.customStore == nil || studio.settingsState.Saving {
		return
	}
	s := studio.settingsState
	s.Saving = true
	s.ErrorKey = ""
	r.async(func() {
		summary, err := studio.customStore.ClearKey()
		r.enqueue(func(c *ui.Context) {
			s.Saving = false
			if err != nil {
				s.ErrorKey = "settings_image_api_save_failed"
				kit.Toast(c, kit.Failed, i18n.T(c, "settings_image_api_save_failed"))
				return
			}
			s.ApplySummary(summary, true)
			studio.state.Custom = summary
			kit.Toast(c, kit.Done, i18n.T(c, "settings_image_api_key_cleared"))
		})
	})
}

func (r *Runtime) clearCustomImageAPI() {
	studio := r.imageStudio
	if studio == nil || studio.customStore == nil || studio.settingsState.Saving {
		return
	}
	s := studio.settingsState
	s.Saving = true
	s.ErrorKey = ""
	r.async(func() {
		err := studio.customStore.Clear()
		r.enqueue(func(c *ui.Context) {
			s.Saving = false
			if err != nil {
				s.ErrorKey = "settings_image_api_save_failed"
				kit.Toast(c, kit.Failed, i18n.T(c, "settings_image_api_save_failed"))
				return
			}
			s.ApplySummary(core.CustomSummary{}, true)
			studio.state.Custom = core.CustomSummary{}
			kit.Toast(c, kit.Done, i18n.T(c, "settings_image_api_cleared"))
		})
	})
}

func (r *Runtime) submitImageStudioJob() {
	studio := r.imageStudio
	if studio == nil || !studio.state.CanSubmit() {
		return
	}
	s := studio.state
	prompt := strings.TrimSpace(s.Prompt)
	if prompt == "" {
		return
	}
	spec := core.GenerateSpec{
		Channel: s.EffectiveChannel(),
		Model:   s.EffectiveModel(),
		Prompt:  prompt,
		Size:    s.EffectiveSize(),
		Quality: s.EffectiveQuality(),
		Count:   s.EffectiveCount(),
	}
	var estimate *float64
	if est, ok := s.EstimateUSD(); ok {
		copied := est
		estimate = &copied
	}
	s.Prompt = ""
	r.pushImageJob(spec, estimate)
}

func (r *Runtime) retryImageStudioJob(jobID string) {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	for i := range studio.state.Jobs {
		if studio.state.Jobs[i].ID == jobID {
			spec := studio.state.Jobs[i].Spec
			var estimate *float64
			if studio.state.Jobs[i].EstimateUSD != nil {
				copied := *studio.state.Jobs[i].EstimateUSD
				estimate = &copied
			}
			r.pushImageJob(spec, estimate)
			return
		}
	}
}

func (r *Runtime) pushImageJob(spec core.GenerateSpec, estimate *float64) {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	now := time.Now()
	id := uuid.NewString()
	short := strings.ReplaceAll(id, "-", "")
	if len(short) > 6 {
		short = short[:6]
	}
	job := core.Job{
		ID:          id,
		CreatedAt:   now.Unix(),
		StartedAt:   now.Unix(),
		Spec:        spec,
		EstimateUSD: estimate,
		Stem:        now.Format("20060102-150405") + "-" + short,
		Folder:      now.Format("2006-01"),
		Status:      core.JobRunning,
	}
	if spec.Channel == core.ChannelCustom {
		job.Endpoint = core.DisplayHost(studio.state.Custom.BaseURL)
	}
	studio.state.Jobs = append([]core.Job{job}, studio.state.Jobs...)
	r.persistImageStudioIndex()
	r.startImageJob(job)
}

func (r *Runtime) cancelImageStudioJob(jobID string) {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	if cancel, ok := studio.running[jobID]; ok {
		cancel()
		delete(studio.running, jobID)
	}
	now := time.Now().Unix()
	for i := range studio.state.Jobs {
		if studio.state.Jobs[i].ID == jobID && studio.state.Jobs[i].Status == core.JobRunning {
			studio.state.Jobs[i].Status = core.JobCancelled
			studio.state.Jobs[i].FinishedAt = now
			studio.state.Jobs[i].Error = &core.ImageError{Kind: core.ErrCancelled, Message: "cancelled"}
			break
		}
	}
	r.persistImageStudioIndex()
}

func (r *Runtime) removeImageStudioJob(jobID string) {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	if cancel, ok := studio.running[jobID]; ok {
		cancel()
		delete(studio.running, jobID)
	}
	kept := studio.state.Jobs[:0]
	for _, job := range studio.state.Jobs {
		if job.ID == jobID {
			for _, p := range job.Outputs {
				delete(studio.state.Thumbs, p)
			}
			continue
		}
		kept = append(kept, job)
	}
	studio.state.Jobs = kept
	r.persistImageStudioIndex()
}

func (r *Runtime) startImageJob(job core.Job) {
	studio := r.imageStudio
	if studio == nil {
		return
	}
	jobCtx, cancel := context.WithCancel(r.ctx)
	studio.running[job.ID] = cancel

	outputDir := filepath.Join(core.DefaultOutputDir(studio.profileDir), job.Folder)
	catalogCopy := append([]cheapauth.Model(nil), studio.state.CheapCatalog...)
	denialsCopy := append([]core.Denial(nil), studio.denials...)
	tryAsync := !studio.asyncUnavailable

	r.async(func() {
		defer cancel()
		var (
			paths         []string
			finalErr      *core.ImageError
			winningGroup  int64
			asyncDisabled bool
			newDenials    []int64
		)

		if job.Spec.Channel == core.ChannelCustom {
			paths, finalErr = r.runCustomImageJob(jobCtx, studio, job, outputDir)
		} else {
			paths, winningGroup, asyncDisabled, newDenials, finalErr = r.runCheapRouterImageJob(
				jobCtx, studio, job, outputDir, catalogCopy, denialsCopy, tryAsync,
			)
		}

		r.enqueue(func(*ui.Context) {
			delete(studio.running, job.ID)
			if asyncDisabled {
				studio.asyncUnavailable = true
			}
			now := time.Now().Unix()
			if len(newDenials) > 0 {
				idx := core.Index{Denials: studio.denials}
				for _, gid := range newDenials {
					core.RecordDenial(&idx, job.Spec.Model, gid, now)
				}
				studio.denials = idx.Denials
			}
			for i := range studio.state.Jobs {
				if studio.state.Jobs[i].ID != job.ID {
					continue
				}
				if studio.state.Jobs[i].Status != core.JobRunning {
					break
				}
				studio.state.Jobs[i].FinishedAt = now
				if winningGroup > 0 {
					studio.state.Jobs[i].GroupID = winningGroup
				}
				if finalErr != nil {
					if finalErr.Kind == core.ErrCancelled {
						studio.state.Jobs[i].Status = core.JobCancelled
					} else {
						studio.state.Jobs[i].Status = core.JobFailed
					}
					studio.state.Jobs[i].Error = finalErr
				} else {
					studio.state.Jobs[i].Status = core.JobDone
					studio.state.Jobs[i].Outputs = paths
					studio.state.Jobs[i].Error = nil
					for _, path := range paths {
						studio.state.Thumbs[path] = imagestudioui.Thumb{Status: imagestudioui.ThumbLoading}
						r.loadStudioThumb(path)
					}
				}
				break
			}
			r.persistImageStudioIndex()
		})
	})
}

func (r *Runtime) runCustomImageJob(ctx context.Context, studio *imageStudioRuntime, job core.Job, outputDir string) ([]string, *core.ImageError) {
	if studio.customStore == nil {
		return nil, &core.ImageError{Kind: core.ErrBadRequest, Message: "custom image API is not configured"}
	}
	cfg, err := studio.customStore.Load()
	if err != nil || !cfg.Configured() {
		return nil, &core.ImageError{Kind: core.ErrBadRequest, Message: "custom image API is not configured"}
	}
	outputs, genErr := studio.client.GenerateCustom(ctx, cfg, job.Spec)
	if genErr != nil {
		return nil, genErr
	}
	return studio.client.SaveOutputs(ctx, outputs, outputDir, job.Stem)
}

func (r *Runtime) runCheapRouterImageJob(
	ctx context.Context,
	studio *imageStudioRuntime,
	job core.Job,
	outputDir string,
	catalog []cheapauth.Model,
	denials []core.Denial,
	tryAsync bool,
) ([]string, int64, bool, []int64, *core.ImageError) {
	if r.cheapAuth == nil || r.cheapAuth.service == nil || !r.cheapAuth.service.SignedIn() {
		return nil, 0, false, nil, &core.ImageError{Kind: core.ErrUnauthorized, Message: "not signed in to CheapRouter"}
	}
	candidates := r.cheapAuth.service.ImageRouteCandidates(catalog, job.Spec.Model)
	now := time.Now().Unix()
	tempIdx := core.Index{Denials: denials}

	var filtered []cheapauth.ImageGroup
	for _, g := range candidates {
		if !core.IsDenied(&tempIdx, job.Spec.Model, g.GroupID, now) {
			filtered = append(filtered, g)
		}
	}
	if len(filtered) > 0 {
		candidates = filtered
	}
	if len(candidates) == 0 {
		// Fall back to account default key when catalog carries no explicit group ID.
		candidates = []cheapauth.ImageGroup{{GroupID: 0}}
	}

	var (
		lastErr       *core.ImageError
		newDenials    []int64
		asyncDisabled bool
	)
	for _, group := range candidates {
		if ctx.Err() != nil {
			return nil, 0, asyncDisabled, newDenials, &core.ImageError{Kind: core.ErrCancelled, Message: "cancelled"}
		}
		var gidPtr *int64
		if group.GroupID > 0 {
			gid := group.GroupID
			gidPtr = &gid
		}
		groupName := group.Name
		groupID := group.GroupID
		r.enqueue(func(*ui.Context) {
			for i := range studio.state.Jobs {
				if studio.state.Jobs[i].ID == job.ID && studio.state.Jobs[i].Status == core.JobRunning {
					studio.state.Jobs[i].GroupID = groupID
					studio.state.Jobs[i].GroupName = groupName
					break
				}
			}
		})

		origin, apiKey, keyErr := r.cheapAuth.service.ImageRouteKey(ctx, gidPtr)
		if keyErr != nil {
			if errors.Is(keyErr, context.Canceled) {
				return nil, 0, asyncDisabled, newDenials, &core.ImageError{Kind: core.ErrCancelled, Message: "cancelled"}
			}
			if cheapAuthRejected(keyErr) {
				return nil, 0, asyncDisabled, newDenials, &core.ImageError{Kind: core.ErrUnauthorized, Message: "CheapRouter session expired"}
			}
			lastErr = &core.ImageError{Kind: core.ErrNetwork, Message: "failed to resolve CheapRouter gateway key"}
			continue
		}

		onTaskID := func(taskID string) {
			r.enqueue(func(*ui.Context) {
				for i := range studio.state.Jobs {
					if studio.state.Jobs[i].ID == job.ID && studio.state.Jobs[i].Status == core.JobRunning {
						studio.state.Jobs[i].TaskID = taskID
						break
					}
				}
			})
		}

		outputs, disabled, genErr := studio.client.GenerateCheapRouter(ctx, origin, apiKey, job.Spec, tryAsync && !asyncDisabled, onTaskID)
		if disabled {
			asyncDisabled = true
		}
		if genErr == nil {
			paths, saveErr := studio.client.SaveOutputs(ctx, outputs, outputDir, job.Stem)
			if saveErr != nil {
				return nil, group.GroupID, asyncDisabled, newDenials, saveErr
			}
			if group.GroupID > 0 {
				_ = r.cheapAuth.service.RememberImageGroup(r.ctx, job.Spec.Model, group.GroupID)
			}
			return paths, group.GroupID, asyncDisabled, newDenials, nil
		}
		if genErr.Kind == core.ErrCancelled {
			return nil, 0, asyncDisabled, newDenials, genErr
		}
		if genErr.ShouldTryNextGroup() && group.GroupID > 0 {
			newDenials = append(newDenials, group.GroupID)
			lastErr = genErr
			continue
		}
		return nil, group.GroupID, asyncDisabled, newDenials, genErr
	}
	if lastErr == nil {
		lastErr = &core.ImageError{Kind: core.ErrNoImagePermission, Message: "no group can generate images for this model"}
	}
	return nil, 0, asyncDisabled, newDenials, lastErr
}

func (r *Runtime) persistImageStudioIndex() {
	studio := r.imageStudio
	if studio == nil || studio.indexStore == nil || !studio.state.Loaded {
		return
	}
	s := studio.state
	idx := core.Index{
		Prefs: core.Prefs{
			Channel: s.EffectiveChannel(),
			Model:   s.Model,
			Size:    s.Size,
			Quality: s.Quality,
			Count:   s.EffectiveCount(),
		},
		Denials: append([]core.Denial(nil), studio.denials...),
		Jobs:    append([]core.Job(nil), s.Jobs...),
	}
	studio.saveMu.Lock()
	studio.saveSeq++
	seq := studio.saveSeq
	studio.saveMu.Unlock()

	r.async(func() {
		studio.saveMu.Lock()
		defer studio.saveMu.Unlock()
		if seq < studio.saveSeq {
			return
		}
		_ = studio.indexStore.Save(idx)
	})
}

func (r *Runtime) loadStudioThumb(path string) {
	studio := r.imageStudio
	if studio == nil || strings.TrimSpace(path) == "" {
		return
	}
	r.async(func() {
		data, err := os.ReadFile(path)
		if err != nil {
			r.enqueue(func(*ui.Context) {
				studio.state.Thumbs[path] = imagestudioui.Thumb{Status: imagestudioui.ThumbMissing}
			})
			return
		}
		decoded, err := ui.DecodeImage(data)
		if err != nil {
			r.enqueue(func(*ui.Context) {
				studio.state.Thumbs[path] = imagestudioui.Thumb{Status: imagestudioui.ThumbMissing}
			})
			return
		}
		r.enqueue(func(*ui.Context) {
			studio.state.Thumbs[path] = imagestudioui.Thumb{Status: imagestudioui.ThumbReady, Image: decoded}
		})
	})
}

func (r *Runtime) saveStudioImageAs(srcPath string) {
	if r.attachments == nil || r.attachments.native.SaveFile == nil || strings.TrimSpace(srcPath) == "" {
		return
	}
	r.attachments.native.SaveFile(filepath.Base(srcPath), func(dstPath string) {
		if strings.TrimSpace(dstPath) == "" {
			return
		}
		r.async(func() {
			err := copyFileAtomic(srcPath, dstPath)
			r.enqueue(func(c *ui.Context) {
				if err != nil {
					kit.Toast(c, kit.Failed, i18n.T(c, "image_studio_save_failed"))
					return
				}
				kit.Toast(c, kit.Done, i18n.T(c, "image_studio_saved_toast"))
			})
		})
	})
}

func (r *Runtime) openStudioImage(path string) {
	if r.attachments != nil && r.attachments.native.OpenLocal != nil && strings.TrimSpace(path) != "" {
		r.attachments.native.OpenLocal(path)
	}
}

func (r *Runtime) revealStudioImage(path string) {
	if r.attachments != nil && r.attachments.native.RevealLocal != nil && strings.TrimSpace(path) != "" {
		r.attachments.native.RevealLocal(path)
	}
}

func (r *Runtime) copyStudioImage(path string) {
	if r.attachments == nil || r.attachments.native.CopyImage == nil || strings.TrimSpace(path) == "" {
		return
	}
	copyFn := r.attachments.native.CopyImage
	r.async(func() {
		data, err := os.ReadFile(path)
		if err != nil {
			return
		}
		mime := "image/png"
		switch core.SniffExtension(data) {
		case "jpg":
			mime = "image/jpeg"
		case "webp":
			mime = "image/webp"
		case "gif":
			mime = "image/gif"
		}
		copyFn(data, mime)
		r.enqueue(func(c *ui.Context) { kit.Copied(c) })
	})
}

func (r *Runtime) openStudioOutputFolder() {
	studio := r.imageStudio
	if studio == nil || r.attachments == nil || r.attachments.native.OpenLocal == nil {
		return
	}
	dir := core.DefaultOutputDir(studio.profileDir)
	openFn := r.attachments.native.OpenLocal
	r.async(func() {
		_ = os.MkdirAll(dir, 0o755)
		r.enqueue(func(*ui.Context) { openFn(dir) })
	})
}

func copyFileAtomic(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(dst), ".image-save-*")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := io.Copy(tmp, in); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(tmpName, dst)
}
