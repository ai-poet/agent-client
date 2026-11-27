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
	anchor := "header"
	if after, err = ApplyPatchChunks("header\nbody\n", []PatchChunk{{Context: &anchor, New: []string{"last"}}}); err != nil || after != "header\nbody\nlast\n" {
		t.Fatalf("anchored append: %q %v", after, err)
	}
	if after, err = ApplyPatchChunks("hello\u2014\u201cworld\u201d\n", []PatchChunk{{Old: []string{`hello-"world"`}, New: []string{"updated"}}}); err != nil || after != "updated\n" {
		t.Fatalf("normalized punctuation: %q %v", after, err)
	}
	if after, err = ApplyPatchChunks("func main() {\n\tfmt.Println(\"hi\")   \n}\n", []PatchChunk{{Old: []string{"fmt.Println(\"hi\")"}, New: []string{"\tfmt.Println(\"bye\")"}}}); err != nil || !strings.Contains(after, "bye") {
		t.Fatalf("fuzzy patch failed: %q %v", after, err)
	}
	// Fuzzy patch matching preserves original text of untouched context lines (e.g., tabs in Makefile/Python).
	tabFile := "all:\n\techo one\n\techo two\n\techo three\n"
	tabPatch := "*** Begin Patch\n*** Update File: Makefile\n@@\n     echo one\n-    echo two\n+\techo TWO\n     echo three\n*** End Patch"
	parsedTab, err := ParsePatch(tabPatch)
	if err != nil || len(parsedTab) != 1 {
		t.Fatalf("parse tab patch: %v", err)
	}
	if after, err = ApplyPatchChunks(tabFile, parsedTab[0].Chunks); err != nil || after != "all:\n\techo one\n\techo TWO\n\techo three\n" {
		t.Fatalf("fuzzy patch rewrote untouched context lines: got %q, err=%v", after, err)
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
	if _, err = PreparePatch(ctx, root, "*** Begin Patch\n*** Update File: first.txt\n@@\n-after\n+next\n*** End Patch", protocol.WorktreeID{}, core.ManualFiles, func(p string) string { return p }, nil); err == nil || !strings.Contains(err.Error(), "first.txt") {
		t.Fatalf("unobserved file update: %v", err)
	}
}
func observedForPatch(t *testing.T, root string) map[string]string {
	t.Helper()
	out := map[string]string{}
	for _, name := range []string{"a", "b", "c", "Foo", "foo", "hardlink", "source"} {
		data, err := os.ReadFile(filepath.Join(root, name))
		if err == nil {
			out[name] = revision(data)
		} else if !os.IsNotExist(err) {
			t.Fatal(err)
		}
	}
	return out
}
func TestPatchOverwritesAndMovesInOrder(t *testing.T) {
	root, _ := filesFixture(t)
	for name, content := range map[string]string{"a": "source\n", "b": "destination\n", "c": "existing\n"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	patch := `*** Begin Patch
*** Update File: a
*** Move to: b
@@
-source
+moved
*** Update File: b
@@
-moved
+changed
*** Add File: c
+replaced
*** Add File: new/nested.txt
+nested
*** End Patch`
	ctx := context.Background()
	p, err := PreparePatch(ctx, root, patch, protocol.WorktreeID{}, core.ManualFiles, func(path string) string { return path }, observedForPatch(t, root))
	if err != nil {
		t.Fatal(err)
	}
	out, err := Apply(ctx, root, p)
	if err != nil || out.Data.(protocol.FileChanges).Status != protocol.ChangesStatusApplied {
		t.Fatalf("apply: %+v %v", out, err)
	}
	for name, want := range map[string]string{"b": "changed\n", "c": "replaced\n", "new/nested.txt": "nested\n"} {
		got, err := os.ReadFile(filepath.Join(root, name))
		if err != nil || string(got) != want {
			t.Fatalf("%s = %q, %v", name, got, err)
		}
	}
	if _, err := os.Stat(filepath.Join(root, "a")); !os.IsNotExist(err) {
		t.Fatalf("move source: %v", err)
	}
}

func TestPatchMovesCaseVariantsAndHardLinks(t *testing.T) {
	for _, tt := range []struct {
		name, destination, chunk, want string
		link                           bool
	}{
		{"nested rename", "new/dir/renamed", "", "original\n", false},
		{"case rename", "foo", "", "original\n", false},
		{"case rename with edit", "foo", "@@\n-original\n+edited\n", "edited\n", false},
		{"move over hard link", "hardlink", "", "original\n", true},
		{"edit and move over hard link", "hardlink", "@@\n-original\n+edited\n", "edited\n", true},
	} {
		t.Run(tt.name, func(t *testing.T) {
			root, _ := filesFixture(t)
			source := filepath.Join(root, "Foo")
			if err := os.WriteFile(source, []byte("original\n"), 0o700); err != nil {
				t.Fatal(err)
			}
			destination := filepath.Join(root, tt.destination)
			if tt.link {
				if err := os.Link(source, destination); err != nil {
					t.Skipf("hard links unavailable: %v", err)
				}
				if err := os.Link(source, filepath.Join(root, "retained")); err != nil {
					t.Fatal(err)
				}
			}
			ctx := context.Background()
			patch := "*** Begin Patch\n*** Update File: Foo\n*** Move to: " + tt.destination + "\n" + tt.chunk + "*** End Patch"
			prepared, err := PreparePatch(ctx, root, patch, protocol.WorktreeID{}, core.ManualFiles, func(path string) string { return path }, observedForPatch(t, root))
			if err != nil {
				t.Fatal(err)
			}
			out, err := Apply(ctx, root, prepared)
			if err != nil || out.Data.(protocol.FileChanges).Status != protocol.ChangesStatusApplied {
				t.Fatalf("move: %+v %v", out, err)
			}
			entries, err := os.ReadDir(root)
			if err != nil {
				t.Fatal(err)
			}
			for _, entry := range entries {
				if entry.Name() == "Foo" {
					t.Fatal("original directory entry still exists")
				}
			}
			if got, err := os.ReadFile(destination); err != nil || string(got) != tt.want {
				t.Fatalf("destination: %q %v", got, err)
			}
			if tt.link {
				if got, err := os.ReadFile(filepath.Join(root, "retained")); err != nil || string(got) != "original\n" {
					t.Fatalf("retained hard link: %q %v", got, err)
				}
			}
		})
	}
}

func TestMoveCreatesPatchParentsOnlyAfterRevisionCheck(t *testing.T) {
	root, fs := filesFixture(t)
	path := filepath.Join(root, "source")
	if err := os.WriteFile(path, []byte("original\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	dst := "new/target"
	if err := fs.MoveFile(ctx, "source", dst, revision([]byte("original\n")), false); err == nil {
		t.Fatal("ordinary move created missing parent")
	}
	p, err := PreparePatch(ctx, root, "*** Begin Patch\n*** Update File: source\n*** Move to: "+dst+"\n*** End Patch", protocol.WorktreeID{}, core.ManualFiles, func(path string) string { return path }, observedForPatch(t, root))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("changed\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	out, err := Apply(ctx, root, p)
	if err != nil || out.Data.(protocol.FileChanges).Status != protocol.ChangesStatusRejected {
		t.Fatalf("stale move: %+v %v", out, err)
	}
	if _, err := os.Stat(filepath.Join(root, "new")); !os.IsNotExist(err) {
		t.Fatalf("stale move created parent: %v", err)
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
			p, err := PreparePatch(ctx, root, patch, protocol.NewWorktreeID(), core.RestrictedFiles, func(s string) string { return s }, map[string]string{"a": revision([]byte(tc.before))})
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
