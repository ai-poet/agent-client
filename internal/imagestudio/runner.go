package imagestudio

import (
	"context"
	"errors"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
)

// AuthProvider abstracts the CheapRouter credential and routing operations
// required by Runner. *cheapauth.Service satisfies this interface.
type AuthProvider interface {
	SignedIn() bool
	ImageRouteCandidates(catalog []cheapauth.Model, model string) []cheapauth.ImageGroup
	ImageRouteKey(ctx context.Context, groupID *int64) (origin, apiKey string, err error)
	RememberImageGroup(ctx context.Context, model string, groupID int64) error
}

// RunOptions customizes one Runner execution.
type RunOptions struct {
	Catalog        []cheapauth.Model
	Denials        []Denial
	PersistDenials bool
	CustomConfig   *CustomConfig
	OnGroup        func(groupID int64, groupName string)
	OnTaskID       func(taskID string)
}

// RunResult captures outputs and routing metadata from one Runner execution.
type RunResult struct {
	Outputs       []ImageOutput
	Channel       Channel
	Model         string
	GroupID       int64
	GroupName     string
	Endpoint      string
	TaskID        string
	NewDenials    []int64
	AsyncDisabled bool
}

// Runner coordinates image generation and editing across CheapRouter
// (candidate group probing, 24h Denial cache, RememberImageGroup, async task
// polling with SSE/JSON fallback) and custom OpenAI-compatible endpoints.
// Desktop Creative Studio and Node's generate_image tool share this runner.
type Runner struct {
	client      *Client
	auth        AuthProvider
	customStore *CustomStore
	indexStore  *IndexStore
	now         func() time.Time

	mu               sync.Mutex
	asyncUnavailable bool
}

// NewRunner constructs a shared image orchestration runner.
func NewRunner(client *Client, auth AuthProvider, customStore *CustomStore, indexStore *IndexStore) *Runner {
	if client == nil {
		client = NewClient()
	}
	return &Runner{
		client:      client,
		auth:        auth,
		customStore: customStore,
		indexStore:  indexStore,
		now:         time.Now,
	}
}

// Client returns the underlying HTTP client.
func (r *Runner) Client() *Client { return r.client }

// AsyncUnavailable reports whether this runner has observed that the gateway
// does not support /async image tasks.
func (r *Runner) AsyncUnavailable() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.asyncUnavailable
}

// SetAsyncUnavailable seeds or resets the async-unavailable memo.
func (r *Runner) SetAsyncUnavailable(v bool) {
	r.mu.Lock()
	r.asyncUnavailable = v
	r.mu.Unlock()
}

// Run executes spec on spec.Channel (ChannelCheapRouter or ChannelCustom).
func (r *Runner) Run(ctx context.Context, spec GenerateSpec, opts RunOptions) (RunResult, *ImageError) {
	if spec.Channel == ChannelCustom {
		return r.runCustom(ctx, spec, opts)
	}
	return r.runCheapRouter(ctx, spec, opts)
}

func (r *Runner) runCustom(ctx context.Context, spec GenerateSpec, opts RunOptions) (RunResult, *ImageError) {
	var cfg CustomConfig
	if opts.CustomConfig != nil {
		cfg = *opts.CustomConfig
	} else {
		if r.customStore == nil {
			return RunResult{}, &ImageError{Kind: ErrBadRequest, Message: "custom image API is not configured"}
		}
		loaded, err := r.customStore.Load()
		if err != nil {
			return RunResult{}, &ImageError{Kind: ErrBadRequest, Message: "custom image API is not configured"}
		}
		cfg = loaded
	}
	if !cfg.Configured() {
		return RunResult{}, &ImageError{Kind: ErrBadRequest, Message: "custom image API is not configured"}
	}
	model := strings.TrimSpace(spec.Model)
	if model == "" {
		model = strings.TrimSpace(cfg.Model)
	}
	spec.Model = model
	outputs, genErr := r.client.GenerateCustom(ctx, cfg, spec)
	if genErr != nil {
		return RunResult{
			Channel:  ChannelCustom,
			Model:    model,
			Endpoint: DisplayHost(cfg.BaseURL),
		}, genErr
	}
	return RunResult{
		Outputs:  outputs,
		Channel:  ChannelCustom,
		Model:    model,
		Endpoint: DisplayHost(cfg.BaseURL),
	}, nil
}

func (r *Runner) runCheapRouter(ctx context.Context, spec GenerateSpec, opts RunOptions) (RunResult, *ImageError) {
	if r.auth == nil || !r.auth.SignedIn() {
		return RunResult{}, &ImageError{Kind: ErrUnauthorized, Message: "not signed in to CheapRouter"}
	}
	model := strings.TrimSpace(spec.Model)
	if model == "" {
		model = PickDefaultImageModel(cheapauth.ImageCatalogModels(opts.Catalog))
	}
	spec.Model = model

	now := r.now()
	nowUnix := now.Unix()
	denials := opts.Denials
	if denials == nil && r.indexStore != nil {
		if loaded, err := r.indexStore.LoadDenials(now); err == nil {
			denials = loaded
		}
	}

	candidates := r.auth.ImageRouteCandidates(opts.Catalog, model)
	tempIdx := Index{Denials: denials}
	var filtered []cheapauth.ImageGroup
	for _, g := range candidates {
		if !IsDenied(&tempIdx, model, g.GroupID, nowUnix) {
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

	tryAsync := !r.AsyncUnavailable()
	var (
		lastErr       *ImageError
		newDenials    []int64
		asyncDisabled bool
	)
	recordDenials := func() {
		if opts.PersistDenials && r.indexStore != nil && len(newDenials) > 0 {
			_, _ = r.indexStore.RecordDenials(model, newDenials, r.now())
		}
	}

	for _, group := range candidates {
		if ctx.Err() != nil {
			recordDenials()
			return RunResult{
				Channel:       ChannelCheapRouter,
				Model:         model,
				NewDenials:    newDenials,
				AsyncDisabled: asyncDisabled,
			}, ctxImageError(ctx.Err())
		}
		var gidPtr *int64
		if group.GroupID > 0 {
			gid := group.GroupID
			gidPtr = &gid
		}
		if opts.OnGroup != nil {
			opts.OnGroup(group.GroupID, group.Name)
		}

		origin, apiKey, keyErr := r.auth.ImageRouteKey(ctx, gidPtr)
		if keyErr != nil {
			if errors.Is(keyErr, context.Canceled) {
				recordDenials()
				return RunResult{
					Channel:       ChannelCheapRouter,
					Model:         model,
					NewDenials:    newDenials,
					AsyncDisabled: asyncDisabled,
				}, &ImageError{Kind: ErrCancelled, Message: "cancelled"}
			}
			if errors.Is(keyErr, context.DeadlineExceeded) {
				recordDenials()
				return RunResult{
					Channel:       ChannelCheapRouter,
					Model:         model,
					NewDenials:    newDenials,
					AsyncDisabled: asyncDisabled,
				}, &ImageError{Kind: ErrTimeout, Message: "request timed out"}
			}
			if errors.Is(keyErr, cheapauth.ErrRefresh) || errors.Is(keyErr, cheapauth.ErrUnauthorized) {
				recordDenials()
				return RunResult{
					Channel:       ChannelCheapRouter,
					Model:         model,
					NewDenials:    newDenials,
					AsyncDisabled: asyncDisabled,
				}, &ImageError{Kind: ErrUnauthorized, Message: "CheapRouter session expired"}
			}
			lastErr = &ImageError{Kind: ErrNetwork, Message: "failed to resolve CheapRouter gateway key"}
			continue
		}

		var taskID string
		onTaskID := func(id string) {
			taskID = id
			if opts.OnTaskID != nil {
				opts.OnTaskID(id)
			}
		}

		outputs, disabled, genErr := r.client.GenerateCheapRouter(ctx, origin, apiKey, spec, tryAsync && !asyncDisabled, onTaskID)
		if disabled {
			asyncDisabled = true
			r.SetAsyncUnavailable(true)
		}
		if genErr == nil {
			recordDenials()
			if group.GroupID > 0 {
				_ = r.auth.RememberImageGroup(context.WithoutCancel(ctx), model, group.GroupID)
			}
			return RunResult{
				Outputs:       outputs,
				Channel:       ChannelCheapRouter,
				Model:         model,
				GroupID:       group.GroupID,
				GroupName:     group.Name,
				Endpoint:      DisplayHost(origin),
				TaskID:        taskID,
				NewDenials:    newDenials,
				AsyncDisabled: asyncDisabled,
			}, nil
		}
		if taskID != "" && genErr.TaskID == "" {
			genErr.TaskID = taskID
		}
		if genErr.Kind == ErrCancelled {
			recordDenials()
			return RunResult{
				Channel:       ChannelCheapRouter,
				Model:         model,
				GroupID:       group.GroupID,
				GroupName:     group.Name,
				TaskID:        taskID,
				NewDenials:    newDenials,
				AsyncDisabled: asyncDisabled,
			}, genErr
		}
		// Never retry another group if an async task was already accepted and
		// its server-side outcome is uncertain.
		if taskID != "" && !genErr.TaskEnded {
			recordDenials()
			return RunResult{
				Channel:       ChannelCheapRouter,
				Model:         model,
				GroupID:       group.GroupID,
				GroupName:     group.Name,
				TaskID:        taskID,
				NewDenials:    newDenials,
				AsyncDisabled: asyncDisabled,
			}, genErr
		}
		if genErr.ShouldTryNextGroup() && group.GroupID > 0 {
			newDenials = append(newDenials, group.GroupID)
			lastErr = genErr
			continue
		}
		recordDenials()
		return RunResult{
			Channel:       ChannelCheapRouter,
			Model:         model,
			GroupID:       group.GroupID,
			GroupName:     group.Name,
			TaskID:        taskID,
			NewDenials:    newDenials,
			AsyncDisabled: asyncDisabled,
		}, genErr
	}

	recordDenials()
	if lastErr == nil {
		lastErr = &ImageError{Kind: ErrNoImagePermission, Message: "no group can generate images for this model"}
	}
	return RunResult{
		Channel:       ChannelCheapRouter,
		Model:         model,
		NewDenials:    newDenials,
		AsyncDisabled: asyncDisabled,
	}, lastErr
}

func ctxImageError(err error) *ImageError {
	if errors.Is(err, context.DeadlineExceeded) {
		return &ImageError{Kind: ErrTimeout, Message: "request timed out"}
	}
	return &ImageError{Kind: ErrCancelled, Message: "cancelled"}
}
