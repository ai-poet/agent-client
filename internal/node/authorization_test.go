package node

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node/admission"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestCommandAuthorizationCoverage(t *testing.T) {
	registered := map[string]bool{}
	for _, kind := range protocol.CommandKinds() {
		if removedCapabilityCommand(kind) {
			continue
		}
		registered[kind] = true
		if commandPolicies[kind] == 0 {
			t.Errorf("command %s has no authorization policy", kind)
		}
	}
	for kind := range commandPolicies {
		if !registered[kind] {
			t.Errorf("authorization policy names unregistered command %s", kind)
		}
	}
	if _, ok := commandPolicies["unregistered_command"]; ok {
		t.Fatal("unknown command has a policy")
	}
}

func TestUsageV2RefusesPlugin(t *testing.T) {
	node := nodeFixture(t)
	query := protocol.DefaultUsageQuery()
	query.Dimension, query.EndMs = protocol.UsageDimensionModel, 1
	req := protocol.NewRequest(node.ID(), protocol.Command{Kind: "read_usage_v2", Data: query})
	scope := protocol.DefaultPluginContext()
	req.Plugin = &scope
	if reply := dispatchFixture(t, node, node.ID(), req); reply.Fault == nil || reply.Fault.Code != protocol.ErrorCodePermissionDenied {
		t.Fatalf("plugin read_usage_v2: %+v", reply)
	}
}

func TestProjectUsersAndSubscriptionRevocation(t *testing.T) {
	node := nodeFixture(t)
	ctx := t.Context()
	owner, other := protocol.NodeID{31}, protocol.NodeID{32}
	if err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		if err := store.PairLinkPeer(ctx, db, owner, link.DirectionController); err != nil {
			return err
		}
		return store.PairLinkPeer(ctx, db, other, link.DirectionController)
	}); err != nil {
		t.Fatal(err)
	}
	send := func(caller protocol.NodeID, command protocol.Command) link.Response {
		return dispatchFixture(t, node, caller, protocol.NewRequest(node.ID(), command))
	}
	draft := protocol.DefaultProjectsDraft()
	draft.Name = "Private"
	draft.Visibility = protocol.ProjectVisibilityPrivate
	draft.Source = protocol.ProjectsSource{Kind: "local"}
	draft.Path = t.TempDir()
	created := send(owner, protocol.Command{Kind: "create_project", Data: draft})
	if created.Fault != nil {
		t.Fatal(created.Fault)
	}
	project := created.Output.Data.(protocol.Project)
	snapshot := func(caller protocol.NodeID) protocol.Snapshot {
		t.Helper()
		value := send(caller, protocol.Command{Kind: "snapshot"})
		if value.Fault != nil {
			t.Fatal(value.Fault)
		}
		return value.Output.Data.(protocol.Snapshot)
	}
	if got := snapshot(other); len(got.Projects) != 0 || len(got.Worktrees) != 0 {
		t.Fatalf("private resources leaked: %+v", got.Projects)
	}
	catalog := func(caller protocol.NodeID) protocol.ResourceSource {
		t.Helper()
		value := send(caller, protocol.Command{Kind: "read_resource_catalog"})
		if value.Fault != nil || value.Output.Kind != "resource_catalog" {
			t.Fatalf("resource catalog: %+v", value)
		}
		result := value.Output.Data.(protocol.ResourceCatalog)
		if result.Entry != node.ID() || len(result.Sources) != 1 {
			t.Fatalf("unexpected resource catalog scope: %+v", result)
		}
		return result.Sources[0]
	}
	if got := catalog(other); len(got.Projects) != 0 || len(got.Worktrees) != 0 {
		t.Fatal("another user's private resources entered the catalog")
	}
	if got := catalog(owner); len(got.Projects) != 1 || len(got.Worktrees) != 1 {
		t.Fatal("owner's catalog omitted private resources")
	}
	own := snapshot(owner)
	if own.User == nil || len(own.Projects) != 1 || len(own.Worktrees) != 1 {
		t.Fatal("owner snapshot missing resources or identity")
	}
	tree := own.Worktrees[0].ID
	terminal := protocol.DefaultTerminalInfo()
	terminal.ID, terminal.Worktree, terminal.Owner = protocol.NewTerminalID(), &tree, &owner
	terminal.Status.Kind, terminal.Revision = "running", 1
	node.terminalChanged(terminal)
	takeover := protocol.Command{Kind: "takeover_terminal", Data: protocol.CommandTakeoverTerminalPayload{
		Terminal: terminal.ID, ExpectedRevision: terminal.Revision,
	}}
	var refused *protocol.Fault
	if accepted, err := node.Dispatch(ctx, other, protocol.NewRequest(node.ID(), takeover)); !errors.As(err, &refused) ||
		refused.Code != protocol.ErrorCodePermissionDenied || accepted.Receipt.ID != (protocol.RequestID{}) {
		t.Fatalf("a private terminal takeover was not rejected before admission: %+v, %v", accepted.Receipt, err)
	}
	if seen, hidden := snapshot(owner).Terminals, snapshot(other).Terminals; len(seen) != 1 || seen[0].ID != terminal.ID || len(hidden) != 0 {
		t.Fatalf("private terminal snapshot coverage: owner %v, other %v", seen, hidden)
	}
	denied := send(other, protocol.Command{Kind: "read_worktree_catalog", Data: protocol.CommandReadWorktreeCatalogPayload{Worktree: tree}})
	if denied.Fault == nil || denied.Fault.Code != protocol.ErrorCodePermissionDenied {
		t.Fatalf("private raw ID access: %+v", denied)
	}
	if stream, err := node.Subscribe(ctx, other, protocol.Topic{Kind: "files", Data: tree}); err == nil {
		stream.Close()
		t.Fatal("private subscription allowed")
	}
	shared := protocol.ProjectVisibilityShared
	result := send(owner, protocol.Command{Kind: "update_project", Data: protocol.CommandUpdateProjectPayload{Expected: project, Name: project.Name, Path: project.Path, Appearance: project.Appearance, Visibility: &shared}})
	if result.Fault != nil {
		t.Fatal(result.Fault)
	}
	project = result.Output.Data.(protocol.Project)
	if len(snapshot(other).Projects) != 1 {
		t.Fatal("shared project absent")
	}
	if got := catalog(other); len(got.Projects) != 1 || len(got.Worktrees) != 1 {
		t.Fatal("shared project or worktree absent from resource catalog")
	}
	config := protocol.DefaultSessionConfig()
	config.Provider, config.Model = protocol.NewProviderID(), "fixture"
	config.Mode, config.Permission = protocol.WorkModeCode, protocol.PermissionProject
	createdSession := send(owner, protocol.Command{Kind: "create_session", Data: protocol.CommandCreateSessionPayload{Project: &project.ID, Config: &config}})
	if createdSession.Fault != nil {
		t.Fatal(createdSession.Fault)
	}
	session := createdSession.Output.Data.(protocol.Session)
	rename := protocol.NewRequest(node.ID(), protocol.Command{Kind: "rename_session", Data: protocol.CommandRenameSessionPayload{Session: session.ID, ExpectedRevision: session.Revision, Title: "Shared edit"}})
	if result := dispatchFixture(t, node, other, rename); result.Fault != nil {
		t.Fatal(result.Fault)
	}
	stream, err := node.Subscribe(ctx, other, protocol.Topic{Kind: "files", Data: tree})
	if err != nil {
		t.Fatal(err)
	}
	defer stream.Close()
	if _, err = stream.Next(ctx); err != nil {
		t.Fatal(err)
	}
	private := protocol.ProjectVisibilityPrivate
	result = send(owner, protocol.Command{Kind: "update_project", Data: protocol.CommandUpdateProjectPayload{Expected: project, Name: project.Name, Path: project.Path, Appearance: project.Appearance, Visibility: &private}})
	if result.Fault != nil {
		t.Fatal(result.Fault)
	}
	timed, cancel := context.WithTimeout(ctx, testBudget()/30)
	defer cancel()
	if _, err = stream.Next(timed); err == nil {
		t.Fatal("detail observation survived lost access")
	}
	if len(snapshot(other).Projects) != 0 {
		t.Fatal("revoked project remains projected")
	}
	if got := catalog(other); len(got.Projects) != 0 || len(got.Worktrees) != 0 {
		t.Fatal("resource catalog retained the formerly shared project or worktree")
	}
	body, err := json.Marshal(rename)
	if err != nil {
		t.Fatal(err)
	}
	inspection := send(other, protocol.Command{Kind: "inspect_request", Data: protocol.CommandInspectRequestPayload{ID: rename.ID, Digest: admission.Digest(body)}})
	if inspection.Fault != nil {
		t.Fatal(inspection.Fault)
	}
	outcome := inspection.Output.Data.(protocol.OutputRequestOutcomePayload).Outcome
	resultBody, err := json.Marshal(outcome)
	if err != nil {
		t.Fatal(err)
	}
	var decoded protocol.RequestOutcome
	if err = json.Unmarshal(resultBody, &decoded); err != nil {
		t.Fatal(err)
	}
	denial := decoded.Data.(*protocol.Result[protocol.Output, protocol.Fault]).Err
	if decoded.Kind != "completed" || denial == nil || denial.Code != protocol.ErrorCodePermissionDenied {
		t.Fatalf("private result disclosed by inspection: %s", resultBody)
	}
	if replay := dispatchFixture(t, node, other, rename); replay.Fault == nil || replay.Fault.Code != protocol.ErrorCodePermissionDenied {
		t.Fatalf("private result disclosed by replay: %+v", replay)
	}
	admin := send(other, protocol.Command{Kind: "save_power_settings", Data: protocol.PowerSettings{}})
	if admin.Fault == nil || admin.Fault.Code != protocol.ErrorCodePermissionDenied {
		t.Fatalf("non-admin host mutation: %+v", admin)
	}
}
