package node

import (
	"context"
	"database/sql"

	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/operations"
	"github.com/ai-poet/cheaprouter-harness/internal/node/files"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func (node *Node) inspectWorkspaceFiles(ctx context.Context, request protocol.Request, root string, access core.FileAccess) (protocol.Output, error) {
	return files.Inspect(ctx, root, request.Command, access)
}
func (node *Node) readWorkspaceArtifact(ctx context.Context, request protocol.Request) (protocol.Output, error) {
	var capture store.ArtifactRead
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		capture, err = store.CaptureArtifactRead(ctx, db, request)
		return err
	})
	if err != nil {
		return protocol.Output{}, err
	}
	capture.Content.Html, err = files.ReadArtifactHTML(ctx, node.profile.Ownership.Path, capture.Content.Artifact, capture.Digest)
	if err != nil {
		return protocol.Output{}, err
	}
	err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		current, err := store.CaptureArtifactRead(ctx, db, request)
		if err != nil {
			return err
		}
		if current.Digest != capture.Digest || current.Content.Artifact.Revision != capture.Content.Artifact.Revision {
			return unavailable()
		}
		return nil
	})
	return protocol.Output{Kind: "artifact", Data: capture.Content}, err
}
func (node *Node) restoreWorkspaceCheckpoint(ctx context.Context, request protocol.Request, root string, effect *workspaceCompletion) (protocol.Output, error) {
	input := request.Command.Data.(*protocol.CommandRestoreFileCheckpointPayload)
	var capture protocol.ConversationCheckpointContent
	var settled []*protocol.ConversationCheckpointVersion
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		capture, settled, err = store.PrepareCheckpointRestore(ctx, db, input.Session, input.Checkpoint, input.Worktree, root)
		return err
	})
	if err != nil {
		return protocol.Output{}, err
	}
	if err = node.protectEntry(root, capture.File.Path); err != nil {
		return protocol.Output{}, err
	}
	defer node.watches.changed(root, nil)
	value, err := files.RestoreCheckpoint(ctx, root, capture, settled)
	if err == nil {
		// The file is restored; that it was undone is stored with the receipt.
		effect.external = true
		effect.commit = func(ctx context.Context, db *sql.Conn, out protocol.Output) (protocol.Output, []protocol.Event, error) {
			return out, nil, store.MarkCheckpointRestored(ctx, db, input.Checkpoint)
		}
	}
	return protocol.Output{Kind: "file_restored", Data: value}, err
}
func (node *Node) applyWorkspaceHunk(ctx context.Context, request protocol.Request, root string) (protocol.Output, error) {
	input := *request.Command.Data.(*protocol.ConversationCheckpointHunkSelection)
	var capture files.HunkChange
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		capture, err = store.PrepareTurnHunk(ctx, db, input, root)
		return err
	})
	if err != nil {
		return protocol.Output{}, err
	}
	if err = node.protectEntry(root, input.Path); err != nil {
		return protocol.Output{}, err
	}
	defer node.watches.changed(root, nil)
	if input.Operation == protocol.ConversationCheckpointHunkOperationReject {
		value, err := files.RejectHunk(ctx, root, input, capture)
		return protocol.Output{Kind: "hunk_applied", Data: value}, err
	}
	if input.ExpectedIndex == nil {
		return protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "staging a hunk requires the index revision"}
	}
	index, err := node.git.EditIndex(ctx, root, input.Path, *input.ExpectedIndex, input.ExpectedHead, input.ExpectedRevision, func(text string) (string, error) { return files.ApplyHunkText(text, capture.Before, capture.After) })
	value := protocol.ConversationCheckpointHunkApplied{Session: input.Session, Turn: input.Turn, Path: input.Path, Hunk: input.Hunk, Revision: input.ExpectedRevision, Index: &index}
	return protocol.Output{Kind: "hunk_applied", Data: value}, err
}

func (node *Node) writeWorkspaceArtifact(ctx context.Context, request protocol.Request, effect *workspaceCompletion) (protocol.Output, error) {
	catalog, err := operations.Bundled()
	if err != nil {
		return protocol.Output{}, err
	}
	var classes []string
	if request.Plugin != nil {
		classes = catalog.ArtifactClasses[request.Plugin.Package.Name]
	}
	var capture store.ArtifactWrite
	err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		capture, err = store.CaptureArtifactWrite(ctx, db, request, classes, time.Now())
		return err
	})
	if err != nil {
		return protocol.Output{}, err
	}
	node.stored.publish(effect)
	digest, err := files.WriteArtifactHTML(ctx, node.profile.Ownership.Path, capture.Artifact, capture.HTML)
	if err != nil {
		return protocol.Output{}, err
	}
	effect.external = true
	effect.commit = func(ctx context.Context, db *sql.Conn, out protocol.Output) (protocol.Output, []protocol.Event, error) {
		value, err := store.PublishArtifactWrite(ctx, db, capture, digest)
		return value, nil, err
	}
	return protocol.Output{Kind: "artifact_written", Data: protocol.OutputArtifactWrittenPayload{Artifact: capture.Artifact, Warnings: capture.Warnings}}, nil
}
