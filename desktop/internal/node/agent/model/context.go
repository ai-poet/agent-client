// Context discovery policy derived from Goose 14741e9a providers/openai.rs
// (Apache-2.0). See LICENSE.goose.
package model

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/node/providers"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/netproxy"
)

// defaultContext is the window assumed for a model nothing describes.
const defaultContext = 256000

const (
	contextProbeTimeout = 5 * time.Second
	contextKnown        = 15 * time.Minute
	contextFailed       = 60 * time.Second
	contextCapacity     = 128
)

type contextEntry struct {
	limits  providers.ModelLimits
	expires time.Time
}

type modelLimitKey struct {
	provider protocol.ProviderID
	revision uint64
	api      protocol.ConversationModelApi
	endpoint string
	model    string
}

var contextCache = struct {
	sync.Mutex
	entries map[modelLimitKey]contextEntry
}{entries: map[modelLimitKey]contextEntry{}}

// ContextLimit returns the effective window for budgeting. Call ResolvedModel
// when the distinction between a declared window and an estimate matters.
func ContextLimit(binding Binding) uint32 {
	selected, _ := ResolvedModel(binding)
	return selected.Context
}

// ResolvedModel is shared by execution and statistics. It never probes:
// provider declarations > models.dev > cached probe > internal estimate.
func ResolvedModel(binding Binding) (selected protocol.ConversationModel, estimated bool) {
	selected = protocol.DefaultConversationModel()
	selected.ID = binding.Config.Model
	if listed := metadata(binding); listed != nil {
		selected = *listed
	}
	if selected.Context == 0 || selected.Output == 0 {
		registered := providers.CatalogModelLimits(binding.Provider.Api, selected.ID)
		probed, _ := reportedContext(contextKey(binding))
		if selected.Context == 0 {
			selected.Context = registered.Context
			if selected.Context == 0 {
				selected.Context = probed.Context
			}
		}
		if selected.Output == 0 {
			selected.Output = registered.Output
			if selected.Output == 0 {
				selected.Output = probed.Output
			}
		}
	}
	estimated = selected.Context == 0
	if estimated {
		selected.Context = estimatedContext(selected.ID)
	}
	selected.Output = min(selected.Output, selected.Context, providers.MaxModelOutput)
	return selected, estimated
}

func contextKey(binding Binding) modelLimitKey {
	p := binding.Provider
	return modelLimitKey{p.ID, p.Revision, p.Api, p.Endpoint, binding.Config.Model}
}

func reportedContext(key modelLimitKey) (providers.ModelLimits, bool) {
	contextCache.Lock()
	defer contextCache.Unlock()
	entry, ok := contextCache.entries[key]
	if ok && !time.Now().Before(entry.expires) {
		delete(contextCache.entries, key)
		return providers.ModelLimits{}, false
	}
	return entry.limits, ok
}

func rememberContext(key modelLimitKey, limits providers.ModelLimits, life time.Duration) {
	contextCache.Lock()
	defer contextCache.Unlock()
	now := time.Now()
	if len(contextCache.entries) >= contextCapacity {
		var oldest modelLimitKey
		oldestSet := false
		for name, entry := range contextCache.entries {
			if !now.Before(entry.expires) {
				delete(contextCache.entries, name)
			} else if !oldestSet || entry.expires.Before(contextCache.entries[oldest].expires) {
				oldest, oldestSet = name, true
			}
		}
		if len(contextCache.entries) >= contextCapacity {
			delete(contextCache.entries, oldest)
		}
	}
	contextCache.entries[key] = contextEntry{limits: limits, expires: now.Add(life)}
}

// discoverContext asks an OpenAI-compatible server for the window it allocated
// to a model whose window is not declared or registered. The answer, or its
// absence, is cached; a cancelled turn keeps nothing.
func discoverContext(ctx context.Context, binding Binding, config transportConfig) {
	if listed := metadata(binding); listed != nil && listed.Context > 0 {
		return
	}
	if known := providers.CatalogModelLimits(binding.Provider.Api, binding.Config.Model); known.Context > 0 {
		return
	}
	key := contextKey(binding)
	switch binding.Provider.Api {
	case protocol.ConversationModelApiResponses, protocol.ConversationModelApiChatCompletions, protocol.ConversationModelApiDeepSeek, protocol.ConversationModelApiOpenCodeGo, protocol.ConversationModelApiOpenCodeZen:
	case protocol.ConversationModelApiAzureAi:
		// A Foundry deployment is sized by the model it serves.
		if config.Model != binding.Config.Model {
			rememberContext(key, providers.CatalogModelLimits(binding.Provider.Api, config.Model), contextKnown)
		}
		return
	default:
		return
	}
	if _, ok := reportedContext(key); ok {
		return
	}
	probe, cancel := context.WithTimeout(ctx, contextProbeTimeout)
	limits, err := allocatedContext(probe, config, binding.Config.Model)
	cancel()
	if ctx.Err() != nil {
		return
	}
	life := contextKnown
	if err != nil || limits == (providers.ModelLimits{}) {
		life = contextFailed
	}
	rememberContext(key, limits, life)
}

// allocatedContext reads the model's limits from the server's model list.
func allocatedContext(ctx context.Context, config transportConfig, name string) (providers.ModelLimits, error) {
	var unknown providers.ModelLimits
	target, err := url.Parse(config.Endpoint + "/models")
	if err != nil {
		return unknown, err
	}
	query := target.Query()
	for key, value := range config.Query {
		query.Set(key, value)
	}
	target.RawQuery = query.Encode()
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, target.String(), nil)
	if err != nil {
		return unknown, err
	}
	for key, value := range config.Headers {
		request.Header.Set(key, value)
	}
	client := &http.Client{Transport: netproxy.Transport(), CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Do(request)
	if err != nil {
		return unknown, &Failure{Kind: Network, Message: "model context discovery failed"}
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusNotFound {
		return unknown, nil
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return unknown, &Failure{Kind: Network, Message: "model context discovery failed"}
	}
	type entry struct {
		ID            string `json:"id"`
		Context       uint32 `json:"context_window"`
		ContextTokens uint32 `json:"context_window_tokens"`
		ContextLength uint32 `json:"context_length"`
		Output        uint32 `json:"max_output_tokens"`
		Completion    uint32 `json:"max_completion_tokens"`
		Meta          struct {
			Context uint32 `json:"n_ctx"`
		} `json:"meta"`
	}
	var body struct {
		Data []entry `json:"data"`
	}
	if err = json.NewDecoder(io.LimitReader(response.Body, 4<<20)).Decode(&body); err != nil {
		return unknown, &Failure{Kind: Network, Message: "model context response is invalid"}
	}
	limits := func(found entry) providers.ModelLimits {
		var window uint32
		for _, candidate := range []uint32{found.Meta.Context, found.Context, found.ContextTokens, found.ContextLength} {
			if candidate >= providers.MinCatalogContext {
				window = candidate
				break
			}
		}
		output := found.Output
		if output == 0 {
			output = found.Completion
		}
		return providers.ModelLimits{Context: window, Output: min(output, providers.MaxModelOutput)}
	}
	for _, found := range body.Data {
		if found.ID == name {
			return limits(found), nil
		}
	}
	// Retain the single-loaded-model alias for servers exposing n_ctx.
	if len(body.Data) == 1 {
		if found := body.Data[0]; found.Meta.Context >= providers.MinCatalogContext {
			return limits(found), nil
		}
	}
	return unknown, nil
}

func estimatedContext(name string) uint32 {
	name = strings.ToLower(name)
	if strings.Contains(name, "claude-2") || strings.Contains(name, "claude-instant") {
		return 100000
	}
	for _, family := range []string{"claude", "opus", "sonnet", "haiku", "fable"} {
		if strings.Contains(name, family) {
			return 200000
		}
	}
	return defaultContext
}
