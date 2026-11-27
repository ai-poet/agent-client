package engine

import (
	"context"
	"database/sql"
	"errors"
	"slices"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func (f *engineFixture) pendingWriteApproval(call string) protocol.ApprovalID {
	f.t.Helper()
	for {
		var id string
		err := f.db.Node.Read(f.ctx, func(db *sql.Conn) error {
			return db.QueryRowContext(f.ctx, "SELECT id FROM agent_approvals WHERE call=?1 AND json_extract(body,'$.state')='pending'", call).Scan(&id)
		})
		if err == nil {
			key, err := protocol.ParseApprovalID(id)
			if err != nil {
				f.t.Fatal(err)
			}
			return key
		}
		if !errors.Is(err, sql.ErrNoRows) {
			f.t.Fatal(err)
		}
		select {
		case <-f.changed:
		case <-f.ctx.Done():
			f.t.Fatalf("waiting for approval of %s", call)
		}
	}
}

func (f *engineFixture) decideWrite(call, decision string) {
	f.t.Helper()
	f.command("resolve_approval", &protocol.CommandResolveApprovalPayload{
		Session: f.session.ID, Approval: f.pendingWriteApproval(call), Decision: protocol.ConversationDecision{Kind: decision},
	})
}

func writeBatchIDs(records []canonical.Record) []string {
	var ids []string
	for _, record := range records {
		if msg := record.Message(); msg != nil {
			for _, block := range msg.Content {
				if result, ok := block.Data.(*canonical.ToolResponse); ok {
					ids = append(ids, result.ID)
				}
			}
		}
	}
	return ids
}

func TestSameFileReadsAndWritesFollowRequestOrder(t *testing.T) {
	write := toolAccess{parallel: true, paths: []string{"a"}}
	read := toolAccess{parallel: true, read: true, paths: []string{"a"}}
	batch := []toolAccess{write, read, read, write, read}
	predecessors := [][]int{nil, {0}, {0}, {1, 2}, {3}}
	done := make([]bool, len(batch))
	err := runQueuedTools(t.Context(), 0, batch, func(i int) error {
		for _, prior := range predecessors[i] {
			if !done[prior] {
				t.Errorf("admitted call %d before call %d completed", i, prior)
			}
		}
		return nil
	}, func(int) {}, func(i int) { done[i] = true })
	if err != nil || slices.Contains(done, false) {
		t.Fatalf("queue completion %v, %v", done, err)
	}
}

func TestWorkspaceQueueDrainsIndependentWritesOnAdmissionError(t *testing.T) {
	release := make(chan struct{})
	started := make(chan struct{})
	stop := errors.New("admission failed")
	completed := false
	err := runQueuedTools(t.Context(), 0, []toolAccess{{paths: []string{"a"}}, {paths: []string{"b"}}}, func(i int) error {
		if i == 1 {
			select {
			case <-started:
			case <-t.Context().Done():
				return t.Context().Err()
			}
			close(release)
			return stop
		}
		return nil
	}, func(i int) {
		if i != 0 {
			t.Errorf("unexpected execution %d", i)
			return
		}
		close(started)
		<-release
	}, func(i int) { completed = true })
	if err != stop || !completed {
		t.Fatalf("drain: %v, %v", completed, err)
	}
}

func TestWorkspaceDisjointWriteBatchWaitsOnlyForTheSameFile(t *testing.T) {
	t.Parallel()
	// The second call targets the same file as the first. The third shares
	// only a path prefix and can run while the first is still executing.
	paths := map[string]string{"first": "assets", "again": "assets", "child": "assets/icon"}
	releaseFirst, releaseChild := make(chan struct{}), make(chan struct{})
	started := make(chan string, len(paths))
	write := objectTool("write_files", false, false, func(ctx context.Context, req canonical.ToolRequest) ToolResult {
		started <- req.ID
		var release <-chan struct{}
		switch req.ID {
		case "first":
			release = releaseFirst
		case "child":
			release = releaseChild
		}
		if release != nil {
			select {
			case <-release:
			case <-ctx.Done():
				return toolFault(ctx.Err())
			}
		}
		return ToolResult{Value: req.ID}
	})
	write.Approval = true
	write.FilePaths = func(req canonical.ToolRequest) []string { return []string{paths[req.ID]} }
	var receipts []string
	write.ObserveHistory = func(records []canonical.Record) { receipts = writeBatchIDs(records) }
	write.Prepare = func(_ context.Context, req canonical.ToolRequest) error {
		if req.ID == "again" && !slices.Contains(receipts, "first") {
			t.Errorf("same-file preparation did not observe the preceding write: %v", receipts)
		}
		return nil
	}
	m := &scriptedModel{scripts: []script{
		response(call("first", "write_files", "{}"), call("again", "write_files", "{}"), call("child", "write_files", "{}"), finish()),
		response(text("Done"), finish()),
	}}
	f := newFixture(t, m, write)
	f.start()
	turn := f.submit("Edit the files")
	waitStarted := func(want string) {
		t.Helper()
		select {
		case id := <-started:
			if id != want {
				t.Fatalf("started %q, want %q", id, want)
			}
		case <-f.ctx.Done():
			t.Fatalf("waiting for %s to start", want)
		}
	}
	f.decideWrite("first", "approve")
	waitStarted("first")
	f.decideWrite("child", "approve")
	waitStarted("child")
	// The slow independent operation cannot delay the second write once
	// the first write to that file is complete.
	close(releaseFirst)
	f.decideWrite("again", "approve")
	waitStarted("again")
	close(releaseChild)
	f.waitRun(turn, protocol.ConversationStatusCompleted)
	state := f.state()
	if ids := writeBatchIDs(state.Records); !slices.Equal(ids, []string{"first", "again", "child"}) {
		t.Fatalf("persisted receipt order: %v", ids)
	}
}
