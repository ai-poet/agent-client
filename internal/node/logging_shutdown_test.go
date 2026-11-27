package node

import (
	"context"
	"errors"
	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"log/slog"
	"path/filepath"
	"sync"
	"testing"
)

func TestDispatchShutdownAdmission(t *testing.T) {
	t.Setenv("SAILRY_TELEMETRY", "off")
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	profile := filepath.Join(root, "profile")
	closeLog, err := logging.Install(profile, "host")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(closeLog)
	owner, err := startFixtureNode(t.Context(), profile)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = owner.Stop(context.Background()) })

	catalog := protocol.Command{Kind: "read_resource_catalog"}
	during := protocol.NewRequest(owner.ID(), catalog)
	after := protocol.NewRequest(owner.ID(), catalog)
	remote := protocol.NodeID{1}
	unreachable := protocol.NewRequest(remote, protocol.Command{Kind: "read_knowledge_citation", Data: protocol.KnowledgeCitationRequest{
		Node: remote, Collection: protocol.NewKnowledgeID(), Chunk: "fixture", Revision: "1", Limit: 1,
	}})
	var reports []logging.Fields
	logging.SetReport(func(_ slog.Level, fields logging.Fields) {
		switch fields["request"] {
		case during.ID.String(), after.ID.String(), unreachable.ID.String():
			reports = append(reports, fields)
		}
	})
	defer logging.SetReport(nil)
	dispatch := func(ctx context.Context, request protocol.Request) *protocol.Fault {
		t.Helper()
		admitted, err := owner.Dispatch(ctx, owner.ID(), request)
		fault, ok := err.(*protocol.Fault)
		if !ok || fault.Code != protocol.ErrorCodeUnavailable || admitted.Completion != nil {
			t.Fatalf("%s: admission = %+v, error = %v", request.Command.Kind, admitted, err)
		}
		return fault
	}
	fault := dispatch(t.Context(), unreachable)
	if !fault.ReportOwned || len(reports) != 1 ||
		reports[0]["command_stage"] != "admission" || reports[0]["request"] != unreachable.ID.String() {
		t.Fatalf("running Node lost an unavailable admission report: fault %+v, reports %v", fault, reports)
	}

	// Hold one operation so reporting is still attached when the catalog read
	// reaches the closed gate after authorization.
	if err := owner.begin(); err != nil {
		t.Fatal(err)
	}
	release := sync.OnceFunc(owner.work.Done)
	defer release()
	wait, cancel := context.WithCancel(t.Context())
	cancel()
	var stopErr error
	readCtx := stopOnCommandAuthorization{Context: t.Context(), stop: sync.OnceFunc(func() { stopErr = owner.Stop(wait) })}
	checkClosed := func(ctx context.Context, request protocol.Request) {
		t.Helper()
		if fault := dispatch(ctx, request); fault.Message != unavailable().Message || fault.ReportOwned || len(reports) != 1 {
			t.Fatalf("closed admission produced a fault report: fault %+v, reports %v", fault, reports)
		}
	}
	checkClosed(readCtx, during)
	if !errors.Is(stopErr, context.Canceled) {
		t.Fatalf("stop waiter = %v", stopErr)
	}
	release()
	if err := owner.Stop(t.Context()); err != nil {
		t.Fatal(err)
	}
	checkClosed(t.Context(), after)
}

type stopOnCommandAuthorization struct {
	context.Context
	stop func()
}

func (ctx stopOnCommandAuthorization) Value(key any) any {
	if _, captured := key.(capturedDispatchKey); captured {
		ctx.stop()
	}
	return ctx.Context.Value(key)
}
