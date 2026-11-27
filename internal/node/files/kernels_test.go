package files

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func filesFixture(t testing.TB) (string, *core.Files) {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	f, err := core.OpenFiles(root, core.ManualFiles)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = f.Close() })
	return root, f
}
func TestFileTextKernels(t *testing.T) {
	t.Parallel()
	for _, test := range []struct {
		name, before, old, new, want string
		all, fail                    bool
	}{{"CRLF line boundary", "a\r\nb\r\n", "\nb\n", "\nc\n", "a\r\nc\r\n", false, false}, {"ambiguity", "same same", "same", "next", "", false, true}, {"replace all", "same same", "same", "next", "next next", true, false}, {"overlapping occurrences", "aaa", "aa", "x", "", false, true}} {
		t.Run(test.name, func(t *testing.T) {
			after, err := PrepareEdits(test.before, []protocol.FileEdit{{OldText: test.old, NewText: test.new, ReplaceAll: test.all}})
			if (err != nil) != test.fail || !test.fail && after != test.want {
				t.Fatalf("result %q, error %v", after, err)
			}
		})
	}
	patch := "*** Begin Patch\n*** Update File: note.txt\n@@\n a\n-b\n+c\n*** End Patch"
	parsed, err := ParsePatch(patch)
	if err != nil || len(parsed) != 1 {
		t.Fatalf("parse: %v", err)
	}
	after, err := ApplyPatchChunks("a\r\nb\r\n", parsed[0].Chunks)
	if err != nil || after != "a\r\nc\r\n" {
		t.Fatalf("patch %q: %v", after, err)
	}
	if _, err = ApplyPatchChunks("a\nx\na\n", []PatchChunk{{Old: []string{"a"}, New: []string{"b"}}}); err == nil || !strings.Contains(err.Error(), "lines 1 and 3") {
		t.Fatalf("ambiguous hunk locations: %v", err)
	}
	hunks, err := SplitHunks("note.txt", "one\ntwo\nthree\n", "one\nnext\nthree\n", 0)
	if err != nil || len(hunks) != 1 || !validRevision(hunks[0].Hunk.ID) {
		t.Fatalf("hunks: %+v %v", hunks, err)
	}
	restored, err := ApplyHunkText("outside\none\nnext\nthree\n", hunks[0].After, hunks[0].Before)
	if err != nil || restored != "outside\none\ntwo\nthree\n" {
		t.Fatalf("hunk restore %q: %v", restored, err)
	}
}
func TestPreparedFileChanges(t *testing.T) {
	t.Parallel()
	root, _ := filesFixture(t)
	ctx := context.Background()
	if err := os.WriteFile(filepath.Join(root, "first.txt"), []byte("before\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	expected := revision([]byte("before\n"))
	command := protocol.Command{Kind: "apply_changes", Data: &protocol.CommandApplyChangesPayload{Changes: []protocol.FileChange{{Kind: "write", Data: protocol.FileChangeWritePayload{Path: "first.txt", Text: "after\n", ExpectedRevision: expected}}, {Kind: "create", Data: protocol.FileChangeCreatePayload{Path: "second.txt", Text: "ours\n"}}, {Kind: "create", Data: protocol.FileChangeCreatePayload{Path: "third.txt", Text: "later\n"}}}}}
	prepared, err := Prepare(ctx, root, command, core.ManualFiles)
	if err != nil || prepared.Rejected != nil {
		t.Fatalf("prepare: %+v %v", prepared, err)
	}
	if err = os.WriteFile(filepath.Join(root, "second.txt"), []byte("external\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := Apply(ctx, root, prepared)
	if err != nil {
		t.Fatal(err)
	}
	result := out.Data.(protocol.FileChanges)
	if result.Status != protocol.ChangesStatusPartial || result.Files[0].Status != protocol.ChangeStatusApplied || result.Files[1].Status != protocol.ChangeStatusFailed || result.Files[2].Status != protocol.ChangeStatusNotAttempted {
		t.Fatalf("effects: %+v", result)
	}
	data, err := os.ReadFile(filepath.Join(root, "second.txt"))
	if err != nil || string(data) != "external\n" {
		t.Fatalf("external content changed: %q %v", data, err)
	}
	if _, err = os.Stat(filepath.Join(root, "third.txt")); !os.IsNotExist(err) {
		t.Fatalf("unattempted file exists: %v", err)
	}
	if _, err = ResolvePatchWithRevisions(ctx, root, "*** Begin Patch\n*** Update File: first.txt\n@@\n-after\n+next\n*** End Patch", nil, core.ManualFiles); err == nil || !strings.Contains(err.Error(), "first.txt") {
		t.Fatalf("unobserved file update: %v", err)
	}
}
func TestNotebookPreparation(t *testing.T) {
	t.Parallel()
	draft := `{"nbformat":4,"nbformat_minor":5,"metadata":{},"cells":[{"cell_type":"code","metadata":{},"source":["old\n"],"outputs":[{"output_type":"stream","text":"cached"}],"execution_count":2}]}`
	prepared, err := PrepareCreation("note.ipynb", draft)
	if err != nil {
		t.Fatal(err)
	}
	stable, err := PrepareCreation("note.ipynb", draft)
	if err != nil || stable != prepared {
		t.Fatalf("unstable IDs: %v", err)
	}
	edit := protocol.NotebookEdit{Path: "note.ipynb", ExpectedRevision: revision([]byte(prepared)), Edits: []protocol.NotebookChange{{Kind: "replace", Data: protocol.NotebookChangeReplacePayload{Index: 0, Source: "next\n"}}}}
	after, err := PrepareNotebook(prepared, edit)
	if err != nil {
		t.Fatal(err)
	}
	var document struct {
		Cells []struct {
			ID      string  `json:"id"`
			Outputs []any   `json:"outputs"`
			Count   *uint64 `json:"execution_count"`
			Source  string  `json:"source"`
		}
	}
	if err = json.Unmarshal([]byte(after), &document); err != nil || len(document.Cells) != 1 || document.Cells[0].ID == "" || len(document.Cells[0].Outputs) != 0 || document.Cells[0].Count != nil || document.Cells[0].Source != "next\n" {
		t.Fatalf("edited notebook: %s (%v)", after, err)
	}
	if _, err = parseNotebook(strings.Replace(draft, `["old\n"]`, `null`, 1)); err == nil {
		t.Fatal("null notebook source was accepted")
	}
}

func TestRepreparePatchKeepsOriginalTargets(t *testing.T) {
	for _, tc := range []struct{ name, patch, before, current, want string }{
		{"exact edit", "*** Update File: a\n@@\n-old\n+new", "old\n", "extra\nold\ntail", "extra\nnew\ntail"},
		{"ambiguous edit", "*** Update File: a\n@@\n-old\n+new", "old\n", "old\nold\n", ""},
		{"append", "*** Update File: a\n@@\n+new", "old\n", "extra\n", ""},
		{"delete", "*** Delete File: a", "old\n", "new\n", ""},
		{"create", "*** Add File: a\n+new", "", "appeared\n", ""},
		{"move destination", "*** Update File: a\n*** Move to: b", "old\n", "old\n", ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx := context.Background()
			root, _ := filesFixture(t)
			write := func(path, data string) {
				if err := os.WriteFile(filepath.Join(root, path), []byte(data), 0600); err != nil {
					t.Fatal(err)
				}
			}
			if tc.before != "" {
				write("a", tc.before)
			}
			patch := "*** Begin Patch\n" + tc.patch + "\n*** End Patch"
			changes, err := ResolvePatchWithRevisions(ctx, root, patch, map[string]string{"a": revision([]byte(tc.before))}, core.RestrictedFiles)
			if err != nil {
				t.Fatal(err)
			}
			p, err := Prepare(ctx, root, protocol.Command{Kind: "apply_changes", Data: &protocol.CommandApplyChangesPayload{Changes: changes}}, core.RestrictedFiles)
			if err != nil || p.Rejected != nil {
				t.Fatalf("prepare: %+v %v", p, err)
			}
			write("a", tc.current)
			if tc.name == "move destination" {
				write("b", "occupied")
			}
			fresh, err := RepreparePatch(ctx, root, patch, p)
			if tc.want == "" {
				if err == nil {
					t.Fatal("changed precondition accepted")
				}
				return
			}
			if err != nil || *fresh.Changes[0].After != tc.want {
				t.Fatalf("reprepare: %+v %v", fresh, err)
			}
		})
	}
}
