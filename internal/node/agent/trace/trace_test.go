package trace

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func readFile(t *testing.T, path string) []byte {
	t.Helper()
	p, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return p
}
func readGZ(t *testing.T, path string) string {
	t.Helper()
	r, err := gzip.NewReader(bytes.NewReader(readFile(t, path)))
	if err != nil {
		t.Fatal(err)
	}
	b, err := io.ReadAll(r)
	if err != nil {
		t.Fatal(err)
	}
	if err = r.Close(); err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestTraceAttemptsWriteWireBytesAndDedupeRequest(t *testing.T) {
	s := Scope{Profile: t.TempDir(), Session: "s", Turn: "t", Purpose: "main", Provider: Provider{Credential: "uuid"}}
	dir := filepath.Join(s.Profile, "traces", "s")
	req := `{ "system" : "stay", "token" : "hidden", "number":1e+03 }`
	sse := "event: answer\r\ndata: {\"x\": \"echoed\", \"n\":2.00}\r\n\r\ndata: [DONE]"
	for i := range 2 {
		a := s.Begin("r", i, "echoed")
		a.Request(http.Header{"Cookie": {"private-cookie"}, "Authorization": {"private-auth"}, "User-Agent": {"echoed"}}, []byte(req))
		resp := &http.Response{Body: io.NopCloser(strings.NewReader(sse)), Header: http.Header{"Content-Type": {"text/event-stream"}, "Set-Cookie": {"private-set"}}, StatusCode: 200, Uncompressed: true}
		a.Response(resp)
		received, err := io.ReadAll(resp.Body)
		if err != nil || string(received) != sse {
			t.Fatalf("wire: %q %v", received, err)
		}
		if err = resp.Body.Close(); err != nil {
			t.Fatal(err)
		}
		a.Finish(Facts{Outcome: "ok", GatewayRequestID: "gw", Usage: map[string]int{"cache_write": 3}, Failure: map[string]string{"password": "private-failure"}})
		a.Finish(Facts{})
	}
	rows := readFile(t, filepath.Join(dir, "trace.jsonl"))
	if bytes.Contains(rows, []byte("private-")) || bytes.Contains(rows, []byte("echoed")) {
		t.Fatal("metadata contains a secret")
	}
	lines := bytes.Split(bytes.TrimSpace(rows), []byte("\n"))
	if len(lines) != 2 {
		t.Fatalf("rows: %d", len(lines))
	}
	var first rowKey
	for i, b := range lines {
		var row rowKey
		if err := json.Unmarshal(b, &row); err != nil {
			t.Fatal(err)
		}
		if row.RequestID != "r" || row.Request.Body == "" || !strings.HasPrefix(row.Digest["res"], "sha256:") {
			t.Fatalf("row: %s", b)
		}
		if i == 0 {
			first = row
		} else if row.Request.Body != first.Request.Body {
			t.Fatal("retry duplicated request")
		}
	}
	paths, err := os.ReadDir(filepath.Join(dir, "bodies"))
	if err != nil || len(paths) != 3 {
		t.Fatalf("bodies: %v %v", paths, err)
	}
	if got := readGZ(t, filepath.Join(dir, first.Request.Body)); got != strings.Replace(req, "hidden", mask, 1) {
		t.Fatalf("request: %q", got)
	}
	if got := readGZ(t, filepath.Join(dir, "bodies", "r-1.res.gz")); got != strings.Replace(sse, "echoed", mask, 1) {
		t.Fatalf("response: %q", got)
	}
	if runtime.GOOS != "windows" {
		for p, mode := range map[string]os.FileMode{"": 0700, "bodies": 0700, "trace.jsonl": 0600, first.Request.Body: 0600} {
			i, err := os.Stat(filepath.Join(dir, p))
			if err != nil || i.Mode().Perm() != mode {
				t.Fatalf("%s: %v %v", p, i, err)
			}
		}
	}
}

func TestTraceRedactionPreservesSurroundingBytes(t *testing.T) {
	wire := "event: message\r\ndata: {\"a\": [ {\"pass\\u0077ord\":\r\ndata:   {\"bad\": 1}}, \"Bearer abc\", \"a\\u0062c\"], \"b\":1e-1}\r\n\r\n"
	want := "event: message\r\ndata: {\"a\": [ {\"pass\\u0077ord\":\r\ndata:   \"[REDACTED]\"}, \"[REDACTED]\", \"[REDACTED]\"], \"b\":1e-1}\r\n\r\n"
	got, ok := redactSSE([]byte(wire), []string{"abc"})
	if !ok || string(got) != want {
		t.Fatalf("%q", got)
	}
	for _, x := range []string{"sk-123", "ghp_456", "AKIA789", "-----BEGIN PRIVATE KEY-----\nprivate\n-----END PRIVATE KEY-----", "-----BEGIN CERTIFICATE-----\ncertificate\n-----END CERTIFICATE-----"} {
		got, ok := payload([]byte(x), nil)
		if !ok || string(got) != mask {
			t.Errorf("%q => %q", x, got)
		}
	}
	if _, ok := payload([]byte(`{"token":"unfinished`), nil); ok {
		t.Fatal("partial structured secret captured")
	}
}

func TestTraceDisableDuringRequest(t *testing.T) {
	enabled := true
	s := Scope{Profile: t.TempDir(), Session: "s", Enabled: func() bool { return enabled }}
	a := s.Begin("r", 1)
	a.Request(nil, []byte("body"))
	enabled = false
	a.Finish(Facts{})
	if s.Begin("r", 2) != nil {
		t.Fatal("retry admitted after disabling collection")
	}
	if _, err := os.Stat(filepath.Join(s.Profile, "traces")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("capture was published: %v", err)
	}
}

func TestTraceClearInvalidatesOnlyAdmittedSession(t *testing.T) {
	s := Scope{Profile: t.TempDir(), Session: "old"}
	old := s.Begin("a", 0)
	s.Session = "other"
	other := s.Begin("b", 0)
	if err := Clear(s.Profile, "old"); err != nil {
		t.Fatal(err)
	}
	old.Finish(Facts{})
	other.Finish(Facts{})
	if _, err := os.Stat(filepath.Join(s.Profile, "traces", "old")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	readFile(t, filepath.Join(s.Profile, "traces", "other", "trace.jsonl"))
	s.Session = "old"
	s.Begin("new", 0).Finish(Facts{})
	if err := Clear(s.Profile, "old"); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(s.Profile, "traces", "old")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
}

func TestTraceFailOpenAndConfined(t *testing.T) {
	s := Scope{Profile: t.TempDir(), Session: "s"}
	for _, id := range []string{"", "..", "a/b", `a\b`, "../x", "C:foo"} {
		if s.Begin(id, 0) != nil || Clear(s.Profile, id) == nil {
			t.Errorf("accepted %q", id)
		}
	}
	target := t.TempDir()
	if err := os.Symlink(target, filepath.Join(s.Profile, "traces")); err != nil {
		t.Skip(err)
	}
	a := s.Begin("r", 0)
	r := &http.Response{Body: io.NopCloser(strings.NewReader("original"))}
	a.Response(r)
	got, err := io.ReadAll(r.Body)
	if err != nil || string(got) != "original" {
		t.Fatal(string(got), err)
	}
	a.Finish(Facts{})
	if Clear(s.Profile, s.Session) == nil {
		t.Fatal("clear followed root symlink")
	}
	content, err := os.ReadDir(target)
	if err != nil || len(content) != 0 {
		t.Fatal(content, err)
	}
}

func TestTraceLimitsIncludeIndexAndSharedBodies(t *testing.T) {
	s := Scope{Profile: t.TempDir(), Session: "s"}
	dir := filepath.Join(s.Profile, "traces", "s")
	const budget int64 = 2500
	for i := range 10 {
		a := Attempt{scope: s, request: "r", index: i}
		a.Request(nil, []byte("one"))
		a.publish(Facts{}, a.prepare(&a.req, false), nil, budget, budget, 2)
	}
	sizes, err := diskUsage(filepath.Dir(dir), dir)
	if err != nil || sizes.total > budget {
		t.Fatalf("budget: %v %v", sizes, err)
	}
	lines := bytes.Count(readFile(t, filepath.Join(dir, "trace.jsonl")), []byte("\n"))
	if lines != 2 {
		t.Fatalf("round count %d", lines)
	}
	s.Session = "small"
	a := Attempt{scope: s, request: "r"}
	a.Request(nil, []byte("one"))
	a.publish(Facts{}, a.prepare(&a.req, false), nil, 20, budget, 2)
	if b := readFile(t, filepath.Join(s.Profile, "traces", "small", "trace.jsonl")); len(b) != 0 {
		t.Fatal("metadata exceeded limit")
	}
	s.Session = "big"
	big := s.Begin("r", 0)
	big.Request(nil, make([]byte, maxBody+1))
	big.Finish(Facts{})
	b := readFile(t, filepath.Join(s.Profile, "traces", "big", "trace.jsonl"))
	if !bytes.Contains(b, []byte(`"req_truncated":true`)) {
		t.Fatal(string(b))
	}
}

func TestTraceDiskCeilings(t *testing.T) {
	for _, size := range []int64{64 << 20, 512 << 20} {
		s := Scope{Profile: t.TempDir(), Session: "active"}
		base := filepath.Join(s.Profile, "traces")
		usedBy := "active"
		if size == 512<<20 {
			usedBy = "other"
		}
		dir := filepath.Join(base, usedBy)
		if err := os.MkdirAll(dir, 0700); err != nil {
			t.Fatal(err)
		}
		f, err := os.Create(filepath.Join(dir, "prior"))
		if err != nil {
			t.Fatal(err)
		}
		// A sparse file exercises the real restart accounting without allocating the profile.
		if err = f.Truncate(size - 2048); err != nil {
			t.Fatal(err)
		}
		f.Close()
		a := s.Begin("req", 1)
		// Incompressible printable data leaves room for the row but not its gzip body.
		var content []byte
		state := uint32(1)
		for range 8192 {
			state = state*1664525 + 1013904223
			content = append(content, 'a'+byte((state>>16)%26))
		}
		a.Request(nil, content)
		a.Finish(Facts{})
		row := readFile(t, filepath.Join(base, "active", "trace.jsonl"))
		if size == maxProfile {
			if _, err := os.Stat(dir); !errors.Is(err, os.ErrNotExist) || !bytes.Contains(row, []byte(`"req_truncated":false`)) {
				t.Fatalf("global eviction did not preserve new body: %s (%v)", row, err)
			}
		} else if !bytes.Contains(row, []byte(`"req_truncated":true`)) {
			t.Fatalf("cap %d: %s", size, row)
		}
		used, err := diskUsage(base, filepath.Join(base, "active"))
		if err != nil || used.total > size || used.session > 64<<20 {
			t.Fatalf("cap %d: %+v %v", size, used, err)
		}
	}
}

func TestTraceBodyWriteFailureStillIndexesAttempt(t *testing.T) {
	s := Scope{Profile: t.TempDir(), Session: "s"}
	dir := filepath.Join(s.Profile, "traces", "s")
	path := filepath.Join(dir, "bodies", "r-1.req.gz")
	if err := os.MkdirAll(path, 0700); err != nil {
		t.Fatal(err)
	}
	a := s.Begin("r", 1)
	a.Request(nil, []byte("{}"))
	a.Finish(Facts{Outcome: "failed"})
	row := readFile(t, filepath.Join(dir, "trace.jsonl"))
	if !bytes.Contains(row, []byte(`"req_truncated":true`)) || bytes.Contains(row, []byte(`.req.gz`)) {
		t.Fatalf("missing failed sidecar accounting: %s", row)
	}
}

type sizes struct{ total, session int64 }

func diskUsage(base, session string) (s sizes, err error) {
	err = filepath.WalkDir(base, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		i, err := d.Info()
		if err != nil {
			return err
		}
		s.total += i.Size()
		if strings.HasPrefix(path, session+string(filepath.Separator)) {
			s.session += i.Size()
		}
		return nil
	})
	return
}
