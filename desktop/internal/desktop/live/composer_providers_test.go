package live

import (
	"slices"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/composer"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func TestComposerProvidersMergeFollowedChannelsByIdentity(t *testing.T) {
	t.Parallel()
	shared, controllerOnly, executorOnly, private := protocol.ProviderID{1}, protocol.ProviderID{2}, protocol.ProviderID{3}, protocol.ProviderID{4}
	provider := func(id protocol.ProviderID, name string) protocol.ConversationProvider {
		return protocol.ConversationProvider{ID: id, Name: name, Enabled: true,
			Models: []protocol.ConversationModel{{ID: "gpt-6-sol"}, {ID: "gpt-6.1-sol"}}}
	}
	controllerSnapshot, executorSnapshot := protocol.DefaultSnapshot(), protocol.DefaultSnapshot()
	controllerSnapshot.Node, executorSnapshot.Node = protocol.NodeID{1}, protocol.NodeID{2}
	controllerSnapshot.Providers = []protocol.ConversationProvider{provider(shared, "Controller copy"), provider(controllerOnly, "Controller only")}
	foreign := provider(protocol.ProviderID{5}, "Foreign")
	foreign.Credential = &protocol.CredentialRef{Node: protocol.NodeID{3}, ID: protocol.CredentialID{1}}
	followed := provider(shared, "Execution copy")
	followed.Source = &protocol.ConfigurationOrigin{Node: controllerSnapshot.Node, Revision: 1}
	followed.Credential = &protocol.CredentialRef{Node: controllerSnapshot.Node, ID: protocol.CredentialID{2}}
	controllerSnapshot.Providers[1].Credential = followed.Credential // Local API key, including CheapAuth.
	executorSnapshot.Providers = []protocol.ConversationProvider{followed, provider(executorOnly, "Execution only"), foreign}
	captured := protocol.DefaultSessionProfile()
	captured.Source = controllerSnapshot.Node
	captured.Provider = provider(private, "Captured")
	captured.Provider.Credential = followed.Credential
	session := protocol.SessionID{1}
	executorSnapshot.Sessions = []protocol.Session{{ID: session, Profile: &captured}}
	controller := client.New(&conversationTestTransport{snapshot: controllerSnapshot})
	executor := client.New(&conversationTestTransport{snapshot: executorSnapshot})
	defer controller.Close()
	defer executor.Close()
	for _, source := range []*client.Client{controller, executor} {
		if err := source.Start(t.Context()); err != nil {
			t.Fatal(err)
		}
	}
	root := &Runtime{controller: controller}
	r := &Runtime{controller: executor, parent: root, admittedSessions: map[string]protocol.Session{"pending": {Profile: &captured}}}
	providers := r.composerProviders()
	var ids []protocol.ProviderID
	modelIDs := map[string]bool{}
	for _, p := range providers {
		ids = append(ids, p.ID)
		for _, model := range p.Models {
			key := modelKey(p.ID, model.ID)
			if modelIDs[key] {
				t.Fatalf("the composer offers duplicate model identity %q", key)
			}
			modelIDs[key] = true
		}
	}
	if !slices.Equal(ids, []protocol.ProviderID{shared, executorOnly, controllerOnly, private}) || providers[0].Name != "Execution copy" {
		t.Fatalf("merged providers changed execution resolution or order: %+v", providers)
	}
	if r.forwards(shared) || !r.forwards(controllerOnly) {
		t.Fatal("offered provider precedence differs from the creation route")
	}
	if len(r.snapshot().Providers) != 3 || len(root.snapshot().Providers) != 2 {
		t.Fatal("merging channels modified a source snapshot")
	}
	comp := composer.New("")
	for _, tc := range []struct {
		id       string
		provider protocol.ProviderID
		allowed  bool
	}{
		{"draft/1", foreign.ID, false},
		{"draft/1", shared, true},
		{"draft/1", controllerOnly, true},
		{"draft/1", private, false},
		{session.String(), private, true},
		{session.String(), controllerOnly, false},
	} {
		config, _, err := r.composerConfig(tc.id, comp, composer.Message{Model: modelKey(tc.provider, "gpt-6-sol")})
		if (err == nil) != tc.allowed || tc.allowed && (config.Credential == nil || *config.Credential != *followed.Credential) {
			t.Errorf("%s on %s: config=%+v, err=%v", tc.provider, tc.id, config, err)
		}
	}

}
