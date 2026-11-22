package live

import (
	"encoding/json"
	"slices"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/client"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/composer"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preferences"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

// A conversation selected on the unbound CheapRouter route follows its model
// to the lane that replaced the route, when it is shown and before a manual
// compaction. Nothing else moves a conversation to another provider.
func TestConversationFollowsTheLaneOfItsModel(t *testing.T) {
	node, project, worktree, session := protocol.NodeID{1}, protocol.ProjectID{2}, protocol.WorktreeID{3}, protocol.SessionID{5}
	unbound, claude, codex := providers.CheapRouterProviderID, providers.CheapRouterLaneID(core.LaneClaude), providers.CheapRouterLaneID(core.LaneCodex)
	const model = "claude-sonnet-4-6"
	credential := func(id protocol.ProviderID) *protocol.CredentialRef {
		return &protocol.CredentialRef{Node: node, ID: protocol.CredentialID(id)}
	}
	offer := func(id protocol.ProviderID, enabled bool, efforts ...string) protocol.ConversationProvider {
		offered := protocol.ConversationModel{ID: model, Reasoning: true, DefaultEffort: protocol.Effort{Kind: "high"}}
		for _, effort := range efforts {
			offered.Efforts = append(offered.Efforts, protocol.Effort{Kind: effort})
		}
		return protocol.ConversationProvider{ID: id, Revision: 3, Enabled: enabled, Name: "CheapRouter", Api: protocol.ConversationModelApiChatCompletions,
			Authentication: protocol.AuthenticationApiKey, Models: []protocol.ConversationModel{offered}, Credential: credential(id)}
	}
	for _, row := range []struct {
		name      string
		selected  protocol.ProviderID
		effort    string
		providers []protocol.ConversationProvider
		// reconciling holds the shown conversation back; moved is the session
		// as route reconciliation saved it meanwhile.
		reconciling, moved bool
		follows            protocol.ProviderID
		saved              string
	}{
		{name: "a shown conversation saves its lane", selected: unbound, effort: "max", follows: claude, saved: "max",
			providers: []protocol.ConversationProvider{offer(unbound, false, "low", "high", "max"), offer(claude, true, "low", "high", "max"), offer(codex, true, "low", "high", "max")}},
		{name: "an effort the lane cannot take becomes its default", selected: unbound, effort: "max", follows: claude, saved: "high",
			providers: []protocol.ConversationProvider{offer(unbound, false, "low", "high", "max"), offer(claude, true, "low", "high")}},
		{name: "compaction follows during reconciliation", selected: unbound, effort: "high", reconciling: true, follows: claude, saved: "high",
			providers: []protocol.ConversationProvider{offer(unbound, false, "low", "high"), offer(claude, true, "low", "high")}},
		{name: "a session reconciliation moved is adopted", selected: unbound, effort: "high", moved: true, follows: claude,
			providers: []protocol.ConversationProvider{offer(unbound, true, "low", "high"), offer(claude, true, "low", "high")}},
		{name: "another lane never stands in", selected: unbound, effort: "high",
			providers: []protocol.ConversationProvider{offer(unbound, false, "low", "high"), offer(claude, false, "low", "high"), offer(codex, true, "low", "high")}},
		{name: "an unbound lane keeps its conversation", selected: claude, effort: "high",
			providers: []protocol.ConversationProvider{offer(unbound, false, "low", "high"), offer(claude, false, "low", "high"), offer(codex, true, "low", "high")}},
		{name: "removed unbound route stays unavailable", selected: unbound, effort: "high",
			providers: []protocol.ConversationProvider{offer(claude, true, "low", "high")}},
		{name: "the unbound route still serves", selected: unbound, effort: "high",
			providers: []protocol.ConversationProvider{offer(unbound, true, "low", "high"), offer(claude, true, "low", "high")}},
	} {
		t.Run(row.name, func(t *testing.T) {
			config := protocol.DefaultSessionConfig()
			config.Provider, config.Model, config.Credential = row.selected, model, credential(row.selected)
			config.Effort, config.Mode, config.Permission = protocol.Effort{Kind: row.effort}, protocol.WorkModePlan, protocol.PermissionProject
			snapshot := protocol.DefaultSnapshot()
			snapshot.Node, snapshot.Providers = node, row.providers
			snapshot.Projects = []protocol.Project{{ID: project, Name: "Workspace", Path: "/fixture/project"}}
			snapshot.Worktrees = []protocol.Worktree{{ID: worktree, Project: &project, Path: "/fixture/worktree", Main: true}}
			snapshot.Sessions = []protocol.Session{{ID: session, Revision: 6, Worktree: worktree, Project: &project, Config: config, Roles: protocol.DefaultRoleSnapshot()}}
			source := &selectionTestTransport{
				conversationTestTransport: conversationTestTransport{snapshot: snapshot},
				node:                      &conversationTestSubscription{updates: make(chan protocol.Update, 2), closed: make(chan struct{})},
			}
			source.node.updates <- protocol.Update{Kind: "snapshot", Data: snapshot}
			const unavailableMessage = "Select an available model before compacting this conversation."
			var compactProvider protocol.ProviderID
			// The Node admits a compaction only on an enabled provider.
			source.handle = func(request protocol.Request) (link.Response, bool) {
				input, compaction := request.Command.Data.(protocol.CommandCompactContextPayload)
				if !compaction {
					return link.Response{}, false
				}
				selectedID := source.sessions[input.Session].Config.Provider
				for _, provider := range source.snapshot.Providers {
					if provider.ID != selectedID {
						continue
					}
					if provider.Enabled {
						compactProvider = provider.ID
						return link.Response{Output: protocol.Output{Kind: "queued_turn", Data: protocol.DefaultQueuedTurn()}}, true
					}
					return link.Response{Fault: protocol.NewConfigurationFault(protocol.ConfigurationProviderDisabled, unavailableMessage)}, true
				}
				return link.Response{Fault: protocol.NewConfigurationFault(protocol.ConfigurationProviderRemoved, unavailableMessage)}, true
			}
			controller := client.New(source)
			if err := controller.Start(t.Context()); err != nil {
				t.Fatal(err)
			}
			defer controller.Close()
			r, err := NewWithPreferences(t.Context(), controller, preferences.Open(t.TempDir()+"/preferences.json"))
			if err != nil {
				t.Fatal(err)
			}
			defer r.Close()
			if row.reconciling {
				r.cheapAuth.state.Stage = cheapauthui.SyncingGateway
			}
			id := session.String()
			cv := r.App.Data.Find(id)
			if cv == nil {
				cv = &viewmodel.Conversation{ID: id}
			}
			comp := r.App.Composer(id)
			var shown *composer.Context
			var actions *conversationActions
			tester := ui.NewTester(func(c *ui.Context) {
				theme.Apply(c, "", theme.Light)
				r.drain(c)
				shown, _ = r.conversation(c, cv, comp)
				actions = &conversationActions{r: r, c: c, cv: cv, comp: comp}
				kit.ToastHost(c)
				r.finishFrame()
			}, 640, 400)
			tester.SetLocale("en")
			settle := func() {
				for range 4 {
					r.workers.Wait()
					tester.Frame()
				}
			}
			settle()
			if row.moved {
				// Route reconciliation disabled the unbound route and saved
				// the session on its lane while the conversation was shown.
				source.mu.Lock()
				snapshot.Cursor++
				snapshot.Providers = []protocol.ConversationProvider{offer(unbound, false, "low", "high"), offer(claude, true, "low", "high")}
				snapshot.Sessions[0].Revision, snapshot.Sessions[0].Config.Provider, snapshot.Sessions[0].Config.Credential = 7, claude, credential(claude)
				source.snapshot, source.sessions = snapshot, map[protocol.SessionID]protocol.Session{session: snapshot.Sessions[0]}
				source.mu.Unlock()
				source.node.updates <- protocol.Update{Kind: "snapshot", Data: snapshot}
				for end := time.Now().Add(10 * time.Second); controller.Snapshot().Cursor != snapshot.Cursor; time.Sleep(time.Millisecond) {
					if time.Now().After(end) {
						t.Fatal("the Client did not apply the reconciled snapshot")
					}
				}
				settle()
			}
			want := config
			if row.follows != (protocol.ProviderID{}) {
				want.Provider, want.Credential, want.Effort = row.follows, credential(row.follows), protocol.Effort{Kind: row.saved}
			}
			// requests are the configuration saves and compactions sent so far.
			requests := func() (saves []protocol.CommandSetSessionConfigPayload, compactions []uint64) {
				source.mu.Lock()
				defer source.mu.Unlock()
				for _, request := range source.requests {
					switch input := request.Command.Data.(type) {
					case protocol.CommandSetSessionConfigPayload:
						saves = append(saves, input)
					case protocol.CommandCompactContextPayload:
						compactions = append(compactions, input.ExpectedRevision)
					}
				}
				return saves, compactions
			}
			saved := func(saves []protocol.CommandSetSessionConfigPayload) {
				t.Helper()
				if len(saves) != 1 {
					t.Fatalf("saved %d configurations, want the lane once", len(saves))
				}
				got, _ := json.Marshal(saves[0].Config)
				expected, _ := json.Marshal(want)
				if saves[0].Session != session || saves[0].ExpectedRevision != 6 || string(got) != string(expected) {
					t.Fatalf("saved at revision %d:\n got %s\nwant %s", saves[0].ExpectedRevision, got, expected)
				}
			}
			followed := row.follows != (protocol.ProviderID{}) && !row.reconciling
			saves, compactions := requests()
			switch {
			case len(compactions) != 0:
				t.Fatalf("showing the conversation compacted it: %v", compactions)
			case followed && !row.moved:
				saved(saves)
			case len(saves) != 0:
				t.Fatalf("showing the conversation saved %+v", saves)
			}
			selection := row.selected
			if followed {
				selection = row.follows
			}
			selectedEnabled := slices.ContainsFunc(row.providers, func(p protocol.ConversationProvider) bool { return p.ID == row.selected && p.Enabled })
			usable := followed || selectedEnabled && !row.moved
			if comp.Model != modelKey(selection, model) || shown.SendBlocked == usable {
				t.Fatalf("composer selects %s, send blocked %t", comp.Model, shown.SendBlocked)
			}

			actions.compact("")
			settle()
			saves, compactions = requests()
			revision := uint64(6)
			if row.follows != (protocol.ProviderID{}) {
				revision = 7
				if !row.moved {
					saved(saves)
				}
			}
			if row.follows == (protocol.ProviderID{}) || row.moved {
				if len(saves) != 0 {
					t.Fatalf("compaction saved %+v", saves)
				}
			}
			if len(compactions) != 1 || compactions[0] != revision {
				t.Fatalf("compactions at revisions %v, want one at %d", compactions, revision)
			}
			wantCompactProvider := row.follows
			if wantCompactProvider == (protocol.ProviderID{}) && selectedEnabled {
				wantCompactProvider = row.selected
			}
			source.mu.Lock()
			gotCompactProvider := compactProvider
			source.mu.Unlock()
			failTitle := i18n.In("en", "chat_compact_start_failed")
			if wantCompactProvider != (protocol.ProviderID{}) {
				if gotCompactProvider != wantCompactProvider || tester.HasText(failTitle) || tester.HasText(unavailableMessage) {
					t.Fatalf("compaction provider = %s (want %s), unexpected failure toast in %q", gotCompactProvider, wantCompactProvider, tester.Texts())
				}
			} else if gotCompactProvider != (protocol.ProviderID{}) || !tester.HasText(failTitle) || !tester.HasText(unavailableMessage) {
				t.Fatalf("compaction without successor admitted %s or missed failure toast; texts: %q", gotCompactProvider, tester.Texts())
			}
		})
	}
}
