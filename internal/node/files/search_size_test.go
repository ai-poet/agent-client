package files

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Exercise Search, not just its scanner: a single maximum-size line must
// match at its tail without allocating storage proportional to the file.
func TestLiteralSearchAtFileLimit(t *testing.T) {
	root, files := filesFixture(t)
	f, err := os.Create(filepath.Join(root, "limit.txt"))
	if err != nil {
		t.Fatal(err)
	}
	chunk := bytes.Repeat([]byte("x"), boundedReadChunkSize)
	for end := len(chunk); end <= searchFileByteLimit; end += len(chunk) {
		if end == searchFileByteLimit {
			copy(chunk[len(chunk)-6:], "needle")
		}
		if _, err := f.Write(chunk); err != nil {
			f.Close()
			t.Fatal(err)
		}
	}
	if err := f.Close(); err != nil {
		t.Fatal(err)
	}
	var before, after runtime.MemStats
	runtime.ReadMemStats(&before)
	got, err := Search(t.Context(), files, protocol.FileSearch{Path: ".", Query: "needle", Mode: protocol.SearchModeContent, CaseSensitive: true, Limit: 1})
	runtime.ReadMemStats(&after)
	if err != nil || len(got.Matches) != 1 || !got.Matches[0].Matched || !got.Matches[0].Truncated || len(got.Matches[0].Line) != searchLineByteLimit {
		t.Fatalf("maximum-length file: matches=%d, skipped=%d, error=%v", len(got.Matches), got.Skipped, err)
	}
	t.Logf("64 MiB literal search allocated %d bytes (process-wide)", after.TotalAlloc-before.TotalAlloc)
}
