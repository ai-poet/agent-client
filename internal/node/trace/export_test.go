package trace

import (
	"archive/zip"
	"bytes"
	"compress/gzip"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func compressed(t *testing.T, b []byte) []byte {
	t.Helper()
	var out bytes.Buffer
	g := gzip.NewWriter(&out)
	if _, err := g.Write(b); err != nil {
		t.Fatal(err)
	}
	if err := g.Close(); err != nil {
		t.Fatal(err)
	}
	return out.Bytes()
}
func TestTraceExportSelectionsAndRedaction(t *testing.T) {
	p := t.TempDir()
	session := protocol.NewSessionID()
	a, b := protocol.NewTurnID(), protocol.NewTurnID()
	dir := filepath.Join(p, "traces", session.String())
	if err := os.MkdirAll(filepath.Join(dir, "bodies"), 0700); err != nil {
		t.Fatal(err)
	}
	write := func(name string, data []byte) {
		t.Helper()
		if err := os.WriteFile(filepath.Join(dir, name), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	const secret = "private-sentinel"
	body := compressed(t, []byte(`{"instructions":"private-sentinel","tools":["private-sentinel"]}`))
	write("bodies/private-sentinel.req.gz", body)
	line := func(turn protocol.TurnID, outcome string) string {
		return fmt.Sprintf(`{"v":1,"session":%q,"turn":%q,"request_id":"private-sentinel","attempt":1,"outcome":%q,"request":{"headers":{"User-Agent":["private-sentinel"]},"body":"bodies/private-sentinel.req.gz"},"provider":{"model":"private-sentinel"},"response":{"gateway_request_id":"private-sentinel"},"extra":"private-sentinel"}`, session.String(), turn.String(), outcome) + "\n"
	}
	write("trace.jsonl", []byte(line(a, "succeeded")+line(b, "failed")))
	for _, tc := range []struct {
		scope, mode string
		turn        protocol.TurnID
		rows        int
	}{{"all", "raw", protocol.TurnID{}, 2}, {"all", "shareable", protocol.TurnID{}, 2}, {"failed", "shareable", protocol.TurnID{}, 1}, {"turn", "raw", a, 1}} {
		t.Run(tc.scope+"-"+tc.mode, func(t *testing.T) {
			var data bytes.Buffer
			err := Export(t.Context(), p, protocol.TraceExportQuery{Session: session, Scope: tc.scope, Mode: tc.mode, Turn: tc.turn}, &data)
			if err != nil {
				t.Fatal(err)
			}
			z, err := zip.NewReader(bytes.NewReader(data.Bytes()), int64(data.Len()))
			if err != nil {
				t.Fatal(err)
			}
			entries := map[string][]byte{}
			for _, f := range z.File {
				r, err := f.Open()
				if err != nil {
					t.Fatal(err)
				}
				content, err := io.ReadAll(r)
				r.Close()
				if err != nil {
					t.Fatal(err)
				}
				entries[f.Name] = content
				check := content
				if strings.HasPrefix(f.Name, "bodies/") {
					g, err := gzip.NewReader(bytes.NewReader(content))
					if err != nil {
						t.Fatal(err)
					}
					check, err = io.ReadAll(g)
					g.Close()
					if err != nil {
						t.Fatal(err)
					}
					if tc.mode == "raw" && !bytes.Equal(content, body) {
						t.Fatal("raw sidecar changed")
					}
					if tc.mode == "shareable" && !bytes.Contains(check, []byte("sha256:")) {
						t.Fatal("missing body fingerprint")
					}
				}
				if tc.mode == "shareable" && (strings.Contains(f.Name, secret) || bytes.Contains(check, []byte(secret))) {
					t.Fatalf("private content in %s", f.Name)
				}
			}
			var m struct{ Files map[string]string }
			if err = json.Unmarshal(entries["manifest.json"], &m); err != nil {
				t.Fatal(err)
			}
			if len(entries) != 5 || len(m.Files) != 4 {
				t.Fatalf("unexpected inventory %v", m.Files)
			}
			for name, content := range entries {
				if name != "manifest.json" && m.Files[name] != fmt.Sprintf("%x", sha256.Sum256(content)) {
					t.Fatalf("wrong hash for %s", name)
				}
			}
			idx := string(entries["trace.jsonl"])
			if strings.Count(idx, "\n") != tc.rows || tc.scope == "turn" && strings.Contains(idx, b.String()) || tc.scope == "failed" && strings.Contains(idx, a.String()) {
				t.Fatal("incorrect selection", idx)
			}
			if tc.scope == "failed" && !strings.Contains(string(entries["summary.md"]), b.String()+", attempt 1") {
				t.Fatal("missing failure summary")
			}
		})
	}
}
func TestTraceExportLimits(t *testing.T) {
	for _, mode := range []string{"shareable", "raw"} {
		if _, _, err := exportBody(compressed(t, bytes.Repeat([]byte("a"), 17)), mode, 16, false); err != tooLarge {
			t.Fatalf("%s expanded beyond budget: %v", mode, err)
		}
	}
	w := limitedWriter{io.Discard, 4}
	if _, err := w.Write([]byte("1234")); err != nil {
		t.Fatal(err)
	}
	if _, err := w.Write([]byte("5")); err != tooLarge {
		t.Fatal("oversized archive accepted")
	}
	p := t.TempDir()
	q := protocol.TraceExportQuery{Session: protocol.NewSessionID(), Mode: "raw", Scope: "all"}
	dir := filepath.Join(p, "traces", q.Session.String())
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	f, err := os.Create(filepath.Join(dir, "trace.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	if err = f.Truncate(protocol.MaxTraceExportBytes + 1); err != nil {
		t.Fatal(err)
	}
	f.Close()
	if err = Export(t.Context(), p, q, io.Discard); err != tooLarge {
		t.Fatal("oversized index accepted", err)
	}
}
