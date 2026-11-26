package node

import (
	"context"
	"log/slog"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestRoleCaptureCommandLog(t *testing.T) {
	profile := filepath.Join(t.TempDir(), "profile")
	closeLog, err := logging.Install(profile, "host")
	if err != nil {
		t.Fatal(err)
	}
	defer closeLog()
	node, err := startFixtureNode(context.Background(), profile)
	if err != nil {
		t.Fatal(err)
	}
	defer node.Stop(context.Background())
	session := sessionFixture(t, node)
	provider := forwardingProvider(t, node, "https://private.example/v1")
	role := protocol.DefaultRoleProfile()
	role.ID, role.Key, role.Name = protocol.NewRoleID(), "role-log-test", "Private role name"
	role.Model = &protocol.RoleModel{Provider: provider.ID, Model: provider.DefaultModel}
	commandFixture(t, node, protocol.Command{Kind: "put_role", Data: protocol.CommandPutRolePayload{Role: role}})

	var fields logging.Fields
	var degraded []logging.Fields
	logging.SetReport(func(_ slog.Level, values logging.Fields) {
		if values["area"] == "role_capture" {
			degraded = append(degraded, values)
		}
		if values["command"] == "set_session_team" || values["command"] == "create_session" {
			fields = values
		}
	})
	defer logging.SetReport(nil)
	for _, test := range []struct {
		name   string
		model  *protocol.RoleModel
		reason protocol.ConfigurationReason
	}{
		{"missing override provider", &protocol.RoleModel{Provider: protocol.NewProviderID(), Model: "private-model"}, protocol.ConfigurationProviderMissing},
		{"missing override model", &protocol.RoleModel{Provider: provider.ID, Model: "private-model"}, ""},
		{"disabled role provider", nil, protocol.ConfigurationProviderDisabled},
	} {
		t.Run(test.name, func(t *testing.T) {
			input := protocol.CommandSetSessionExecutionPayload{Session: session.ID, ExpectedRevision: session.Revision, Execution: protocol.ConversationExecutionTeam}
			var command protocol.Command
			wantProvider := provider.ID
			if test.model != nil {
				input.Models = map[protocol.RoleID]protocol.RoleModel{role.ID: *test.model}
				command = protocol.Command{Kind: "set_session_team", Data: input}
				wantProvider = test.model.Provider
			} else {
				provider.Enabled = false
				commandFixture(t, node, protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: provider, ExpectedRevision: provider.Revision}})
				command = protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Config: &session.Config}}
			}
			request := protocol.NewRequest(node.ID(), command)
			fields = nil
			result := dispatchFixture(t, node, node.ID(), request)
			if test.model == nil {
				if result.Fault != nil || len(degraded) != 1 {
					t.Fatalf("capture fault=%v, warnings=%v", result.Fault, degraded)
				}
				warning := degraded[0]
				if warning["phase"] != "degraded" || warning["role"] != role.ID.String() || warning["provider"] != wantProvider.String() || warning["reason"] != string(test.reason) {
					t.Fatalf("wrong degradation warning: %v", warning)
				}
				selected := session.Config
				selected.Provider, selected.Model, selected.Credential = provider.ID, provider.DefaultModel, provider.Credential
				command.Data = protocol.CommandCreateSessionPayload{Config: &selected}
				rejected := dispatchFixture(t, node, node.ID(), protocol.NewRequest(node.ID(), command))
				if rejected.Fault == nil || rejected.Fault.ConfigurationReason() != protocol.ConfigurationProviderDisabled || rejected.Fault.Message != "selected model provider is unavailable" {
					t.Fatalf("disabled lead was not rejected as a selected-model failure: %v", rejected.Fault)
				}
				return
			}
			if result.Fault == nil || result.Fault.Code != protocol.ErrorCodeNotConfigured || result.Fault.ConfigurationReason() != test.reason {
				t.Fatalf("unexpected fault: %+v", result.Fault)
			}
			if fields["request"] != request.ID.String() || fields["command_stage"] != "completion" || fields["role"] != role.ID.String() || fields["provider"] != wantProvider.String() {
				t.Fatalf("wrong roster context: %v", fields)
			}
			if test.reason == "" {
				if fields["reason"] != "role_model_missing" || result.Fault.Message != "role model is unavailable" {
					t.Fatalf("missing model indistinguishable: %v, %v", fields, result.Fault)
				}
			} else if fields["configuration_reason"] != string(test.reason) || fields["reason"] != nil {
				t.Fatalf("wrong provider reason: %v", fields)
			}
			if remote := logging.Remote(fields); remote["role"] != nil || remote["provider"] != nil {
				t.Fatalf("roster identifiers left the local log: %v", remote)
			}
		})
	}
}
