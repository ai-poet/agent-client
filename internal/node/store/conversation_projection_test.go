package store

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"fmt"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"lukechampine.com/blake3"
)

func TestCanonicalProjectionSuppressesOnlyLegacyDuplicateDiagnostics(t *testing.T) {
	const diagnostic = "Model stream ended before a completion event"
	for _, sample := range []struct {
		name    string
		failure bool
		blocks  []canonical.Block
		want    []protocol.ConversationPart
	}{
		{"legacy diagnostic", true, []canonical.Block{canonical.Text(0, diagnostic)}, nil},
		{"partial before diagnostic", true, []canonical.Block{canonical.Text(0, "Partial answer"), canonical.Text(1, diagnostic)}, []protocol.ConversationPart{{Kind: "text", Data: "Partial answer"}}},
		{"partial after diagnostic", true, []canonical.Block{canonical.Text(0, diagnostic), canonical.Text(1, "Partial answer")}, []protocol.ConversationPart{{Kind: "text", Data: "Partial answer"}}},
		{"ordinary matching text", false, []canonical.Block{canonical.Text(0, diagnostic)}, []protocol.ConversationPart{{Kind: "text", Data: diagnostic}}},
		{"near match", true, []canonical.Block{canonical.Text(0, diagnostic+"\n")}, []protocol.ConversationPart{{Kind: "text", Data: diagnostic + "\n"}}},
		{"matching reasoning", true, []canonical.Block{{Index: 0, Kind: "thinking", Data: &canonical.Thinking{Text: diagnostic}}}, []protocol.ConversationPart{{Kind: "thinking", Data: diagnostic}}},
	} {
		t.Run(sample.name, func(t *testing.T) {
			message := canonical.NewMessage(canonical.Assistant, sample.blocks...)
			if sample.failure {
				message.Error = &canonical.ResponseError{Message: diagnostic}
			}
			record := canonical.NewRecord(protocol.NewTurnID(), "message", &message, time.Unix(1, 0))
			record.Provenance.Author = "assistant"
			before, err := json.Marshal(record)
			if err != nil {
				t.Fatal(err)
			}
			entry, err := CanonicalEntry(record, 1)
			if err != nil {
				t.Fatal(err)
			}
			if len(entry.Parts) != len(sample.want) {
				t.Fatalf("projected parts=%+v, want %+v", entry.Parts, sample.want)
			}
			for index, want := range sample.want {
				if !reflect.DeepEqual(entry.Parts[index], want) {
					t.Fatalf("projected part=%+v, want %+v", entry.Parts[index], want)
				}
			}
			after, err := json.Marshal(record)
			if err != nil || !bytes.Equal(before, after) {
				t.Fatal("projection changed the canonical diagnostic or partial content")
			}
		})
	}
}

// A record with many responses resolves them in one lookup, with the pairing
// each response gets from its own narrow lookup.
func TestRecordToolResponsesPairAsTheirSingleLookups(t *testing.T) {
	t.Parallel()
	storage, _, session, turn, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	err := storage.Node.Write(ctx, func(db *sql.Conn) error {
		request := func(index uint64, id, path string) canonical.Block {
			return canonical.Block{Index: index, Kind: "tool_request", Data: &canonical.ToolRequest{ID: id, Name: "read_file", Arguments: json.RawMessage(`{"path":"` + path + `"}`)}}
		}
		for _, blocks := range [][]canonical.Block{{request(0, "first", "a"), request(1, "reused", "b")}, {request(0, "second", "c")}, {request(0, "reused", "d")}} {
			m := canonical.NewMessage(canonical.Assistant, blocks...)
			r := canonical.NewRecord(turn, "message", &m, now)
			r.Provenance.Author = "assistant"
			if _, _, err := AppendCanonical(ctx, db, session.ID, r); err != nil {
				return err
			}
		}
		var blocks []canonical.Block
		for index, id := range []string{"first", "second", "reused", "unknown"} {
			body := []byte(`"done"`)
			blocks = append(blocks, canonical.Block{Index: uint64(index), Kind: "tool_response", Data: &canonical.ToolResponse{ID: id, Name: "read_file", Status: canonical.ToolSuccess, Revision: fmt.Sprintf("%x", blake3.Sum256(body)), Result: canonical.Payload{Value: body}}})
		}
		m := canonical.NewMessage(canonical.User, blocks...)
		r := canonical.NewRecord(turn, "message", &m, now)
		r.Provenance.Author = "tool"
		entry, _, err := AppendCanonical(ctx, db, session.ID, r)
		if err != nil {
			return err
		}
		requests := recordToolRequests(r)
		if requests == nil {
			t.Fatal("a record with several responses kept one lookup per response")
		}
		want := map[string]string{"first": `{"path":"a"}`, "second": `{"path":"c"}`}
		for _, block := range m.Content {
			response := block.Data.(*canonical.ToolResponse)
			together, err := requests.find(ctx, db, r, entry.Sequence, response)
			if err != nil {
				return err
			}
			single, err := toolRequests(nil).find(ctx, db, r, entry.Sequence, response)
			if err != nil {
				return err
			}
			if (together == nil) != (single == nil) || together != nil && !bytes.Equal(together.Arguments, single.Arguments) {
				t.Fatalf("%s paired %+v together and %+v alone", response.ID, together, single)
			}
			if arguments, ok := want[response.ID]; ok != (together != nil) || ok && string(together.Arguments) != arguments {
				t.Fatalf("%s paired %+v", response.ID, together)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

// Exercise the append path at the size of a large batched tool reply. Report
// elapsed time for comparison on the same host, without a timing assertion.
func TestAppendLargeToolResponseRecord(t *testing.T) {
	storage, _, session, turn, now := agentHistoryFixture(t, protocol.WorkModeCode)
	ctx := t.Context()
	body := json.RawMessage(`"` + strings.Repeat("x", 9000) + `"`)
	revision := fmt.Sprintf("%x", blake3.Sum256(body))
	var calls, replies []canonical.Block
	for i := range 395 {
		id := fmt.Sprintf("call-%03d", i)
		calls = append(calls, canonical.Block{Index: uint64(i), Kind: "tool_request", Data: &canonical.ToolRequest{ID: id, Name: "read_file", Arguments: json.RawMessage(`{"path":"a"}`)}})
		replies = append(replies, canonical.Block{Index: uint64(i), Kind: "tool_response", Data: &canonical.ToolResponse{ID: id, Name: "read_file", Status: canonical.ToolSuccess, Revision: revision, Result: canonical.Payload{Value: body}}})
	}
	appendMessage := func(author string, m canonical.Message) error {
		r := canonical.NewRecord(turn, "message", &m, now)
		r.Provenance.Author = author
		return storage.Node.Write(ctx, func(db *sql.Conn) error {
			entry, _, err := AppendCanonical(ctx, db, session.ID, r)
			if err == nil && len(entry.Parts) != len(m.Content) {
				return fmt.Errorf("projected %d parts from %d blocks", len(entry.Parts), len(m.Content))
			}
			return err
		})
	}
	if err := appendMessage("assistant", canonical.NewMessage(canonical.Assistant, calls...)); err != nil {
		t.Fatal(err)
	}
	m := canonical.NewMessage(canonical.User, replies...)
	start := time.Now()
	if err := appendMessage("tool", m); err != nil {
		t.Fatal(err)
	}
	t.Logf("committed %d tool responses in %s", len(replies), time.Since(start))
}

func TestStoredProjectionKeepsResourcePositionsAfterLegacyDiagnostic(t *testing.T) {
	storage, _, session, turn, now := agentHistoryFixture(t, protocol.WorkModeCode)
	message := canonical.NewMessage(canonical.Assistant,
		canonical.Text(0, "Fixture diagnostic"),
		canonical.Block{Index: 1, Kind: "image", Data: &canonical.AssetContent{Asset: canonical.AssetRef{Digest: strings.Repeat("a", 64), Mime: "image/png", Size: 1}}},
		canonical.Text(2, "Actual partial answer"),
	)
	message.Error = &canonical.ResponseError{Message: "Fixture diagnostic"}
	record := canonical.NewRecord(turn, "message", &message, now)
	record.Provenance.Author = "assistant"
	err := storage.Node.Read(t.Context(), func(db *sql.Conn) error {
		entry, err := projectStoredEntry(t.Context(), db, record, 1, nil)
		if err != nil {
			return err
		}
		if len(entry.Parts) != 2 || entry.Parts[0].Kind != "image" || entry.Parts[1].Kind != "text" || entry.Parts[1].Data != "Actual partial answer" {
			t.Fatalf("legacy diagnostic shifted or hid actual content: %+v", entry.Parts)
		}
		image, ok := entry.Parts[0].Data.(*protocol.ConversationImage)
		if !ok || image.Entry != record.ID || image.Part != 1 || image.Attachment.Spec.Worktree != session.Worktree {
			t.Fatalf("image lost its canonical identity after diagnostic suppression: %+v", image)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
