package live

import (
	"fmt"
	"testing"

	"github.com/ai-poet/agent-client/internal/desktop/shell"
	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/protocol"
)

func TestProviderPresetCache(t *testing.T) {
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node = protocol.NodeID{1}
	transport := &conversationTestTransport{snapshot: snapshot}
	reads := 0
	transport.handle = func(request protocol.Request) (link.Response, bool) {
		if request.Command.Kind != "list_channel_presets" {
			return link.Response{}, false
		}
		reads++
		switch reads {
		case 1:
			return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeUnavailable, Message: "catalog unavailable"}}, true
		case 2:
			return link.Response{Output: protocol.Output{Kind: "channel_presets", Data: "not a catalog"}}, true
		}
		entry := protocol.DefaultConversationPresetsEntry()
		entry.Source.Kind, entry.Preset.ID, entry.Preset.Name = "builtin", "fixture", fmt.Sprintf("Catalog %d", reads)
		entry.Preset.Offered = true
		entry.Preset.Routes = []protocol.ConversationPresetsRoute{{Protocol: protocol.ConversationPresetsProtocolOpenAiChatCompletions}}
		listing := protocol.DefaultConversationPresetsListing()
		listing.Entries = []protocol.ConversationPresetsEntry{entry}
		return link.Response{Output: protocol.Output{Kind: "channel_presets", Data: listing}}, true
	}
	r := providerReadFixture(t, transport)
	r.App.Page = shell.ConversationPage // No action toast for a background failure.
	check := func(count int, loaded bool, name string) {
		t.Helper()
		if reads != count || r.providerPresetsLoaded != loaded || r.providerBrands["fixture"] != name {
			t.Fatalf("catalog reads=%d loaded=%t brands=%v; want %d, %t, %q", reads, r.providerPresetsLoaded, r.providerBrands, count, loaded, name)
		}
	}
	for attempt := 1; attempt <= 3; attempt++ {
		r.projectProviderSnapshot()
		settleProviderReads(r)
		if attempt == 3 {
			check(attempt, true, "Catalog 3")
		} else {
			check(attempt, false, "")
		}
	}
	install := func(packages ...protocol.PluginSummary) {
		t.Helper()
		transport.mu.Lock()
		transport.snapshot.Cursor++
		transport.snapshot.Plugins = packages
		transport.mu.Unlock()
		if err := r.controller.Refresh(t.Context()); err != nil {
			t.Fatal(err)
		}
	}

	pkg := protocol.DefaultPluginSummary()
	pkg.Name, pkg.Enabled, pkg.Digest = "fixture", true, "version-a"
	install(pkg)
	r.projectProviderSnapshot()
	r.workers.Wait()
	// The old request has answered, but its UI callback hasn't run yet.
	pkg.Digest = "version-b"
	install(pkg)
	settleProviderReads(r)
	r.workers.Wait()
	check(5, false, "Catalog 3")
	settleProviderReads(r)
	check(5, true, "Catalog 5")

	pkg.Revision++
	install(pkg)
	r.projectProviderSnapshot()
	settleProviderReads(r)
	check(5, true, "Catalog 5")

	for i, update := range []func(){
		func() { pkg.SettingsRevision++ },
		func() { pkg.Enabled = false },
		func() {
			pkg.Compatibility = protocol.PluginCompatibility{Kind: "requires_newer",
				Data: protocol.PluginCompatibilityRequiresNewerPayload{Required: "10.0.0"}}
		},
	} {
		update()
		install(pkg)
		r.projectProviderSnapshot()
		settleProviderReads(r)
		check(6+i, true, fmt.Sprintf("Catalog %d", 6+i))
	}
	install()
	r.projectProviderSnapshot()
	settleProviderReads(r)
	check(9, true, "Catalog 9")
	if len(r.providerPresetPackages) != 0 {
		t.Fatalf("removed package retained in catalog key: %v", r.providerPresetPackages)
	}

	// Explicit refreshes still request built-in presets with no package change
	// and are not lost if a catalog reply is already queued.
	r.readProviders()
	r.workers.Wait()
	r.readProviders()
	settleProviderReads(r)
	r.workers.Wait()
	check(11, false, "Catalog 9")
	settleProviderReads(r)
	check(11, true, "Catalog 11")
}
