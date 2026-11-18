package service

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

// TestWorker covers what a pass does around its sources: watching them and
// surviving the ones it cannot read.
func TestWorker(t *testing.T) {
	t.Run("watches request a pass for indexed entries only", func(t *testing.T) {
		fixture := newHarness(t)
		root := t.TempDir()
		write(t, filepath.Join(root, "notes", "first.md"), "first note\n")
		write(t, filepath.Join(root, "build", "output.md"), "generated\n")
		write(t, filepath.Join(root, ".gitignore"), "build/\n")
		fixture.selectEmbedding(false)
		fixture.create("Docs", directory(root))
		requested := make(chan struct{}, 1)
		watches := newWatches(func() {
			select {
			case requested <- struct{}{}:
			default:
			}
		})
		defer watches.close()
		// The first pass registers the scanned directories and asks for a
		// recheck; the second finds the topology settled.
		for range 2 {
			if err := fixture.service.sweep(t.Context(), watches); err != nil {
				t.Fatal(err)
			}
		}
		if watches.needsPoll() {
			t.Fatal("watching fell back to the periodic rescan")
		}
		settle := func() bool {
			select {
			case <-requested:
				return true
			case <-time.After(time.Second):
				return false
			}
		}
		// Changes made while the watches were registered may still arrive.
		for settle() {
		}
		write(t, filepath.Join(root, "build", "more.md"), "generated\n")
		if settle() {
			t.Fatal("an ignored directory requested a pass")
		}
		write(t, filepath.Join(root, "notes", "second.md"), "second note\n")
		select {
		case <-requested:
		case <-time.After(10 * time.Second):
			t.Fatal("an indexed directory did not request a pass")
		}
	})

	t.Run("a source that cannot be read is recorded and the pass continues", func(t *testing.T) {
		fixture := newHarness(t)
		root := t.TempDir()
		write(t, filepath.Join(root, "a.md"), "alpha notes\n")
		write(t, filepath.Join(root, "c.md"), "gamma notes\n")
		write(t, filepath.Join(root, "locked", "inside.md"), "inside notes\n")
		fixture.selectEmbedding(false)
		fixture.create("Docs", directory(root))
		progress := func() protocol.KnowledgeProgress {
			t.Helper()
			fixture.sweep()
			return fixture.collections()[0].Progress
		}
		first := progress()
		if first.Status != protocol.KnowledgeStatusReady || first.Sources != 3 || first.Completed != 3 || first.Failed != 0 {
			t.Fatalf("first pass = %+v", first)
		}
		if found := names(fixture.query("gamma", nil).Hits); !slices.Equal(found, []string{"c.md"}) {
			t.Fatalf("later file = %v", found)
		}

		// A directory that cannot be entered keeps what was published
		// from it and is reported until it can be read.
		locked := filepath.Join(root, "locked")
		if err := os.Chmod(locked, 0); err != nil {
			t.Fatal(err)
		}
		restored := false
		restore := func() {
			if !restored {
				restored = true
				_ = os.Chmod(locked, 0o755)
			}
		}
		t.Cleanup(restore)
		if _, err := os.ReadDir(locked); err == nil {
			t.Log("this user reads directories without permission; the directory case is not covered")
		} else {
			denied := progress()
			if denied.Status != protocol.KnowledgeStatusReady || denied.Failed != 1 || denied.Sources != 3 ||
				!slices.ContainsFunc(denied.Failures, func(failure protocol.KnowledgeFailure) bool {
					return failure.Path == locked && failure.Reason == "knowledge source cannot be read"
				}) {
				t.Fatalf("unreadable directory = %+v", denied)
			}
			if found := names(fixture.query("inside", nil).Hits); !slices.Equal(found, []string{"inside.md"}) {
				t.Fatalf("published source below the directory = %v", found)
			}
		}
		restore()

		// A collection that fails as a whole reports its reason and is
		// retried a bounded number of times.
		if err := os.RemoveAll(root); err != nil {
			t.Fatal(err)
		}
		for pass := 1; pass <= attempts+1; pass++ {
			gone := progress()
			if gone.Status != protocol.KnowledgeStatusPaused || len(gone.Failures) != 1 || gone.Failures[0].Path != "" ||
				!strings.Contains(gone.Failures[0].Reason, "does not exist") {
				t.Fatalf("missing root = %+v", gone)
			}
			if want := pass < attempts; fixture.service.retry != want {
				t.Fatalf("missing root pass %d retry = %v", pass, fixture.service.retry)
			}
		}
	})
}
