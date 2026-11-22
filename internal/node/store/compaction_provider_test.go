package store

import (
	"database/sql"
	"fmt"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestMissingSessionProviderAtAdmission(t *testing.T) {
	storage, node, session, _, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	if err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		// Another provider still offers the same model. Neither command may use it.
		session.Config.Provider = protocol.NewProviderID()
		out, err := queueFixtureCommand(ctx, db, storage, node, "set_session_config", &protocol.CommandSetSessionConfigPayload{
			Session: session.ID, ExpectedRevision: session.Revision, Config: session.Config,
		}, now)
		if err != nil {
			return err
		}
		session = out.Data.(protocol.Session)
		for _, command := range []protocol.Command{
			{Kind: "compact_context", Data: &protocol.CommandCompactContextPayload{Session: session.ID, ExpectedRevision: session.Revision}},
			{Kind: "submit_turn", Data: &protocol.CommandSubmitTurnPayload{Session: session.ID, ExpectedRevision: session.Revision, Message: protocol.ConversationInput{Text: "control"}}},
		} {
			_, err := queueFixtureCommand(ctx, db, storage, node, command.Kind, command.Data, now)
			requireConfigurationReason(t, err, protocol.ConfigurationProviderMissing)
			want := "model provider is unavailable"
			if command.Kind == "compact_context" {
				want = "Select an available model before compacting this conversation."
			}
			if failure := err.(*protocol.Fault); failure.Message != want {
				return fmt.Errorf("%s: %v", command.Kind, failure)
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestCompactionProviderAtClaim(t *testing.T) {
	for _, tc := range []struct {
		name    string
		damage  bool
		raw     []byte
		newLane bool
	}{
		{name: "JSON null", damage: true, raw: []byte("null")},
		{name: "SQL null", damage: true},
		{name: "invalid JSON", damage: true, raw: []byte("{")},
		{name: "missing capture after lane switch", damage: true, newLane: true},
		{name: "valid capture after lane switch", newLane: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			storage, node, session, parent, now := agentHistoryFixture(t, protocol.WorkModeCode)
			ctx := t.Context()
			if err := storage.Node.Write(ctx, func(db *sql.Conn) error {
				if err := queueFixtureInput(ctx, db, session.ID, parent, now); err != nil {
					return err
				}
				if _, _, err := FinishTurn(ctx, db, parent, protocol.ConversationStatusCompleted, nil, now); err != nil {
					return err
				}
				out, err := queueFixtureCommand(ctx, db, storage, node, "compact_context", &protocol.CommandCompactContextPayload{
					Session: session.ID, ExpectedRevision: session.Revision,
				}, now)
				if err != nil {
					return err
				}
				turn := out.Data.(protocol.QueuedTurn).ID
				if tc.damage {
					if _, err := db.ExecContext(ctx, "UPDATE agent_runs SET provider=?2 WHERE turn=?1", turn.String(), tc.raw); err != nil {
						return err
					}
				}
				if tc.newLane {
					other := providerFixture()
					if _, _, err := ExecuteProviderCommand(ctx, db, node, protocol.Request{Command: protocol.Command{Kind: "save_provider", Data: &protocol.CommandSaveProviderPayload{Provider: other}}}, nil); err != nil {
						return err
					}
					config := session.Config
					config.Provider = other.ID
					if _, err := queueFixtureCommand(ctx, db, storage, node, "set_session_config", &protocol.CommandSetSessionConfigPayload{
						Session: session.ID, ExpectedRevision: session.Revision, Config: config,
					}, now); err != nil {
						return err
					}
				}
				inv, _, err := ClaimNext(ctx, db, now)
				if err != nil {
					return err
				}
				if tc.damage && tc.newLane {
					run, err := ReadAgentRun(ctx, db, turn)
					if err != nil {
						return err
					}
					if inv != nil || run.Status != protocol.ConversationStatusFailed || run.Error == nil || run.Error.Code != protocol.ErrorCodeNotConfigured {
						return fmt.Errorf("expected a failed compaction, got %+v", run)
					}
				} else if inv == nil || inv.Run.Turn != turn || inv.Provider == nil || inv.Provider.ID != session.Config.Provider {
					return fmt.Errorf("did not claim compaction with its selected provider")
				}
				return nil
			}); err != nil {
				t.Fatal(err)
			}
		})
	}
}

// A session stores its provider's ID and credential, never the provider's
// revision. Both turn commands resolve that ID alike at every stage of a
// managed provider's life; they differ only in the configuration the
// controller saved before sending them.
func TestSessionProviderLifecycleAtAdmission(t *testing.T) {
	storage, node, fixture, _, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	if err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		provider := func(kind string, data any) (protocol.ConversationProvider, error) {
			out, _, err := ExecuteProviderCommand(ctx, db, node, protocol.Request{Command: protocol.Command{Kind: kind, Data: data}}, nil)
			saved, _ := out.Data.(protocol.ConversationProvider)
			return saved, err
		}
		save := func(value protocol.ConversationProvider, key string) (protocol.ConversationProvider, error) {
			input := &protocol.CommandSaveProviderPayload{Provider: value, ExpectedRevision: value.Revision}
			if key != "" {
				secret := protocol.NewSecret(key)
				input.Secret = &secret
			}
			return provider("save_provider", input)
		}
		// admit sends both commands at the session's current revision.
		admit := func(session protocol.SessionID) (compact, submit protocol.QueuedTurn, compactErr, submitErr error) {
			current, err := ReadSession(ctx, db, session)
			if err != nil {
				return compact, submit, err, err
			}
			out, compactErr := queueFixtureCommand(ctx, db, storage, node, "compact_context", &protocol.CommandCompactContextPayload{Session: session, ExpectedRevision: current.Revision}, now)
			compact, _ = out.Data.(protocol.QueuedTurn)
			out, submitErr = queueFixtureCommand(ctx, db, storage, node, "submit_turn", &protocol.CommandSubmitTurnPayload{Session: session, ExpectedRevision: current.Revision, Message: protocol.ConversationInput{Text: "control"}}, now)
			submit, _ = out.Data.(protocol.QueuedTurn)
			return compact, submit, compactErr, submitErr
		}
		// refused returns an error rather than failing inside the transaction.
		refused := func(stage string, session protocol.SessionID, reason protocol.ConfigurationReason) error {
			_, _, compactErr, submitErr := admit(session)
			for kind, failure := range map[string]error{"compact_context": compactErr, "submit_turn": submitErr} {
				want := "model provider is unavailable"
				if kind == "compact_context" {
					want = compactionModelUnavailable
				}
				refusal, ok := failure.(*protocol.Fault)
				if !ok || refusal.Code != protocol.ErrorCodeNotConfigured || refusal.ConfigurationReason() != reason || refusal.Message != want {
					return fmt.Errorf("%s: %s = %v, want %s", stage, kind, failure, reason)
				}
			}
			return nil
		}

		unbound := providerFixture()
		unbound.ID = providers.CheapRouterProviderID
		unbound, err := save(unbound, "unbound-key")
		if err != nil {
			return err
		}
		config := fixture.Config
		config.Provider, config.Model, config.Credential = unbound.ID, unbound.DefaultModel, unbound.Credential
		var sessions [2]protocol.Session // one the controller moves, one it leaves
		for index := range sessions {
			out, _, err := createSession(ctx, db, node, storage.Profile, protocol.CommandCreateSessionPayload{Config: &config})
			if err != nil {
				return err
			}
			sessions[index] = out.Data.(protocol.Session)
		}
		moved, kept := sessions[0].ID, sessions[1].ID

		// A sign-in or a group switch saves the same ID again: a newer revision
		// and another credential. The session still names the earlier ones.
		first := *unbound.Credential
		if unbound, err = save(unbound, "rotated-key"); err != nil {
			return err
		}
		compact, submit, compactErr, submitErr := admit(moved)
		if compactErr != nil || submitErr != nil {
			return fmt.Errorf("a newer provider revision refused the session: %v, %v", compactErr, submitErr)
		}
		if unbound.Revision != 2 || *unbound.Credential == first || compact.Config.Provider != unbound.ID || submit.Config.Provider != unbound.ID ||
			compact.Config.Credential == nil || *compact.Config.Credential != first || *submit.Config.Credential != first {
			return fmt.Errorf("revision %d, credential %v, captured %+v and %+v", unbound.Revision, unbound.Credential, compact.Config, submit.Config)
		}

		// Route reconciliation gives the model another provider ID and turns
		// the earlier one off. The Node never substitutes that provider.
		lane := providerFixture()
		lane.ID = providers.CheapRouterLaneID("claude")
		if lane, err = save(lane, "lane-key"); err != nil {
			return err
		}
		unbound.Enabled = false
		if unbound, err = save(unbound, ""); err != nil {
			return err
		}
		for _, session := range []protocol.SessionID{moved, kept} {
			if err := refused("disabled", session, protocol.ConfigurationProviderDisabled); err != nil {
				return err
			}
		}

		// Send saves the composer's selection first, so its turn is admitted.
		// A command still at the revision before that save is not.
		stale, err := ReadSession(ctx, db, moved)
		if err != nil {
			return err
		}
		config.Provider, config.Credential = lane.ID, lane.Credential
		if _, err = queueFixtureCommand(ctx, db, storage, node, "set_session_config", &protocol.CommandSetSessionConfigPayload{Session: moved, ExpectedRevision: stale.Revision, Config: config}, now); err != nil {
			return err
		}
		_, err = queueFixtureCommand(ctx, db, storage, node, "compact_context", &protocol.CommandCompactContextPayload{Session: moved, ExpectedRevision: stale.Revision}, now)
		if failure, ok := err.(*protocol.Fault); !ok || failure.Code != protocol.ErrorCodeRevisionConflict {
			return fmt.Errorf("compaction at the earlier revision: %v", err)
		}
		compact, submit, compactErr, submitErr = admit(moved)
		if compactErr != nil || submitErr != nil || compact.Config.Provider != lane.ID || submit.Config.Provider != lane.ID ||
			compact.Config.Credential == nil || *compact.Config.Credential != *lane.Credential ||
			submit.Config.Credential == nil || *submit.Config.Credential != *lane.Credential {
			return fmt.Errorf("saved selection was not admitted: %v, %v", compactErr, submitErr)
		}

		if _, err = provider("remove_provider", &protocol.CommandRemoveProviderPayload{Provider: unbound.ID, ExpectedRevision: unbound.Revision}); err != nil {
			return err
		}
		return refused("removed", kept, protocol.ConfigurationProviderRemoved)
	}); err != nil {
		t.Fatal(err)
	}
}
