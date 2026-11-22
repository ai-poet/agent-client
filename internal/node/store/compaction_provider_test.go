package store

import (
	"database/sql"
	"fmt"
	"testing"

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
