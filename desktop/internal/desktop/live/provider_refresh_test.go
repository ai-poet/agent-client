package live

import (
	"slices"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/composer"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/preferences"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/shell"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/link"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func providerReadFixture(t *testing.T, transport *conversationTestTransport) *Runtime {
	t.Helper()
	controller := client.New(transport)
	t.Cleanup(func() { controller.Close() })
	if err := controller.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	r := &Runtime{controller: controller, ctx: t.Context(),
		App: &shell.App{Settings: settings.New()}, media: mediaState{reading: true},
		authorizations: &authorizationState{}}
	t.Cleanup(r.workers.Wait)
	return r
}

// Hold the presentation callbacks until the test chooses to settle the read.
// A callback can enqueue a replacement read, which the next pass handles.
func settleProviderReads(r *Runtime) {
	r.workers.Wait()
	queue := r.queue
	r.queue = nil
	for _, apply := range queue {
		apply(nil)
	}
}

func TestProviderCredentialMetadata(t *testing.T) {
	for _, name := range []string{"active", "revoked", "expired", "missing", "unavailable", "invalid", "api key", "no saved credential"} {
		t.Run(name, func(t *testing.T) {
			p := providerRouteFixture()
			p.Authentication = protocol.AuthenticationChatGptOAuth
			if name == "api key" {
				p.Authentication = protocol.AuthenticationApiKey
			}
			if name == "no saved credential" {
				p.Credential = nil
			}
			account := "Personal account"
			metadata := protocol.Credential{Provider: p.ID, Authentication: p.Authentication, AccountLabel: &account, Revoked: name == "revoked"}
			if p.Credential != nil {
				metadata.ID = p.Credential.ID
			}
			if name == "expired" {
				expired := uint64(1)
				metadata.ExpiresAtMs = &expired
			}
			snapshot := protocol.DefaultSnapshot()
			snapshot.Node, snapshot.Providers = protocol.NodeID{1}, []protocol.ConversationProvider{p}
			transport := &conversationTestTransport{snapshot: snapshot}
			reads := 0
			transport.handle = func(request protocol.Request) (link.Response, bool) {
				if request.Command.Kind != "list_credentials" {
					return link.Response{}, false
				}
				reads++
				values := []protocol.Credential{metadata}
				switch name {
				case "missing":
					values = nil
				case "unavailable":
					return link.Response{Fault: &protocol.Fault{Code: protocol.ErrorCodeUnavailable, Message: "metadata unavailable"}}, true
				case "invalid":
					return link.Response{Output: protocol.Output{Kind: "credentials", Data: "not metadata"}}, true
				}
				return link.Response{Output: protocol.Output{Kind: "credentials", Data: values}}, true
			}
			r := providerReadFixture(t, transport)
			r.projectProviderSnapshot()
			wantReads := 1
			initialStatus := ""
			if name == "api key" || name == "no saved credential" {
				wantReads = 0
			}
			if name == "no saved credential" {
				initialStatus = "provider_status_signed_out"
			}
			initial := r.App.Settings.Channels[0]
			if initial.AccountPending != (wantReads != 0) || initial.SignedIn || settings.ProviderStatus(&initial) != initialStatus {
				t.Fatalf("initial account status = %+v", initial)
			}
			settleProviderReads(r)
			final := r.App.Settings.Channels[0]
			pending := name == "unavailable" || name == "invalid"
			status, label := initialStatus, ""
			switch name {
			case "revoked", "expired", "missing":
				status = "provider_status_signed_out"
			}
			switch name {
			case "active", "revoked", "expired":
				label = account
			}
			if final.AccountPending != pending || final.SignedIn != (name == "active") || final.Account != label ||
				settings.ProviderStatus(&final) != status || reads != wantReads {
				t.Fatalf("resolved account status = %+v after %d reads", final, reads)
			}
		})
	}
}

func TestProviderCredentialMetadataDropsStaleReply(t *testing.T) {
	p := providerRouteFixture()
	p.Authentication = protocol.AuthenticationChatGptOAuth
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node, snapshot.Providers = protocol.NodeID{1}, []protocol.ConversationProvider{p}
	transport := &conversationTestTransport{snapshot: snapshot}
	reads := 0
	transport.handle = func(request protocol.Request) (link.Response, bool) {
		if request.Command.Kind != "list_credentials" {
			return link.Response{}, false
		}
		reads++
		label := "Old account"
		if reads == 2 {
			label = "New account"
		}
		return link.Response{Output: protocol.Output{Kind: "credentials", Data: []protocol.Credential{
			{ID: p.Credential.ID, Provider: p.ID, Authentication: p.Authentication, AccountLabel: &label, Revoked: reads == 2},
		}}}, true
	}
	r := providerReadFixture(t, transport)
	r.projectProviderSnapshot()
	r.workers.Wait()
	// Refresh accepts another snapshot without changing provider identity or
	// provider revision. The old read is complete but has not been applied.
	if err := r.controller.Refresh(t.Context()); err != nil {
		t.Fatal(err)
	}
	settleProviderReads(r)
	if ch := r.App.Settings.Channels[0]; !ch.AccountPending || ch.SignedIn || ch.Account != "" {
		t.Fatalf("stale credential metadata was published: %+v", ch)
	}
	settleProviderReads(r)
	ch := r.App.Settings.Channels[0]
	if reads != 2 || ch.AccountPending || ch.SignedIn || ch.Account != "New account" || settings.ProviderStatus(&ch) != "provider_status_signed_out" {
		t.Fatalf("new credential metadata was not applied: %+v after %d reads", ch, reads)
	}
}

func TestSavedProviderRefreshesOpenDraft(t *testing.T) {
	for _, removed := range []bool{false, true} {
		name := "saved channel appears without a subscription event"
		if removed {
			name = "a later removal stays authoritative"
		}
		t.Run(name, func(t *testing.T) {
			provider := providerRouteFixture()
			provider.Enabled = true
			snapshot := protocol.DefaultSnapshot()
			snapshot.Node, snapshot.Providers = protocol.NodeID{1}, []protocol.ConversationProvider{provider}
			transport := &conversationTestTransport{snapshot: snapshot}
			var saved protocol.ConversationProvider
			transport.handle = func(request protocol.Request) (link.Response, bool) {
				if request.Command.Kind != "save_provider" {
					return link.Response{}, false
				}
				input, _ := detail[protocol.CommandSaveProviderPayload](request.Command.Data)
				saved = input.Provider
				saved.Revision = 1
				// No event is sent to the existing subscription. A fresh
				// subscription reads the actual post-save state instead.
				transport.snapshot.Cursor++
				if !removed {
					transport.snapshot.Providers = append(slices.Clone(transport.snapshot.Providers), saved)
				}
				return link.Response{Output: protocol.Output{Kind: "provider", Data: saved}}, true
			}
			controller := client.New(transport)
			if err := controller.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			defer controller.Close()
			r, err := NewWithPreferences(t.Context(), controller, preferences.Open(t.TempDir()+"/preferences.json"))
			if err != nil {
				t.Fatal(err)
			}
			defer r.Close()
			id := r.App.FocusedConversation()
			cv := r.App.Data.Find(id)
			if cv == nil {
				t.Fatal("new conversation draft is unavailable")
			}
			comp := r.App.Composer(id)
			var offered *composer.Context
			tester := ui.NewTester(func(c *ui.Context) {
				r.drain(c)
				offered, _ = r.conversation(c, cv, comp)
				r.finishFrame()
				ui.ToastViewportBase(c, func(ui.Element, []ui.Toast) {})
			}, 640, 400)
			settle := func() {
				for range 6 {
					r.workers.Wait()
					tester.Frame()
				}
			}
			settle()
			comp.Draft = "Keep the existing conversation draft"
			selected := comp.Model
			newID := protocol.ProviderID{9}.String()
			done := false
			r.saveProvider(settings.ProviderDraft{Channel: viewmodel.Channel{
				ID: newID, Name: "New channel", API: "responses", Endpoint: "https://models.example.test/v1", Auth: viewmodel.AuthKey, Enabled: true,
				Models: []viewmodel.ChannelModel{{ID: "new-model", Context: 128000, Output: 16384}},
			}}, func(ok bool) { done = ok })
			settle()
			if !done {
				t.Fatal("confirmed channel save was not reported")
			}
			found := slices.ContainsFunc(offered.Models, func(model composer.Model) bool { return model.ID == modelKey(saved.ID, "new-model") })
			if found == removed {
				t.Fatalf("open draft model choices do not match the fresh Node snapshot: %+v", offered.Models)
			}
			if comp.Model != selected || comp.Draft != "Keep the existing conversation draft" {
				t.Fatal("refreshing channel choices replaced the draft or its model selection")
			}
			transport.mu.Lock()
			defer transport.mu.Unlock()
			if count := len(slices.DeleteFunc(slices.Clone(transport.requests), func(request protocol.Request) bool { return request.Command.Kind != "save_provider" })); count != 1 {
				t.Fatalf("refresh repeated the channel save: %d requests", count)
			}
		})
	}
}
