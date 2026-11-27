package node

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/node/files"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"lukechampine.com/blake3"
)

// Independent child turns do not share an engine queue. Both can prepare
// against the same observed bytes before either enters the Node's IO boundary.
func TestWorkspaceSubagentPreparedEditsCannotOverwriteWinner(t *testing.T) {
	ctx := t.Context()
	node, err := StartWithOptions(ctx, filepath.Join(t.TempDir(), "profile"), Options{MCP: fixtureMCPOptions()})
	if err != nil {
		t.Fatal(err)
	}
	defer node.Stop(context.Background())
	project, tree := projectFixture(t, node)
	root, err := filepath.EvalSymlinks(project.Path)
	if err != nil {
		t.Fatal(err)
	}
	const name, before = "shared.txt", "original"
	if err := os.WriteFile(filepath.Join(root, name), []byte(before), 0600); err != nil {
		t.Fatal(err)
	}
	type outcome struct {
		want string
		err  error
	}
	var edits []*files.Prepared
	for _, after := range []string{"child one", "child two"} {
		old := before
		command := protocol.Command{Kind: "edit_file", Data: &protocol.CommandEditFilePayload{Worktree: tree, Path: name, OldText: &old, NewText: &after, ExpectedRevision: fmt.Sprintf("%x", blake3.Sum256([]byte(before)))}}
		prepared, err := files.Prepare(ctx, root, command, core.RestrictedFiles)
		if err != nil {
			t.Fatal(err)
		}
		edits = append(edits, prepared)
	}
	ready := make(chan struct{})
	results := make(chan outcome, len(edits))
	for _, prepared := range edits {
		request := protocol.NewRequest(node.ID(), prepared.Command)
		go func() {
			<-ready
			var effect workspaceCompletion
			_, err := node.workspaceIO(ctx, node.ID(), request, prepared, &effect)
			effect.close()
			results <- outcome{*prepared.Changes[0].After, err}
		}()
	}
	close(ready)
	var winner string
	conflicts := 0
	for range 2 {
		result := <-results
		var fault *protocol.Fault
		switch {
		case result.err == nil:
			winner = result.want
		case errors.As(result.err, &fault) && fault.Code == protocol.ErrorCodeRevisionConflict:
			conflicts++
		default:
			t.Fatalf("unexpected edit outcome: %v", result.err)
		}
	}
	got, err := os.ReadFile(filepath.Join(root, name))
	if err != nil || conflicts != 1 || winner == "" || string(got) != winner {
		t.Fatalf("got %q, winner %q, conflicts %d: %v", got, winner, conflicts, err)
	}
}
