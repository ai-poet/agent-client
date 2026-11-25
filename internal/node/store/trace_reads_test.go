package store

import (
	"bytes"
	"compress/gzip"
	"os"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestTracePagesAndBoundedBody(t *testing.T) {
	root := t.TempDir()
	q := protocol.TraceQuery{Session: protocol.NewSessionID(), Limit: 1}
	dir := filepath.Join(root, "traces", q.Session.String())
	if err := os.MkdirAll(filepath.Join(dir, "bodies"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "trace.jsonl"), []byte("{\"attempt\":1}\n{\"attempt\":2}\n"), 0600); err != nil {
		t.Fatal(err)
	}
	first, err := ReadTrace(t.Context(), root, q)
	if err != nil || len(first.Records) != 1 || first.Next == "" {
		t.Fatalf("first: %+v, %v", first, err)
	}
	q.After = first.Next
	second, err := ReadTrace(t.Context(), root, q)
	if err != nil || len(second.Records) != 1 || string(second.Records[0]) != "{\"attempt\":2}" || second.Next != "" {
		t.Fatalf("second: %+v, %v", second, err)
	}
	var packed bytes.Buffer
	z := gzip.NewWriter(&packed)
	if _, err = z.Write(bytes.Repeat([]byte("a"), 300<<10)); err != nil {
		t.Fatal(err)
	}
	if err = z.Close(); err != nil {
		t.Fatal(err)
	}
	q.After, q.Limit, q.Body = "", 0, "bodies/example-1.res.gz"
	if err = os.WriteFile(filepath.Join(dir, filepath.FromSlash(q.Body)), packed.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
	body, err := ReadTrace(t.Context(), root, q)
	if err != nil || !body.Truncated || len(body.Content) != 256<<10 || len(body.Records) != 0 {
		t.Fatalf("body preview bytes=%d truncated=%v: %v", len(body.Content), body.Truncated, err)
	}
	for _, q.Body = range []string{"bodies/../../other/bodies/example-1.res.gz", "bodies/..\\other.req.gz", "trace.jsonl"} {
		if _, err = ReadTrace(t.Context(), root, q); err == nil {
			t.Fatalf("accepted %q", q.Body)
		}
	}
}
