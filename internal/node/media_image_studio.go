package node

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"path"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/engine"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"lukechampine.com/blake3"
)

const (
	imageCatalogTTL        = 5 * time.Minute
	imageCatalogFetchLimit = 8 * time.Second
	maxImageReferences     = 8
)

// ImageStudioOptions injects image backend dependencies for deterministic tests
// or shared in-process services. Nil fields default to profile-backed stores.
type ImageStudioOptions struct {
	Client        *imagestudio.Client
	Auth          imagestudio.AuthProvider
	Catalog       func(context.Context) ([]cheapauth.Model, error)
	StaticCatalog []cheapauth.Model
	CustomStore   *imagestudio.CustomStore
	IndexStore    *imagestudio.IndexStore
}

type imageChannelKind string

const (
	imageChannelAuto        imageChannelKind = "auto"
	imageChannelMedia       imageChannelKind = "media"
	imageChannelCheapRouter imageChannelKind = "cheaprouter"
	imageChannelCustom      imageChannelKind = "custom"
)

type imageCallOptions struct {
	channel          imageChannelKind
	model            string
	size             string
	quality          string
	count            int
	references       []string
	explicitFileName bool
}

type imageBackendState struct {
	mediaAssigned      bool
	mediaAssignedRoute mediaRoute
	mediaAssignedValid bool

	cheapAvailable bool
	cheapDefault   string
	cheapModels    []string
	cheapCatalog   []cheapauth.Model

	customAvailable bool
	customSummary   imagestudio.CustomSummary

	convRoute     mediaRoute
	convAvailable bool

	defaultChannel imageChannelKind
}

func (s imageBackendState) offered() bool {
	return s.defaultChannel != ""
}

func (s imageBackendState) descriptionSuffix() string {
	if !s.offered() {
		return ""
	}
	var channels []string
	if s.mediaAssignedValid {
		channels = append(channels, fmt.Sprintf("media (assigned model=%s)", s.mediaAssignedRoute.model))
	}
	if s.cheapAvailable {
		if len(s.cheapModels) > 0 {
			shown := s.cheapModels
			if len(shown) > 12 {
				shown = shown[:12]
			}
			channels = append(channels, fmt.Sprintf("cheaprouter (default=%s, models=%s)", s.cheapDefault, strings.Join(shown, ", ")))
		} else {
			channels = append(channels, fmt.Sprintf("cheaprouter (default=%s)", s.cheapDefault))
		}
	}
	if s.customAvailable {
		host := imagestudio.DisplayHost(s.customSummary.BaseURL)
		if host != "" {
			channels = append(channels, fmt.Sprintf("custom (host=%s, model=%s)", host, s.customSummary.Model))
		} else {
			channels = append(channels, fmt.Sprintf("custom (model=%s)", s.customSummary.Model))
		}
	}
	if s.convAvailable {
		channels = append(channels, fmt.Sprintf("media (conversation model=%s)", s.convRoute.model))
	}
	if len(channels) == 0 {
		return ""
	}
	return fmt.Sprintf("\nAvailable image backends for this turn: default=%s; channels: %s.", s.defaultChannel, strings.Join(channels, "; "))
}

type nodeImageStudio struct {
	auth         imagestudio.AuthProvider
	cheapService *cheapauth.Service
	catalogFn    func(context.Context) ([]cheapauth.Model, error)
	customStore  *imagestudio.CustomStore
	indexStore   *imagestudio.IndexStore
	runner       *imagestudio.Runner

	mu               sync.Mutex
	cachedCatalog    []cheapauth.Model
	catalogFetchedAt time.Time
	catalogEndpoint  string
	staticCatalog    bool
}

func newNodeImageStudio(profileDir string, opts *ImageStudioOptions) *nodeImageStudio {
	var (
		client        *imagestudio.Client
		auth          imagestudio.AuthProvider
		cheapService  *cheapauth.Service
		catalogFn     func(context.Context) ([]cheapauth.Model, error)
		customStore   *imagestudio.CustomStore
		indexStore    *imagestudio.IndexStore
		staticCatalog []cheapauth.Model
	)
	if opts != nil {
		client = opts.Client
		auth = opts.Auth
		catalogFn = opts.Catalog
		customStore = opts.CustomStore
		indexStore = opts.IndexStore
		staticCatalog = append([]cheapauth.Model(nil), opts.StaticCatalog...)
	}
	if client == nil {
		client = imagestudio.NewClient()
	}
	if customStore == nil && strings.TrimSpace(profileDir) != "" {
		customStore, _ = imagestudio.NewCustomStore(profileDir)
	}
	if indexStore == nil && strings.TrimSpace(profileDir) != "" {
		indexStore, _ = imagestudio.NewIndexStore(profileDir)
	}
	if auth == nil && strings.TrimSpace(profileDir) != "" {
		if svc, err := cheapauth.NewService(profileDir); err == nil {
			_ = svc.Restore()
			cheapService = svc
			auth = svc
			if catalogFn == nil {
				catalogFn = svc.ModelCatalog
			}
		}
	} else if svc, ok := auth.(*cheapauth.Service); ok {
		cheapService = svc
		if catalogFn == nil {
			catalogFn = svc.ModelCatalog
		}
	}
	runner := imagestudio.NewRunner(client, auth, customStore, indexStore)
	studio := &nodeImageStudio{
		auth:         auth,
		cheapService: cheapService,
		catalogFn:    catalogFn,
		customStore:  customStore,
		indexStore:   indexStore,
		runner:       runner,
	}
	if len(staticCatalog) > 0 {
		studio.cachedCatalog = staticCatalog
		studio.staticCatalog = true
	}
	return studio
}

func (s *nodeImageStudio) reloadAuth() {
	if s == nil || s.cheapService == nil {
		return
	}
	_ = s.cheapService.Restore()
}

func (s *nodeImageStudio) snapshotCatalog() []cheapauth.Model {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.cheapService != nil && !s.staticCatalog {
		endpoint := s.cheapService.Endpoint()
		if endpoint != s.catalogEndpoint {
			s.cachedCatalog = nil
			s.catalogFetchedAt = time.Time{}
			s.catalogEndpoint = endpoint
		}
	}
	return append([]cheapauth.Model(nil), s.cachedCatalog...)
}

func (s *nodeImageStudio) resolveCatalog(ctx context.Context) []cheapauth.Model {
	if s == nil {
		return nil
	}
	s.mu.Lock()
	if s.staticCatalog {
		out := append([]cheapauth.Model(nil), s.cachedCatalog...)
		s.mu.Unlock()
		return out
	}
	endpoint := ""
	if s.cheapService != nil {
		endpoint = s.cheapService.Endpoint()
		if endpoint != s.catalogEndpoint {
			s.cachedCatalog = nil
			s.catalogFetchedAt = time.Time{}
			s.catalogEndpoint = endpoint
		}
	}
	fresh := len(s.cachedCatalog) > 0 && time.Since(s.catalogFetchedAt) < imageCatalogTTL
	cached := append([]cheapauth.Model(nil), s.cachedCatalog...)
	fn := s.catalogFn
	s.mu.Unlock()

	if fresh || fn == nil || ctx.Err() != nil {
		return cached
	}
	fetchCtx, cancel := context.WithTimeout(ctx, imageCatalogFetchLimit)
	defer cancel()
	fetched, err := fn(fetchCtx)
	if err != nil {
		return cached
	}
	s.mu.Lock()
	s.cachedCatalog = append([]cheapauth.Model(nil), fetched...)
	s.catalogFetchedAt = time.Now()
	s.catalogEndpoint = endpoint
	s.mu.Unlock()
	return fetched
}

// captureImageBackend evaluates all image backends according to section 3.1
// priority without blocking turn admission on network calls.
func (node *Node) captureImageBackend(inv *store.AgentInvocation) imageBackendState {
	var state imageBackendState
	if inv != nil {
		_, state.mediaAssigned = inv.Media[protocol.MediaKindImage]
		if state.mediaAssigned {
			if route, ok := routeMedia(inv, protocol.MediaKindImage); ok {
				state.mediaAssignedValid = true
				state.mediaAssignedRoute = route
			}
		} else {
			if route, ok := routeMedia(inv, protocol.MediaKindImage); ok {
				state.convAvailable = true
				state.convRoute = route
			}
		}
	}
	if studio := node.imageStudio; studio != nil {
		studio.reloadAuth()
		if studio.auth != nil && studio.auth.SignedIn() {
			state.cheapAvailable = true
			catalog := studio.snapshotCatalog()
			state.cheapCatalog = catalog
			imgModels := cheapauth.ImageCatalogModels(catalog)
			state.cheapDefault = imagestudio.PickDefaultImageModel(imgModels)
			for _, m := range imgModels {
				state.cheapModels = append(state.cheapModels, m.ID)
			}
		}
		if studio.customStore != nil {
			if summary, err := studio.customStore.Summary(); err == nil && summary.Configured {
				state.customAvailable = true
				state.customSummary = summary
			}
		}
	}

	switch {
	case state.mediaAssignedValid:
		state.defaultChannel = imageChannelMedia
	case state.cheapAvailable:
		state.defaultChannel = imageChannelCheapRouter
	case state.customAvailable:
		state.defaultChannel = imageChannelCustom
	case state.convAvailable:
		state.defaultChannel = imageChannelMedia
	}
	return state
}

// resolveImageRoute applies the strict channel/model resolution priority:
//  1. Explicit channel or model override (never silently switches to another paid channel)
//  2. Explicit Settings -> Media models -> Image assignment
//  3. CheapRouter signed-in auto-configured image API
//  4. Custom Image API (<profileDir>/image-api.json)
//  5. Conversation model native image capability
func (state imageBackendState) resolveImageRoute(requestedChannel imageChannelKind, requestedModel string) (imageChannelKind, string, mediaRoute, error) {
	modelName := strings.TrimSpace(requestedModel)
	ch := requestedChannel
	if ch == "" {
		ch = imageChannelAuto
	}

	// 1. Explicit channel specified by caller.
	if ch != imageChannelAuto {
		switch ch {
		case imageChannelCheapRouter:
			if !state.cheapAvailable {
				return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, "CheapRouter is not signed in; sign in to CheapRouter or omit channel to use another configured image backend")
			}
			if modelName == "" {
				modelName = state.cheapDefault
				if modelName == "" {
					modelName = imagestudio.DefaultCheapRouterModel
				}
			}
			return imageChannelCheapRouter, modelName, mediaRoute{}, nil
		case imageChannelCustom:
			if !state.customAvailable {
				return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, "custom Image API is not configured; configure it in Settings -> Image API or omit channel")
			}
			if modelName == "" {
				modelName = state.customSummary.Model
			}
			return imageChannelCustom, modelName, mediaRoute{}, nil
		case imageChannelMedia:
			route, ok := state.selectMediaRoute(modelName)
			if !ok {
				if modelName != "" {
					return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, fmt.Sprintf("image model %q is not available on the configured Media provider", modelName))
				}
				return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, "no Media model is assigned for Image and the conversation model does not support native image generation")
			}
			return imageChannelMedia, route.model, route, nil
		default:
			return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeInvalidRequest, "unsupported image channel")
		}
	}

	// 1b. Explicit model specified without an explicit channel: match the
	// channel that owns that model, without silently routing to an unrelated
	// paid backend.
	if modelName != "" {
		if state.mediaAssignedValid && strings.EqualFold(state.mediaAssignedRoute.model, modelName) {
			return imageChannelMedia, state.mediaAssignedRoute.model, state.mediaAssignedRoute, nil
		}
		if state.cheapAvailable && containsFold(state.cheapModels, modelName) {
			return imageChannelCheapRouter, modelName, mediaRoute{}, nil
		}
		if state.customAvailable && strings.EqualFold(state.customSummary.Model, modelName) {
			return imageChannelCustom, state.customSummary.Model, mediaRoute{}, nil
		}
		if state.convAvailable && strings.EqualFold(state.convRoute.model, modelName) {
			return imageChannelMedia, state.convRoute.model, state.convRoute, nil
		}
		if cheapauth.IsImageModelName(modelName) {
			if state.cheapAvailable {
				return imageChannelCheapRouter, modelName, mediaRoute{}, nil
			}
			if state.customAvailable {
				return imageChannelCustom, modelName, mediaRoute{}, nil
			}
			return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, fmt.Sprintf("requested image model %q requires CheapRouter sign-in or a custom Image API configuration", modelName))
		}
		if route, ok := state.selectMediaRoute(modelName); ok {
			return imageChannelMedia, route.model, route, nil
		}
		if state.defaultChannel == imageChannelCustom {
			return imageChannelCustom, modelName, mediaRoute{}, nil
		}
		if state.defaultChannel == imageChannelCheapRouter {
			return imageChannelCheapRouter, modelName, mediaRoute{}, nil
		}
		return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, fmt.Sprintf("requested image model %q is not available on any configured image channel", modelName))
	}

	// 2-5. Default priority resolution.
	switch state.defaultChannel {
	case imageChannelMedia:
		if state.mediaAssignedValid {
			return imageChannelMedia, state.mediaAssignedRoute.model, state.mediaAssignedRoute, nil
		}
		if state.convAvailable {
			return imageChannelMedia, state.convRoute.model, state.convRoute, nil
		}
	case imageChannelCheapRouter:
		modelName = state.cheapDefault
		if modelName == "" {
			modelName = imagestudio.DefaultCheapRouterModel
		}
		return imageChannelCheapRouter, modelName, mediaRoute{}, nil
	case imageChannelCustom:
		return imageChannelCustom, state.customSummary.Model, mediaRoute{}, nil
	}
	return "", "", mediaRoute{}, mediaFault(protocol.ErrorCodeNotConfigured, "no image generation backend is configured; sign in to CheapRouter or configure Settings -> Image API / Media models")
}

func (state imageBackendState) selectMediaRoute(modelName string) (mediaRoute, bool) {
	if modelName == "" {
		if state.mediaAssignedValid {
			return state.mediaAssignedRoute, true
		}
		if state.convAvailable {
			return state.convRoute, true
		}
		return mediaRoute{}, false
	}
	if state.mediaAssignedValid && strings.EqualFold(state.mediaAssignedRoute.model, modelName) {
		return state.mediaAssignedRoute, true
	}
	if state.convAvailable && strings.EqualFold(state.convRoute.model, modelName) {
		return state.convRoute, true
	}
	return mediaRoute{}, false
}

func containsFold(items []string, target string) bool {
	for _, item := range items {
		if strings.EqualFold(strings.TrimSpace(item), strings.TrimSpace(target)) {
			return true
		}
	}
	return false
}

func validateImageOptions(input mediaArguments) (imageCallOptions, error) {
	opts := imageCallOptions{
		channel:          imageChannelAuto,
		count:            1,
		explicitFileName: input.FileName != nil,
	}
	if input.Channel != nil {
		ch := imageChannelKind(strings.ToLower(strings.TrimSpace(*input.Channel)))
		switch ch {
		case "", imageChannelAuto, imageChannelCheapRouter, imageChannelCustom, imageChannelMedia:
			if ch != "" {
				opts.channel = ch
			}
		default:
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, "channel must be one of auto, cheaprouter, custom, or media")
		}
	}
	if input.Model != nil {
		m := strings.TrimSpace(*input.Model)
		if len(m) > 128 || strings.ContainsFunc(m, func(r rune) bool { return r < 0x20 || r == 0x7f }) {
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, "image model name is invalid")
		}
		opts.model = m
	}
	if input.Size != nil {
		sz := strings.TrimSpace(*input.Size)
		if sz != "" && !validImageSize(sz) {
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, "image size must be auto or WxH (e.g. 1024x1024)")
		}
		opts.size = sz
	}
	if input.Quality != nil {
		q := strings.ToLower(strings.TrimSpace(*input.Quality))
		switch q {
		case "", "auto", "low", "medium", "high", "standard", "hd":
			opts.quality = q
		default:
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, "image quality must be one of auto, low, medium, high, standard, or hd")
		}
	}
	if input.Count != nil {
		if *input.Count < 1 || *input.Count > imagestudio.MaxCount {
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("image count must be between 1 and %d", imagestudio.MaxCount))
		}
		opts.count = *input.Count
	}
	if input.References != nil {
		if len(input.References) == 0 || len(input.References) > maxImageReferences {
			return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("references must contain between 1 and %d items", maxImageReferences))
		}
		refs := make([]string, 0, len(input.References))
		for _, raw := range input.References {
			ref := strings.TrimSpace(raw)
			if ref == "" {
				return imageCallOptions{}, mediaFault(protocol.ErrorCodeInvalidRequest, "reference image entry must not be empty")
			}
			if _, err := protocol.ParseAttachmentID(ref); err != nil {
				if err := core.CheckFilePath(ref); err != nil {
					return imageCallOptions{}, err
				}
			}
			refs = append(refs, ref)
		}
		opts.references = refs
	}
	return opts, nil
}

func validImageSize(s string) bool {
	if len(s) > 32 {
		return false
	}
	if strings.EqualFold(s, "auto") {
		return true
	}
	wStr, hStr, ok := strings.Cut(strings.ToLower(s), "x")
	if !ok {
		return false
	}
	w, errW := strconv.Atoi(wStr)
	h, errH := strconv.Atoi(hStr)
	return errW == nil && errH == nil && w >= 64 && h >= 64 && w <= 16384 && h <= 16384
}

func supportedImageOutputExtension(name string) bool {
	ext := strings.ToLower(path.Ext(name))
	switch ext {
	case ".png", ".jpg", ".jpeg", ".webp":
		return true
	default:
		return false
	}
}

func generatedImagePaths(baseTarget string, count int) []string {
	if count <= 1 {
		return []string{baseTarget}
	}
	ext := path.Ext(baseTarget)
	stem := strings.TrimSuffix(baseTarget, ext)
	paths := make([]string, count)
	for i := range count {
		paths[i] = fmt.Sprintf("%s-%d%s", stem, i+1, ext)
	}
	return paths
}

func swapExtension(target, sniffedExt string) string {
	ext := "." + strings.TrimPrefix(strings.ToLower(strings.TrimSpace(sniffedExt)), ".")
	if ext == "." || ext == ".gif" {
		return target
	}
	current := strings.ToLower(path.Ext(target))
	if current == ext {
		return target
	}
	return strings.TrimSuffix(target, path.Ext(target)) + ext
}

// loadReferenceImages loads worktree-relative image files (through RestrictedFiles)
// or conversation attachments (bound to this turn's worktree) for /v1/images/edits.
func (node *Node) loadReferenceImages(ctx context.Context, inv *store.AgentInvocation, refs []string) ([]imagestudio.ReferenceImage, error) {
	if len(refs) == 0 {
		return nil, nil
	}
	out := make([]imagestudio.ReferenceImage, 0, len(refs))
	var confined *core.Files
	defer func() {
		if confined != nil {
			_ = confined.Close()
		}
	}()
	for _, ref := range refs {
		if err := ctx.Err(); err != nil {
			return nil, mediaFault(protocol.ErrorCodeCancelled, "image edit request was cancelled")
		}
		if id, err := protocol.ParseAttachmentID(ref); err == nil {
			var attachment protocol.Attachment
			err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				var err error
				attachment, err = store.ResolveTurnAttachment(ctx, db, inv.Run.Turn, id)
				return err
			})
			if err != nil {
				return nil, err
			}
			if attachment.Spec.Worktree != inv.Run.Worktree {
				return nil, mediaFault(protocol.ErrorCodePermissionDenied, "reference attachment belongs to another worktree")
			}
			if attachment.Spec.Size > imagestudio.MaxReferenceBytes {
				return nil, mediaFault(protocol.ErrorCodeInvalidRequest, "reference attachment exceeds the 20 MiB edit limit")
			}
			data, err := node.transfers.AttachmentBytes(ctx, attachment)
			if err != nil {
				return nil, err
			}
			mime, ok := imagestudio.ReferenceMimeType(data)
			if !ok {
				return nil, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("reference attachment %s is not a supported PNG, JPEG, or WebP image", ref))
			}
			name := strings.TrimSpace(attachment.Spec.Name)
			if name == "" {
				name = "attachment-" + id.String() + "." + imagestudio.SniffExtension(data)
			}
			out = append(out, imagestudio.ReferenceImage{
				Filename: name,
				MimeType: mime,
				Data:     data,
			})
			continue
		}
		if err := core.CheckFilePath(ref); err != nil {
			return nil, err
		}
		if confined == nil {
			var err error
			confined, err = core.OpenFiles(inv.Worktree.Path, core.RestrictedFiles)
			if err != nil {
				return nil, err
			}
		}
		f, err := confined.Open(ref)
		if err != nil {
			return nil, err
		}
		data, readErr := io.ReadAll(io.LimitReader(f, imagestudio.MaxReferenceBytes+1))
		_ = f.Close()
		if readErr != nil {
			return nil, readErr
		}
		if len(data) == 0 {
			return nil, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("reference image %q is empty", ref))
		}
		if len(data) > imagestudio.MaxReferenceBytes {
			return nil, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("reference image %q exceeds the 20 MiB edit limit", ref))
		}
		mime, ok := imagestudio.ReferenceMimeType(data)
		if !ok {
			return nil, mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("reference image %q is not a supported PNG, JPEG, or WebP image", ref))
		}
		out = append(out, imagestudio.ReferenceImage{
			Filename: path.Base(ref),
			MimeType: mime,
			Data:     data,
		})
	}
	return out, nil
}

func (node *Node) generateImageViaStudio(
	ctx context.Context,
	inv *store.AgentInvocation,
	selectedChannel imageChannelKind,
	selectedModel string,
	prompt string,
	baseTarget string,
	opts imageCallOptions,
) (engine.ToolResult, error) {
	studio := node.imageStudio
	if studio == nil || studio.runner == nil {
		return engine.ToolResult{}, mediaFault(protocol.ErrorCodeNotConfigured, "image backend is not initialized")
	}

	refs, err := node.loadReferenceImages(ctx, inv, opts.references)
	if err != nil {
		return engine.ToolResult{}, err
	}

	planned := generatedImagePaths(baseTarget, opts.count)
	// Reserve every planned target before paying the provider so collisions are
	// rejected upfront.
	type reservedTarget struct {
		path string
		file *core.FileTarget
		done func()
	}
	reserved := make([]reservedTarget, 0, len(planned))
	releaseReserved := func() {
		for _, item := range reserved {
			if item.done != nil {
				item.done()
			}
		}
	}
	defer releaseReserved()

	for _, target := range planned {
		file, done, err := generatedTarget(ctx, inv.Worktree.Path, target, imagestudio.MaxDownloadBytes)
		if err != nil {
			var fault *protocol.Fault
			if errors.As(err, &fault) && fault.Code == protocol.ErrorCodeRevisionConflict {
				return engine.ToolResult{}, mediaFault(protocol.ErrorCodeConflict, "a file already exists at the media output path")
			}
			return engine.ToolResult{}, err
		}
		reserved = append(reserved, reservedTarget{path: target, file: file, done: done})
	}

	spec := imagestudio.GenerateSpec{
		Channel:    imagestudio.ChannelCheapRouter,
		Model:      selectedModel,
		Prompt:     prompt,
		Size:       opts.size,
		Quality:    opts.quality,
		Count:      opts.count,
		References: refs,
	}
	if selectedChannel == imageChannelCustom {
		spec.Channel = imagestudio.ChannelCustom
	}

	runOpts := imagestudio.RunOptions{PersistDenials: true}
	if selectedChannel == imageChannelCheapRouter {
		studio.reloadAuth()
		runOpts.Catalog = studio.resolveCatalog(ctx)
	}

	runRes, runErr := studio.runner.Run(ctx, spec, runOpts)
	if runErr != nil {
		return engine.ToolResult{}, imageStudioFault(runErr, selectedChannel)
	}
	if len(runRes.Outputs) == 0 {
		return engine.ToolResult{}, mediaFault(protocol.ErrorCodeUnavailable, "image backend returned no images")
	}

	client := studio.runner.Client()
	decoded := make([]imagestudio.DecodedImage, 0, len(runRes.Outputs))
	for _, out := range runRes.Outputs {
		img, decErr := client.DecodeOutput(ctx, out)
		if decErr != nil {
			if runRes.TaskID != "" && decErr.TaskID == "" {
				decErr.TaskID = runRes.TaskID
			}
			return engine.ToolResult{}, imageStudioFault(decErr, selectedChannel)
		}
		decoded = append(decoded, img)
	}

	// Write decoded images into confined worktree targets. If the user did not
	// name the file explicitly and the provider returned jpg/webp instead of
	// png, retarget the file extension to match the actual bytes.
	savedPaths := make([]string, 0, len(decoded))
	var revisedPrompt string
	for i, img := range decoded {
		if revisedPrompt == "" && img.RevisedPrompt != "" {
			revisedPrompt = img.RevisedPrompt
		}
		var (
			targetPath string
			file       *core.FileTarget
			done       func()
		)
		if i < len(reserved) {
			targetPath = reserved[i].path
			file = reserved[i].file
			done = reserved[i].done
			reserved[i].done = nil
		} else {
			extraPath := generatedImagePaths(baseTarget, len(decoded))[i]
			if !opts.explicitFileName {
				extraPath = swapExtension(extraPath, img.Ext)
			}
			var err error
			file, done, err = generatedTarget(ctx, inv.Worktree.Path, extraPath, imagestudio.MaxDownloadBytes)
			if err != nil {
				return engine.ToolResult{}, err
			}
			targetPath = extraPath
		}
		if !opts.explicitFileName {
			if adjusted := swapExtension(targetPath, img.Ext); adjusted != targetPath {
				done()
				newFile, newDone, err := generatedTarget(ctx, inv.Worktree.Path, adjusted, imagestudio.MaxDownloadBytes)
				if err != nil {
					return engine.ToolResult{}, err
				}
				targetPath, file, done = adjusted, newFile, newDone
			}
		}
		sum := blake3.Sum256(img.Data)
		_, writeErr := file.Write(bytes.NewReader(img.Data), uint64(len(img.Data)), hex.EncodeToString(sum[:]))
		done()
		if writeErr != nil {
			return engine.ToolResult{}, writeErr
		}
		savedPaths = append(savedPaths, targetPath)
	}
	node.watches.changed(inv.Worktree.Path, nil)

	attachments := make([]canonical.Attachment, 0, len(decoded))
	err = node.profile.Store.Node.Write(context.WithoutCancel(ctx), func(db *sql.Conn) error {
		for _, img := range decoded {
			digest, err := store.PutBlob(context.WithoutCancel(ctx), db, img.Data)
			if err != nil {
				return err
			}
			attachments = append(attachments, canonical.Attachment{
				Asset: canonical.AssetRef{
					Digest: digest,
					Mime:   img.MimeType,
					Size:   uint64(len(img.Data)),
				},
			})
		}
		return nil
	})
	if err != nil {
		return engine.ToolResult{}, mediaFault(protocol.ErrorCodeOutcomeUnknown, "image was saved at "+savedPaths[0]+" but could not be recorded for display")
	}

	providerLabel := "CheapRouter"
	if selectedChannel == imageChannelCustom {
		if runRes.Endpoint != "" {
			providerLabel = runRes.Endpoint
		} else {
			providerLabel = "Custom Image API"
		}
	}
	effectiveModel := runRes.Model
	if effectiveModel == "" {
		effectiveModel = selectedModel
	}
	value := map[string]any{
		"path":       savedPaths[0],
		"paths":      savedPaths,
		"media_type": decoded[0].MimeType,
		"size":       len(decoded[0].Data),
		"channel":    string(selectedChannel),
		"provider":   providerLabel,
		"model":      effectiveModel,
	}
	if revisedPrompt != "" {
		value["revised_prompt"] = revisedPrompt
	}
	if runRes.TaskID != "" {
		value["task_id"] = runRes.TaskID
	}
	return engine.ToolResult{Value: value, Attachments: attachments}, nil
}

// imageStudioFault maps sanitized imagestudio errors onto protocol Fault codes
// per section 3.3 without introducing new wire enum values.
func imageStudioFault(ie *imagestudio.ImageError, channel imageChannelKind) error {
	if ie == nil {
		return mediaFault(protocol.ErrorCodeUnavailable, "image generation failed")
	}
	if ie.Kind == imagestudio.ErrCancelled {
		msg := "image request was cancelled"
		if ie.TaskID != "" {
			msg = fmt.Sprintf("image request was cancelled (gateway task %s may still be running)", ie.TaskID)
		}
		return mediaFault(protocol.ErrorCodeCancelled, msg)
	}
	if ie.TaskID != "" && !ie.TaskEnded {
		return mediaFault(
			protocol.ErrorCodeOutcomeUnknown,
			fmt.Sprintf("image task %s was accepted by the gateway but its result was not retrieved (%s); do not automatically retry or resubmit the prompt", ie.TaskID, ie.Message),
		)
	}
	switch ie.Kind {
	case imagestudio.ErrUnauthorized:
		hint := "sign in to CheapRouter again in Settings -> CheapRouter"
		if channel == imageChannelCustom {
			hint = "verify your API key in Settings -> Image API"
		}
		return &protocol.Fault{
			Code:    protocol.ErrorCodeUnavailable,
			Message: fmt.Sprintf("image authentication failed: %s (%s)", ie.Message, hint),
			Details: &protocol.FaultDetails{
				Kind: "model_request",
				Data: protocol.FaultDetailsModelRequestPayload{Reason: protocol.ModelFailureUnauthorized},
			},
		}
	case imagestudio.ErrNoImagePermission, imagestudio.ErrWrongPlatform:
		return mediaFault(
			protocol.ErrorCodePermissionDenied,
			fmt.Sprintf("current account groups cannot generate images for this model (%s); switch to an image-capable group in Settings -> CheapRouter or configure Settings -> Image API", ie.Message),
		)
	case imagestudio.ErrModelUnavailable:
		if channel == imageChannelCheapRouter {
			return mediaFault(
				protocol.ErrorCodePermissionDenied,
				fmt.Sprintf("no CheapRouter group served the requested image model (%s); switch groups or omit model to use the default", ie.Message),
			)
		}
		return mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("image model is unavailable: %s", ie.Message))
	case imagestudio.ErrOwnBalance, imagestudio.ErrUpstreamBalance:
		return &protocol.Fault{
			Code:    protocol.ErrorCodeUnavailable,
			Message: fmt.Sprintf("image backend balance or quota is insufficient: %s (recharge CheapRouter or switch to Settings -> Image API)", ie.Message),
			Details: &protocol.FaultDetails{
				Kind: "model_request",
				Data: protocol.FaultDetailsModelRequestPayload{Reason: protocol.ModelFailureRateLimited},
			},
		}
	case imagestudio.ErrContentPolicy:
		return mediaFault(protocol.ErrorCodeInvalidRequest, fmt.Sprintf("image request was blocked by the content safety policy: %s (adjust the prompt or references and retry)", ie.Message))
	case imagestudio.ErrBadRequest:
		return mediaFault(protocol.ErrorCodeInvalidRequest, ie.Message)
	case imagestudio.ErrBusy:
		return &protocol.Fault{
			Code:    protocol.ErrorCodeUnavailable,
			Message: fmt.Sprintf("image backend is busy or rate limited: %s", ie.Message),
			Details: &protocol.FaultDetails{
				Kind: "model_request",
				Data: protocol.FaultDetailsModelRequestPayload{Reason: protocol.ModelFailureRateLimited},
			},
		}
	case imagestudio.ErrNetwork, imagestudio.ErrTimeout:
		return mediaFault(protocol.ErrorCodeUnavailable, fmt.Sprintf("image request failed before completion: %s", ie.Message))
	case imagestudio.ErrTaskLost, imagestudio.ErrInterrupted:
		return mediaFault(protocol.ErrorCodeOutcomeUnknown, fmt.Sprintf("image task outcome is unknown: %s; do not blindly resubmit", ie.Message))
	default:
		if ie.TaskID != "" {
			return mediaFault(protocol.ErrorCodeUnavailable, fmt.Sprintf("image task %s failed: %s", ie.TaskID, ie.Message))
		}
		return mediaFault(protocol.ErrorCodeUnavailable, ie.Message)
	}
}

func mergeImageBackendFallback(state imageBackendState, fallback mediaRoute) imageBackendState {
	if fallback.model == "" || state.mediaAssignedValid || state.convAvailable {
		return state
	}
	if fallback.assigned {
		state.mediaAssigned = true
		state.mediaAssignedValid = true
		state.mediaAssignedRoute = fallback
		state.defaultChannel = imageChannelMedia
	} else {
		state.convAvailable = true
		state.convRoute = fallback
		if state.defaultChannel == "" {
			state.defaultChannel = imageChannelMedia
		}
	}
	return state
}
