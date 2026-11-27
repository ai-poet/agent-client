package engine

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestDelegationRosterAndAdmissionUseInheritedRoleTools(t *testing.T) {
	t.Parallel()
	f, x := delegationFixture(t)
	builtin := func(name string) *[]protocol.PluginConversationTool {
		return &[]protocol.PluginConversationTool{{Kind: "builtin", Data: &protocol.PluginConversationToolBuiltinPayload{Name: name}}}
	}
	pkg := &[]protocol.PluginConversationTool{{Kind: "package", Data: &protocol.PluginConversationToolPackagePayload{Package: "fixture", Name: "read"}}}
	roles := []protocol.RoleProfile{
		{ID: protocol.NewRoleID(), Revision: 1, Key: "missing", Name: "Missing", Tools: builtin("missing_tool")},
		{Key: "coordinator", Name: "Coordinator", Tools: builtin("spawn_agent")},
		{Key: "reader", Name: "Reader", ReadOnly: true, Tools: builtin("mutate_fixture")},
	}
	f.r.tools = []Tool{packageTool("fixture", "read", "", true, nil), objectTool("mutate_fixture", false, false, nil)}
	x.inv.Turn.Roles.Profiles = roles
	if err := f.r.captureTools(x); err != nil {
		t.Fatal(err)
	}
	if spawn := findTool(x.tools, "spawn_agent"); spawn == nil || !strings.Contains(spawn.Definition.Description, "No usable roles") {
		t.Fatal("the spawn declaration advertised an unusable roster")
	}
	x.inv.Turn.Roles.Profiles = append(roles, protocol.RoleProfile{Key: "fixture", Name: "Reader", ReadOnly: true, Tools: pkg})
	if err := f.r.captureTools(x); err != nil {
		t.Fatal(err)
	}
	roster := f.r.delegationRoster(x, store.AgentState{})
	if !strings.Contains(roster, "fixture (Reader) [read only]") ||
		!strings.Contains(roster, "coordinator: required tools are unavailable: spawn_agent") {
		t.Fatalf("inaccurate delegation roster: %s", roster)
	}
	for _, role := range roles {
		args, _ := json.Marshal(map[string]string{"role": role.Key, "task": "Check the fixture"})
		result := f.r.spawnChild(x, canonical.ToolRequest{ID: role.Key, Name: "spawn_agent", Arguments: args}, args)
		if result.Failure == nil || !strings.Contains(result.Failure.Message, "required tools are unavailable") {
			t.Fatalf("role %q was not refused for its tool requirements: %+v", role.Key, result)
		}
	}
	x.inv.Message.References = []protocol.ConversationReference{{Target: protocol.ConversationReferenceTarget{
		Kind: "agent", Data: &protocol.RoleReference{ID: roles[0].ID, Revision: roles[0].Revision},
	}}}
	selected, err := f.r.selectedAgent(x, f.state())
	var fault *protocol.Fault
	if selected || !errors.As(err, &fault) || fault.Code != protocol.ErrorCodeInvalidRequest || !strings.Contains(fault.Message, "required tools are unavailable: missing_tool") {
		t.Fatalf("selected role was not refused for its tool requirements: selected=%t, err=%v", selected, err)
	}
	if turns := f.childTurns(); len(turns) != 0 {
		t.Fatalf("an unavailable role was admitted: %v", turns)
	}
}
