package node

import (
	"database/sql"
	"errors"
	"sort"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/node/store"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func authorizationBoundaryCommandSet() map[string]struct{} {
	commands := map[string]struct{}{}
	for _, list := range []string{accessPersonalCommands, accessResourceCommands} {
		for _, kind := range strings.Fields(list) {
			commands[kind] = struct{}{}
		}
	}
	return commands
}

func registeredCommandSet() map[string]struct{} {
	registered := map[string]struct{}{}
	for _, kind := range protocol.CommandKinds() {
		registered[kind] = struct{}{}
	}
	return registered
}

func sortedCommandNames(commands map[string]struct{}) []string {
	names := make([]string, 0, len(commands))
	for kind := range commands {
		names = append(names, kind)
	}
	sort.Strings(names)
	return names
}

func TestRetiredCommandsExcludedFromAuthorization(t *testing.T) {
	conflicts := map[string]struct{}{}
	for kind := range authorizationBoundaryCommandSet() {
		if _, retired := retiredCommands[kind]; retired {
			conflicts[kind] = struct{}{}
		}
	}
	if len(conflicts) != 0 {
		t.Fatalf("retired commands remain in accessPersonalCommands/accessResourceCommands: %s", strings.Join(sortedCommandNames(conflicts), ", "))
	}
}

func TestAuthorizationBoundaryCommandsRegistered(t *testing.T) {
	registered := registeredCommandSet()
	for _, kind := range sortedCommandNames(authorizationBoundaryCommandSet()) {
		if _, ok := registered[kind]; !ok {
			t.Errorf("authorization allowlist names unregistered command %q", kind)
		}
	}
}

func TestRetiredCommandsRemainRegistered(t *testing.T) {
	registered := registeredCommandSet()
	for _, kind := range sortedCommandNames(retiredCommands) {
		if _, ok := registered[kind]; !ok {
			t.Errorf("retired command %q is no longer in the protocol registry; remove it from retiredCommands", kind)
		}
	}
}

func TestRetiredCommandsExecuteAsNotConfigured(t *testing.T) {
	node := nodeFixture(t)
	for _, test := range []struct {
		name    string
		command protocol.Command
	}{
		{name: "save_ssh", command: protocol.Command{Kind: "save_ssh", Data: protocol.DefaultCommandSaveSshPayload()}},
		{name: "read_office", command: protocol.Command{Kind: "read_office", Data: protocol.DefaultCommandReadOfficePayload()}},
		{name: "list_databases", command: protocol.Command{Kind: "list_databases"}},
	} {
		t.Run(test.name, func(t *testing.T) {
			request := protocol.NewRequest(node.ID(), test.command)
			err := node.profile.Store.Node.Read(t.Context(), func(db *sql.Conn) error {
				_, _, err := store.ExecuteCommand(t.Context(), db, node.ID(), node.ID(), "", request, store.PreparedPaths{})
				return err
			})
			var fault *protocol.Fault
			if !errors.As(err, &fault) {
				t.Fatalf("ExecuteCommand error = %v, want protocol fault", err)
			}
			if fault.Code == protocol.ErrorCodeInternal || fault.Code == protocol.ErrorCodeOutcomeUnknown {
				t.Fatalf("ExecuteCommand fault code = %q, must be a definite unavailable result", fault.Code)
			}
			if fault.Code != protocol.ErrorCodeNotConfigured {
				t.Fatalf("ExecuteCommand fault code = %q, want %q", fault.Code, protocol.ErrorCodeNotConfigured)
			}
		})
	}
}
