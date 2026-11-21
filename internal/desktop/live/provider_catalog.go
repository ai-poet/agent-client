package live

import (
	"context"
	"slices"
	"time"

	"github.com/ai-poet/agent-client/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/node/providers"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// providerDiscovery is what one editor's discovery learned about a channel's
// models. A save applies it only to the editor generation, saved revision and
// connection it was read for.
type providerDiscovery struct {
	generation, revision uint64
	endpoint             string
	authentication       protocol.Authentication
	api                  protocol.ConversationModelApi
	models               map[string]protocol.ConversationModel
}

func (r *Runtime) finishDiscovery(provider protocol.ConversationProvider, generation uint64, discovered []protocol.ConversationDiscoveryModel, known map[string]protocol.ConversationModel, done func([]viewmodel.ChannelModel)) {
	r.async(func() {
		ctx, cancel := context.WithTimeout(r.ctx, 10*time.Second)
		defer cancel()
		ids := make([]string, 0, len(discovered))
		for _, model := range discovered {
			ids = append(ids, model.ID)
		}
		metadata, err := r.readProviderCatalog(ctx, provider, ids)
		r.enqueue(func(c *ui.Context) {
			// An account server owns its model list. The public catalog only
			// enriches its metadata and may lag or be unavailable.
			if err != nil && provider.Authentication == protocol.AuthenticationApiKey {
				key := "provider_catalog_read_failed"
				if faultCode(err) == protocol.ErrorCodeNotConfigured {
					key = "provider_catalog_required"
				}
				r.reportKeys(c, err, key, key, key)
				done(nil)
				return
			}
			var models []viewmodel.ChannelModel
			for _, original := range discovered {
				model := known[original.ID]
				var reference *protocol.ConversationCatalogModel
				if catalog, ok := metadata[original.ID]; ok {
					reference = &catalog
					limits := protocol.ConversationDiscoveryModel{}
					if model.Context == 0 {
						limits.Context = catalog.Context
					}
					if model.Output == 0 {
						limits.Output = catalog.Output
					}
					providers.ApplyDiscoveredLimits(&model, limits)
					if model.DisplayName == nil && catalog.Name != "" {
						model.DisplayName = new(catalog.Name)
					}
				}
				if original.Output == nil && model.Context > 0 {
					model.Output = min(model.Output, model.Context)
				}
				model = providers.CompleteChannelModel(provider, model, reference)
				known[model.ID] = model
				models = append(models, channelModel(model))
			}
			if key := provider.ID.String(); r.providerModels[key].generation <= generation {
				r.providerModels[key] = providerDiscovery{generation: generation, revision: provider.Revision,
					endpoint: provider.Endpoint, authentication: provider.Authentication, api: provider.Api, models: known}
			}
			done(models)
		})
	})
}

func (r *Runtime) readProviderCatalog(ctx context.Context, provider protocol.ConversationProvider, ids []string) (map[string]protocol.ConversationCatalogModel, error) {
	result := map[string]protocol.ConversationCatalogModel{}
	remaining := map[string]bool{}
	for _, id := range ids {
		remaining[id] = true
	}
	channel := &protocol.ConversationCatalogChannel{Endpoint: provider.Endpoint, Authentication: provider.Authentication}
	var revision *uint64
	for range 32 {
		if len(remaining) == 0 {
			return result, nil
		}
		ids := make([]string, 0, len(remaining))
		for id := range remaining {
			ids = append(ids, id)
		}
		slices.Sort(ids)
		ids = ids[:min(100, len(ids))]
		out, err := r.controller.Do(ctx, protocol.Command{Kind: "read_model_catalog", Data: protocol.ConversationCatalogQuery{Channel: channel, IDs: ids, Revision: revision, Limit: 100}})
		if err != nil {
			return result, err
		}
		page, ok := detail[protocol.ConversationCatalogPage](out.Data)
		if !ok || out.Kind != "model_catalog" {
			return result, &link.FaultError{Fault: protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "Model catalog page expected"}}
		}
		for _, model := range page.Models {
			if remaining[model.ID] {
				result[model.ID] = model
				delete(remaining, model.ID)
			}
		}
		for _, id := range ids {
			if page.Next == nil || id <= *page.Next {
				delete(remaining, id)
			}
		}
		revision = &page.Revision
	}
	return result, &link.FaultError{Fault: protocol.Fault{Code: protocol.ErrorCodeUnavailable, Message: "Catalog lookup exceeded its page limit"}}
}
