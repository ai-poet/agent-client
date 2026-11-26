package live

import (
	"reflect"
	"testing"
	"time"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/node/providers"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestAffectedCheapRoleBindings(t *testing.T) {
	lane := providers.CheapRouterLaneID(core.LaneClaude)
	custom := protocol.ProviderID{7}
	current := []protocol.ConversationProvider{
		{ID: providers.CheapRouterProviderID}, {ID: custom},
		{ID: lane, Enabled: true, Models: []protocol.ConversationModel{{ID: "offered"}}},
	}
	for _, tc := range []struct {
		model *protocol.RoleModel
		want  bool
	}{
		{nil, false},
		{&protocol.RoleModel{Provider: custom}, false},
		{&protocol.RoleModel{Provider: providers.CheapRouterProviderID}, true},
		{&protocol.RoleModel{Provider: lane, Model: "withdrawn"}, true},
		{&protocol.RoleModel{Provider: lane, Model: "offered"}, false},
		{&protocol.RoleModel{Provider: providers.CheapRouterLaneID(core.LaneGemini)}, false},
	} {
		snapshot := protocol.Snapshot{Roles: []protocol.RoleProfile{{Model: tc.model}}, Providers: current}
		if got := len(affectedCheapRoles(snapshot)) != 0; got != tc.want {
			t.Errorf("affected(%+v) = %t, want %t", tc.model, got, tc.want)
		}
	}
}

func TestRoleMigrationAfterCheapRouteReconciliation(t *testing.T) {
	gateway := newCheapGateway(t)
	const model = "gpt-5"
	gateway.catalogBody = `{"code":0,"data":{"items":[{"model":"gpt-5","billing_mode":"token","context_window":128000,"best_group":{"id":2}}]}}`
	var original protocol.RoleProfile
	d := openCheapDesktop(t, func(d *cheapDesktop) {
		d.signedIn(gateway.URL, cheapTestAccess, time.Hour)
		p, secret, err := providers.CheapRouterProvider(core.Gateway{BaseURL: gateway.URL, APIKey: cheapTestKey,
			Models: []core.Model{{ID: model}, {ID: "withdrawn"}}})
		if err != nil {
			t.Fatal(err)
		}
		d.do(protocol.Command{Kind: "save_provider", Data: protocol.CommandSaveProviderPayload{Provider: p, Secret: secret}})
		for _, key := range []string{"migrate", "concurrent", "manual"} {
			role := protocol.DefaultRoleProfile()
			role.ID, role.Key, role.Name = protocol.NewRoleID(), key, key
			role.Model = &protocol.RoleModel{Provider: p.ID, Model: model}
			role.Instructions, role.MaxTurns, role.ReadOnly = "Check the draft.", new(uint32(7)), true
			if key == "manual" {
				role.Model.Model = "withdrawn"
			}
			output := d.do(protocol.Command{Kind: "put_role", Data: protocol.CommandPutRolePayload{Role: role}})
			if key == "migrate" {
				original, _ = detail[protocol.RoleProfile](output.Data)
			}
		}
	})
	d.await("reconciled routes and affected catalog roles", func() bool {
		return d.account().Stage == cheapauthui.Ready && d.account().AffectedRoles == 3
	})
	d.accountPage()
	if !d.tester.HasText(i18n.In("zh-CN", "cheaprouter_roles_affected", "count", 3)) {
		t.Fatalf("missing affected-role guidance: %q", d.tester.Texts())
	}
	// A concurrent editor switches one role to follow the session just as
	// the user clicks migrate. Its revision must win, independently of the
	// sibling agent that does have an exact-model successor.
	race := func(request protocol.Request) {
		input, ok := detail[protocol.CommandPutRolePayload](request.Command.Data)
		if request.Command.Kind == "put_role" && ok && input.Role.Key == "concurrent" {
			input.Role.Model = nil
			if _, err := d.another.Do(t.Context(), protocol.Command{Kind: "put_role", Data: input}); err != nil {
				t.Error(err)
			}
		}
	}
	d.before.Store(&race)
	d.click(i18n.In("zh-CN", "cheaprouter_roles_migrate"))
	d.await("partial migration receipt", func() bool {
		return d.tester.HasText(i18n.In("zh-CN", "cheaprouter_roles_receipt", "saved", 1, "failed", 1, "skipped", 1)) &&
			d.tester.HasText(i18n.In("zh-CN", "cheaprouter_roles_manual", "names", "manual"))
	})
	d.await("remaining manual binding", func() bool { return d.account().AffectedRoles == 1 })
	roles, _ := detail[[]protocol.RoleProfile](d.do(protocol.Command{Kind: "list_roles"}).Data)
	original.Revision++
	original.Model.Provider = providers.CheapRouterLaneID(core.LaneCodex)
	found := false
	for _, role := range roles {
		switch role.Key {
		case "migrate":
			found = true
			if !reflect.DeepEqual(role, original) {
				t.Fatalf("migrated agent = %+v, want %+v", role, original)
			}
		case "concurrent":
			if role.Model != nil {
				t.Fatal("migration replaced a concurrent edit")
			}
		case "manual":
			if role.Revision != 1 || role.Model.Provider != providers.CheapRouterProviderID {
				t.Fatal("migration changed an agent without a successor")
			}
		}
	}
	if !found {
		t.Fatal("migrated agent missing from role catalog")
	}
}
