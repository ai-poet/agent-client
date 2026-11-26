package store

import (
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestRoleWithMissingImplicitProviderNeverFallsBackToParentModel(t *testing.T) {
	t.Parallel()
	storage, node, base, _, _ := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	role := protocol.DefaultRoleProfile()
	role.ID, role.Key, role.Name = protocol.NewRoleID(), "absent", "Absent provider"
	role.Model = &protocol.RoleModel{Provider: protocol.NewProviderID(), Model: "retired"}
	err := storage.Node.Read(ctx, func(db *sql.Conn) error {
		roster := protocol.DefaultRoleSnapshot()
		roster.Profiles = []protocol.RoleProfile{role}
		roster, err := completeRoleSnapshot(ctx, db, node, roster, nil)
		if err != nil {
			return err
		}
		// The import validates the same frozen state without a live provider.
		roster, err = completeCapturedRoleSnapshot(roster, nil)
		if err != nil {
			return err
		}
		if err := CapturedRoleAvailability(roster, role); err == nil || !strings.Contains(err.Error(), role.Model.Provider.String()) {
			return fmt.Errorf("missing provider was not deferred: %v", err)
		}
		// An explicit valid override repairs the role without needing its obsolete default.
		override := protocol.RoleModel{Provider: base.Config.Provider, Model: base.Config.Model}
		roster, err = AssignSessionMemberModel(ctx, db, node, roster, role.ID, &override)
		if err != nil {
			return err
		}
		return CapturedRoleAvailability(roster, role)
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestUnavailableRoleIsFrozenWithoutBlockingItsPeers(t *testing.T) {
	t.Parallel()
	storage, node, base, _, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	var id protocol.SessionID
	var stale protocol.RoleProfile
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		record, err := readProviderConfiguration(ctx, db, base.Config.Provider)
		if err != nil {
			return err
		}
		model := record.Provider.Models[0]
		model.ID = "retired"
		record.Provider.Models = append(record.Provider.Models, model)
		save := func() error {
			body, err := encode(record)
			if err != nil {
				return err
			}
			_, err = db.ExecContext(ctx, "UPDATE providers SET body=?2 WHERE id=?1", base.Config.Provider.String(), body)
			return err
		}
		if err = save(); err != nil {
			return err
		}
		for _, name := range []string{"healthy", "stale"} {
			role := protocol.DefaultRoleProfile()
			role.ID, role.Key, role.Name = protocol.NewRoleID(), name, name
			role.Model = &protocol.RoleModel{Provider: base.Config.Provider, Model: base.Config.Model}
			if name == "stale" {
				role.Name = "Retired reviewer"
				role.Model.Model = model.ID
			}
			role, err = putRole(ctx, db, role, 0)
			if err != nil {
				return err
			}
			if name == "stale" {
				stale = role
			}
		}
		record.Provider.Models = record.Provider.Models[:1]
		if err = save(); err != nil {
			return err
		}
		out, _, err := createSession(ctx, db, node, storage.Profile, protocol.CommandCreateSessionPayload{Config: &base.Config})
		if err != nil {
			return err
		}
		id = out.Data.(protocol.Session).ID
		// Repairing the catalog must not change the frozen disposition.
		record.Provider.Models = append(record.Provider.Models, model)
		return save()
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := storage.Close(); err != nil {
		t.Fatal(err)
	}
	storage = reopenSchemaFixture(t, storage.Profile, node)
	err = storage.Node.Write(ctx, func(db *sql.Conn) error {
		session, err := ReadSession(ctx, db, id)
		if err != nil {
			return err
		}
		check := func(err error) error {
			var f *protocol.Fault
			if !errors.As(err, &f) || f.Code != protocol.ErrorCodeNotConfigured ||
				!strings.Contains(f.Message, stale.Name) || !strings.Contains(f.Message, stale.Key) || !strings.Contains(f.Message, base.Config.Provider.String()) ||
				!strings.Contains(f.Message, "model was unavailable") {
				return fmt.Errorf("missing frozen role diagnostic: %v", err)
			}
			return nil
		}
		message := protocol.ConversationInput{References: []protocol.ConversationReference{{
			Label: stale.Name, Target: protocol.ConversationReferenceTarget{Kind: "agent", Data: &protocol.RoleReference{ID: stale.ID, Revision: stale.Revision}},
		}}}
		if err := check(validateAgentReferences(ctx, db, session, message, nil)); err != nil {
			return err
		}
		parent, err := delegationFixtureParent(ctx, db, node, session, now)
		if err != nil {
			return err
		}
		for _, key := range []string{stale.Key, "healthy"} {
			call, err := delegationFixtureCall(ctx, db, parent, "spawn_agent", map[string]string{"task": "review", "role": key}, now)
			if err != nil {
				return err
			}
			child, _, err := DelegateAgent(ctx, db, node, parent, call, nil, true, now)
			if key == stale.Key {
				err = check(err)
			} else if err == nil && (child.Provider == nil || child.Provider.ID != base.Config.Provider || child.Turn.Config.Model != base.Config.Model) {
				return errors.New("healthy sibling lost its model")
			}
			if err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func TestForwardedUnavailableRoleDoesNotRequireRuntimeCredential(t *testing.T) {
	t.Parallel()
	storage, node, controller, ctx := controllerCacheFixture(t)
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		p := providerFixture()
		p.Revision, p.Enabled = 1, false
		p.Credential = &protocol.CredentialRef{Node: controller, ID: protocol.NewCredentialID()}
		role := protocol.DefaultRoleProfile()
		role.ID, role.Key, role.Name, role.Revision = protocol.NewRoleID(), "review", "Reviewer", 1
		role.Model = &protocol.RoleModel{Provider: p.ID, Model: p.DefaultModel}
		for kind, record := range map[string]protocol.ConfigurationFollowRecord{
			"providers": followRecord(t, p.ID.String(), p.Revision, p, nil),
			"roles":     followRecord(t, role.ID.String(), role.Revision, role, nil),
		} {
			if _, err := applyConfigurationCache(ctx, db, node, controller, protocol.ConfigurationManifest{
				Source: controller, Category: kind, Revision: 1, Records: []protocol.ConfigurationFollowRecord{record},
			}); err != nil {
				return err
			}
		}
		roster := protocol.DefaultRoleSnapshot()
		roster.Profiles, roster.Providers = []protocol.RoleProfile{role}, []protocol.ConversationProvider{p}
		frozen, err := cachedForwardRoles(ctx, db, node, roster)
		if err != nil {
			return err
		}
		if err := CapturedRoleAvailability(frozen, role); err == nil || !strings.Contains(err.Error(), "disabled") {
			return fmt.Errorf("lost deferred rejection on import: %v", err)
		}
		roster.Providers[0].Enabled = true
		if _, err := cachedForwardRoles(ctx, db, node, roster); !providerFixtureFault(err, protocol.ErrorCodeRevisionConflict) {
			return fmt.Errorf("accepted altered provider state: %v", err)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
