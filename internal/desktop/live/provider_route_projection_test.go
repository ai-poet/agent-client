package live

import (
	"bytes"
	"encoding/json"
	"reflect"
	"slices"
	"testing"

	"github.com/ai-poet/agent-client/internal/client"
	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/ai-poet/agent-client/internal/desktop/shell"
	"github.com/ai-poet/agent-client/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/internal/node/providers"
	"github.com/ai-poet/agent-client/internal/protocol"
)

func TestProviderRouteProjection(t *testing.T) {
	for _, test := range []struct {
		name   string
		api    protocol.ConversationModelApi
		route  *protocol.ConversationPresetsRouteKey
		method string
	}{
		{name: "Responses without a route", api: protocol.ConversationModelApiResponses, method: "builtin:compatible-responses/0"},
		{name: "Chat Completions without a route", api: protocol.ConversationModelApiChatCompletions, method: "builtin:compatible-responses/1"},
		{name: "explicit Chat Completions route", api: protocol.ConversationModelApiChatCompletions,
			route: &protocol.ConversationPresetsRouteKey{Protocol: protocol.ConversationPresetsProtocolOpenAiChatCompletions}, method: "builtin:compatible-responses/1"},
	} {
		t.Run(test.name, func(t *testing.T) {
			provider := providerRouteFixture()
			provider.Api, provider.Preset.Route = test.api, test.route
			listing := providers.Listing(protocol.ConversationPresetsCatalog{}, false)
			channel, _ := projectProviderRouteFixture(t, provider, listing, viewmodel.AuthKey)
			if channel.Method != test.method {
				t.Fatalf("projected method = %q, want %q", channel.Method, test.method)
			}
		})
	}
}

func TestLegacyCompatibleProviderRouteProjection(t *testing.T) {
	for _, test := range []struct {
		name   string
		route  *protocol.ConversationPresetsRouteKey
		target string
	}{
		{name: "without a route", target: "offered"},
		{name: "explicit Chat Completions route", target: "offered", route: &protocol.ConversationPresetsRouteKey{Protocol: protocol.ConversationPresetsProtocolOpenAiChatCompletions}},
		{name: "replacement is not offered", target: "unoffered"},
		{name: "replacement is unavailable", target: "missing"},
	} {
		t.Run(test.name, func(t *testing.T) {
			provider := providerRouteFixture()
			provider.Preset.ID, provider.Preset.Route = "compatible-chat", test.route
			listing := providers.Listing(protocol.ConversationPresetsCatalog{}, false)
			if test.target == "missing" {
				listing.Entries = slices.DeleteFunc(listing.Entries, func(entry protocol.ConversationPresetsEntry) bool {
					return entry.Preset.ID == "compatible-responses"
				})
			} else if test.target == "unoffered" {
				for i := range listing.Entries {
					if listing.Entries[i].Preset.ID == "compatible-responses" {
						listing.Entries[i].Preset.Offered = false
					}
				}
			}
			channel, brands := projectProviderRouteFixture(t, provider, listing, viewmodel.AuthKey)
			if test.target != "offered" {
				if channel.Method != "builtin:compatible-chat/0" {
					t.Fatalf("legacy method = %q without an offered replacement", channel.Method)
				}
				return
			}
			if channel.Method != "builtin:compatible-responses/1" {
				t.Fatalf("legacy method = %q, want the offered Chat Completions route", channel.Method)
			}
			for _, brand := range brands {
				for _, method := range brand.Methods {
					for _, route := range method.Routes {
						if route.ID == channel.Method {
							if route.API != channel.API || len(method.Routes) != 2 {
								t.Fatalf("legacy provider cannot switch its protocol through %+v", method)
							}
							return
						}
					}
				}
			}
			t.Fatal("the projected legacy route is not offered in the editor")
		})
	}
}

func providerRouteFixture() protocol.ConversationProvider {
	provider := protocol.DefaultConversationProvider()
	provider.ID, provider.Revision, provider.Name = protocol.ProviderID{3}, 8, "Configured gateway"
	provider.Api, provider.Authentication = protocol.ConversationModelApiChatCompletions, protocol.AuthenticationApiKey
	provider.Endpoint = "https://gateway.example.test/v1"
	provider.Preset = &protocol.ConversationPresetsPresetRef{Source: protocol.ConversationPresetsSource{Kind: "builtin"}, ID: "compatible-responses"}
	provider.Credential = &protocol.CredentialRef{Node: protocol.NodeID{1}, ID: protocol.CredentialID{2}}
	first := protocol.DefaultConversationModel()
	first.ID, first.Context, first.Output = "first", 128000, 16384
	second := protocol.DefaultConversationModel()
	name, compaction := "Gateway reasoner", uint32(4096)
	second.ID, second.DisplayName, second.Context, second.Output, second.CompactionTokens = "reasoner", &name, 240000, 32768, &compaction
	second.Vision, second.Tools, second.Reasoning, second.CustomCapabilities, second.CustomEfforts = true, true, true, true, true
	second.Efforts, second.DefaultEffort = []protocol.Effort{{Kind: "low"}, {Kind: "high"}}, protocol.Effort{Kind: "high"}
	second.Capabilities = &protocol.ConversationDiscoveryCapabilities{Options: map[string]json.RawMessage{"adapter": json.RawMessage(`"preserved"`)}}
	provider.Models, provider.DefaultModel = []protocol.ConversationModel{first, second}, second.ID
	return provider
}

func projectProviderRouteFixture(t *testing.T, provider protocol.ConversationProvider, listing protocol.ConversationPresetsListing, wantAuth viewmodel.Auth) (viewmodel.Channel, []viewmodel.Brand) {
	t.Helper()
	before, err := json.Marshal(provider)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node, snapshot.Providers = protocol.NodeID{1}, []protocol.ConversationProvider{provider}
	controller := client.New(&conversationTestTransport{snapshot: snapshot})
	defer controller.Close()
	r := &Runtime{controller: controller, snapshotRead: true, snapshotCopy: snapshot,
		App: &shell.App{Settings: settings.New()}, media: mediaState{reading: true}}
	r.applyPresets(listing)
	r.projectProviderSnapshot()
	if len(r.App.Settings.Channels) != 1 {
		t.Fatalf("projected channels = %+v", r.App.Settings.Channels)
	}
	channel := r.App.Settings.Channels[0]
	if channel.ID != provider.ID.String() || channel.Revision != provider.Revision || channel.Name != provider.Name || channel.API != string(provider.Api) ||
		channel.Endpoint != provider.Endpoint || channel.Enabled != provider.Enabled || channel.Auth != wantAuth || channel.KeySet != (provider.Credential != nil) || channel.Default != 1 {
		t.Fatalf("provider configuration changed in the projection: %+v", channel)
	}
	wantModels := []viewmodel.ChannelModel{channelModel(provider.Models[0]), channelModel(provider.Models[1])}
	if !reflect.DeepEqual(channel.Models, wantModels) {
		t.Fatalf("projected model metadata changed: %+v", channel.Models)
	}
	after, err := json.Marshal(r.snapshotCopy.Providers[0])
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) {
		t.Fatal("route projection changed the saved provider, credential or model metadata")
	}
	return channel, r.App.Settings.Brands
}

func TestPackagePresetCredentialStatus(t *testing.T) {
	for _, test := range []struct {
		name, kind, packageName, status string
		auth                            viewmodel.Auth
		match                           bool
	}{
		{name: "no credential required", kind: "none", packageName: "fixture", auth: viewmodel.AuthNone, match: true},
		{name: "API key required", kind: "api_key", packageName: "fixture", auth: viewmodel.AuthKey, status: "provider_status_missing_key", match: true},
		{name: "plugin sign-in", kind: "plugin", packageName: "fixture", auth: viewmodel.AuthOAuth, status: "provider_status_signed_out", match: true},
		{name: "catalog unavailable", auth: viewmodel.AuthUnresolved},
		{name: "same preset ID from another package", kind: "api_key", packageName: "other", auth: viewmodel.AuthUnresolved},
	} {
		t.Run(test.name, func(t *testing.T) {
			provider := providerRouteFixture()
			pkg := protocol.ConversationPresetsSourcePackagePayload{Name: "fixture"}
			provider.Preset.Source = protocol.ConversationPresetsSource{Kind: "package", Data: &pkg}
			provider.Api, provider.Credential = protocol.ConversationModelApiPlugin, nil
			listing := protocol.DefaultConversationPresetsListing()
			if test.kind != "" {
				entry := protocol.DefaultConversationPresetsEntry()
				entry.Source = protocol.ConversationPresetsSource{Kind: "package", Data: protocol.ConversationPresetsSourcePackagePayload{Name: test.packageName}}
				entry.Package = &protocol.PluginReference{Name: test.packageName}
				entry.Preset.ID, entry.Preset.Auth.Kind = provider.Preset.ID, test.kind
				entry.Preset.Routes = []protocol.ConversationPresetsRoute{{Protocol: protocol.ConversationPresetsProtocolPlugin}}
				listing.Entries = []protocol.ConversationPresetsEntry{entry}
				if test.kind == "plugin" {
					provider.Authentication = protocol.AuthenticationPlugin
					login := protocol.PluginModelAuthPresentation{Kind: "pkce", Data: protocol.PluginModelAuthPresentationPkcePayload{Redirect: protocol.PluginModelAuthRedirectPaste}}
					listing.Entries[0].Login = &login
				}
			}
			channel, _ := projectProviderRouteFixture(t, provider, listing, test.auth)
			method := ""
			if test.match {
				method = "package:fixture:" + provider.Preset.ID + "/0"
			}
			if channel.Method != method || settings.ProviderStatus(&channel) != test.status {
				t.Fatalf("package channel method = %q, status = %q", channel.Method, settings.ProviderStatus(&channel))
			}
			r := &Runtime{App: &shell.App{Settings: settings.New()}}
			r.applyPresets(listing)
			entry, matched := r.loginEntry(provider)
			if matched != test.match || test.kind == "plugin" && entry.Login == nil {
				t.Fatalf("login preset match = %t, entry = %+v", matched, entry)
			}
		})
	}
}
