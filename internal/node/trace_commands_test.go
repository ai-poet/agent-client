package node

import (
	"archive/zip"
	"bytes"
	"database/sql"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/link"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestTraceCommandsRequireVisibleSessionAndController(t *testing.T) {
	node := nodeFixture(t)
	session := sessionFixture(t, node)
	dir := filepath.Join(node.profile.Ownership.Path, "traces", session.ID.String())
	capture := func() {
		t.Helper()
		if err := os.MkdirAll(dir, 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, "trace.jsonl"), []byte(fmt.Sprintf("{\"v\":1,\"session\":%q,\"turn\":%q,\"attempt\":1}\n", session.ID.String(), protocol.NewTurnID().String())), 0600); err != nil {
			t.Fatal(err)
		}
	}
	capture()
	read := protocol.Command{Kind: "read_trace", Data: protocol.TraceQuery{Session: session.ID}}
	clear := protocol.Command{Kind: "clear_trace", Data: protocol.CommandReadSessionPayload{Session: session.ID}}
	ctx := t.Context()
	other := protocol.NodeID{74}
	if err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error { return store.PairLinkPeer(ctx, db, other, link.DirectionController) }); err != nil {
		t.Fatal(err)
	}
	for _, cmd := range []protocol.Command{read, clear, {Kind: "export_trace", Data: protocol.TraceExportQuery{Session: session.ID, Mode: "raw", Scope: "all"}}} {
		req := protocol.NewRequest(node.ID(), cmd)
		if reply := dispatchFixture(t, node, other, req); reply.Fault == nil || reply.Fault.Code != protocol.ErrorCodePermissionDenied {
			t.Fatalf("private %s: %+v", cmd.Kind, reply)
		}
		plugin := protocol.DefaultPluginContext()
		req.Plugin = &plugin
		if reply := dispatchFixture(t, node, node.ID(), req); reply.Fault == nil || reply.Fault.Code != protocol.ErrorCodePermissionDenied {
			t.Fatalf("plugin %s: %+v", cmd.Kind, reply)
		}
	}
	if got := commandFixture(t, node, read).Data.(protocol.TracePage); len(got.Records) != 1 {
		t.Fatalf("owner read: %+v", got)
	}
	q := protocol.TraceExportQuery{Session: session.ID, Scope: "all", Mode: "shareable"}
	d := commandFixture(t, node, protocol.Command{Kind: "export_trace", Data: q}).Data.(protocol.TraceDownload)
	if _, err := node.Open(ctx, other, d.Stream); err == nil {
		t.Fatal("different caller opened trace stream")
	}
	stream, err := node.Open(ctx, node.ID(), d.Stream)
	if err != nil {
		t.Fatal(err)
	}
	data, err := io.ReadAll(stream)
	stream.Close()
	if err != nil || uint64(len(data)) != d.Size {
		t.Fatal("incomplete trace download", err)
	}
	if _, err = zip.NewReader(bytes.NewReader(data), int64(len(data))); err != nil {
		t.Fatal(err)
	}
	if _, err = node.Open(ctx, node.ID(), d.Stream); err == nil {
		t.Fatal("trace stream reused")
	}
	controller := client.New(link.NewLocal(node.ID(), node.ID(), node))
	if err := controller.Start(ctx); err != nil {
		t.Fatal(err)
	}
	var received bytes.Buffer
	err = controller.DownloadTrace(ctx, q, &received)
	controller.Close()
	if err != nil {
		t.Fatal(err)
	}
	archive, err := zip.NewReader(bytes.NewReader(received.Bytes()), int64(received.Len()))
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := archive.Open("manifest.json")
	if err != nil {
		t.Fatal(err)
	}
	content, err := io.ReadAll(manifest)
	manifest.Close()
	if err != nil || !bytes.Contains(content, []byte(session.ID.String())) {
		t.Fatal("missing downloaded session manifest", err)
	}
	req := protocol.NewRequest(node.ID(), clear)
	if got := dispatchFixture(t, node, node.ID(), req); got.Fault != nil || got.Output.Kind != "trace_cleared" {
		t.Fatalf("clear: %+v", got)
	}
	if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("clear retained trace directory: %v", err)
	}
	capture()
	if got := dispatchFixture(t, node, node.ID(), req); got.Fault != nil {
		t.Fatal(got.Fault)
	}
	if got := commandFixture(t, node, read).Data.(protocol.TracePage); len(got.Records) != 1 {
		t.Fatal("replayed clear removed a later capture")
	}
	commandFixture(t, node, protocol.Command{Kind: "remove_session", Data: protocol.CommandRemoveSessionPayload{Session: session.ID, ExpectedRevision: session.Revision}})
	if got := dispatchFixture(t, node, node.ID(), protocol.NewRequest(node.ID(), read)); got.Fault == nil || got.Fault.Code != protocol.ErrorCodeNotFound {
		t.Fatalf("removed trace read: %+v", got)
	}
	// Stop joins the post-commit file collection.
	if err := node.Stop(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("removal retained trace directory: %v", err)
	}
}

func TestTraceArchiveAndInterruptedCleanup(t *testing.T) {
	node := nodeFixture(t)
	s := sessionFixture(t, node)
	dir := filepath.Join(node.profile.Ownership.Path, "traces", s.ID.String())
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	commandFixture(t, node, protocol.Command{Kind: "archive_sessions", Data: protocol.CommandArchiveSessionsPayload{Expected: []protocol.SessionRevision{{Session: s.ID, Revision: s.Revision}}}})
	if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("archived trace retained: %v", err)
	}
	// Simulate a prior process stopping after the state change but before collection.
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := store.CleanInactiveTraces(t.Context(), node.profile.Store); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("orphan retained: %v", err)
	}
}
