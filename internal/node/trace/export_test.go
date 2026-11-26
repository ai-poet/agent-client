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
	"runtime"
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
	body := compressed(t, []byte(`{"instructions":"private-sentinel","tools":["private-sentinel"],"input":"ordinary user question"}`))
	write("bodies/private-sentinel.req.gz", body)
	resp := compressed(t, []byte("data: {\"text\":\"private-sentinel\",\"error\":\"useful response evidence\"}\n\n"))
	write("bodies/example.res.gz", resp)
	line := func(turn protocol.TurnID, outcome string) string {
		return fmt.Sprintf(`{"v":1,"session":%q,"turn":%q,"request_id":"private-sentinel","attempt":1,"outcome":%q,"request":{"headers":{"User-Agent":["sdk-fixture","private-sentinel"]},"body":"bodies/private-sentinel.req.gz"},"provider":{"model":"diagnostic-model"},"response":{"gateway_request_id":"gateway-42","body":"bodies/example.res.gz"},"extra":"private-sentinel"}`, session.String(), turn.String(), outcome) + "\n"
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
					want := body
					if strings.HasSuffix(f.Name, ".res.gz") {
						want = resp
					}
					if tc.mode == "raw" && !bytes.Equal(content, want) {
						t.Fatal("raw sidecar changed")
					}
					if tc.mode == "shareable" && strings.HasSuffix(f.Name, ".req.gz") && (!bytes.Contains(check, []byte("sha256:")) || !bytes.Contains(check, []byte("ordinary user question"))) {
						t.Fatal("missing body fingerprint")
					}
				}
				if strings.HasSuffix(f.Name, ".res.gz") && !bytes.Contains(check, []byte("useful response evidence")) {
					t.Fatal("response evidence lost")
				}
				if tc.mode == "shareable" && (strings.Contains(f.Name, secret) || bytes.Contains(check, []byte(secret))) {
					t.Fatalf("private content in %s", f.Name)
				}
			}
			var m struct{ Files map[string]string }
			if err = json.Unmarshal(entries["manifest.json"], &m); err != nil {
				t.Fatal(err)
			}
			if len(entries) != 6 || len(m.Files) != 5 {
				t.Fatalf("unexpected inventory %v", m.Files)
			}
			for name, content := range entries {
				if name != "manifest.json" && m.Files[name] != fmt.Sprintf("%x", sha256.Sum256(content)) {
					t.Fatalf("wrong hash for %s", name)
				}
			}
			idx := string(entries["trace.jsonl"])
			for _, fact := range []string{"sdk-fixture", "diagnostic-model", "gateway-42"} {
				if !strings.Contains(idx, fact) {
					t.Fatalf("index lost %s", fact)
				}
			}
			for _, fact := range []string{"diagnostic-model", "gateway-42"} {
				if !bytes.Contains(entries["manifest.json"], []byte(fact)) {
					t.Fatalf("manifest lost %s", fact)
				}
			}
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
	if _, err := inflate(compressed(t, bytes.Repeat([]byte("a"), 17)), 16); err != tooLarge {
		t.Fatal("expanded beyond budget", err)
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

func TestTraceExportThroughSymlinkedProfileParent(t *testing.T) {
	parent := t.TempDir()
	alias := filepath.Join(t.TempDir(), "tmp")
	if err := os.Symlink(parent, alias); err != nil {
		if runtime.GOOS == "windows" {
			t.Skipf("symlink unavailable: %v", err)
		}
		t.Fatal(err)
	}
	q := protocol.TraceExportQuery{Session: protocol.NewSessionID(), Mode: "raw", Scope: "all"}
	dir := filepath.Join(parent, "profile", "traces", q.Session.String())
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "trace.jsonl"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := Export(t.Context(), filepath.Join(alias, "profile"), q, io.Discard); err != nil {
		t.Fatal(err)
	}
}

func TestTraceExportRejectsSymlinkedSession(t *testing.T) {
	profile := t.TempDir()
	q := protocol.TraceExportQuery{Session: protocol.NewSessionID(), Mode: "raw", Scope: "all"}
	traces := filepath.Join(profile, "traces")
	if err := os.MkdirAll(traces, 0700); err != nil {
		t.Fatal(err)
	}
	other := t.TempDir()
	if err := os.WriteFile(filepath.Join(other, "trace.jsonl"), nil, 0600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(other, filepath.Join(traces, q.Session.String())); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	var out bytes.Buffer
	if err := Export(t.Context(), profile, q, &out); err == nil || out.Len() != 0 {
		t.Fatal("followed link outside authorized session")
	}
}

func TestTraceExportSelectNarrowerScopeAt64MiB(t *testing.T) {
	root := t.TempDir()
	query := protocol.TraceExportQuery{Session: protocol.NewSessionID(), Scope: "all", Mode: "raw"}
	dir := filepath.Join(root, "traces", query.Session.String())
	if err := os.MkdirAll(filepath.Join(dir, "bodies"), 0700); err != nil {
		t.Fatal(err)
	}
	body := compressed(t, bytes.Repeat([]byte("x"), 16<<20))
	var index bytes.Buffer
	turn := protocol.NewTurnID()
	for i := range 5 {
		name := fmt.Sprintf("bodies/r%d.req.gz", i)
		data := body
		outcome := "failed"
		if i == 4 {
			data = compressed(t, []byte("x"))
			outcome = "succeeded"
		}
		if err := os.WriteFile(filepath.Join(dir, name), data, 0600); err != nil {
			t.Fatal(err)
		}
		fmt.Fprintf(&index, `{"v":1,"session":%q,"turn":%q,"request_id":"r%d","attempt":1,"outcome":%q,"request":{"body":%q}}`+"\n", query.Session.String(), turn.String(), i, outcome, name)
	}
	if err := os.WriteFile(filepath.Join(dir, "trace.jsonl"), index.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
	if err := Export(t.Context(), root, query, io.Discard); err != tooLarge {
		t.Fatal("64 MiB plus one was not rejected", err)
	}
	query.Scope = "failed"
	if err := Export(t.Context(), root, query, io.Discard); err != nil {
		t.Fatal("selected 64 MiB should fit", err)
	}
}

func TestTraceExportPromptShapesAndStreamEcho(t *testing.T) {
	const secret = "private-sentinel"
	for _, body := range []string{
		`{"system":[{"type":"text","text":"private-sentinel"}],"messages":[{"role":"user","content":"keep this"}]}`,
		`{"instructions":"private-sentinel","input":"keep this"}`,
		`{"input":[{"role":"developer","content":[{"type":"input_text","text":"private-sentinel"}]},{"role":"user","content":"keep this"}]}`,
		`{"messages":[{"role":"system","content":"private-sentinel"},{"role":"user","content":"keep this"}]}`,
		`{"systemInstruction":{"parts":[{"text":"private-sentinel"}]},"contents":[{"parts":[{"text":"keep this"}]}]}`,
		`{"system_instruction":{"parts":[{"text":"private-sentinel"}]},"contents":[{"parts":[{"text":"keep this"}]}]}`,
	} {
		p := promptSet{}
		b, known, err := p.request([]byte(body))
		if err != nil || !known {
			t.Fatalf("unsupported fixture: %v", err)
		}
		s := p.replacer()
		safe := string(safeEvidence(b, s))
		if strings.Contains(safe, secret) || !strings.Contains(safe, digest([]byte(secret))) || !strings.Contains(safe, "keep this") {
			t.Fatal("incorrect prompt projection", safe)
		}
		resp := string(safeEvidence([]byte("event: delta\ndata: {\"text\":\"prefix private-\"}\n\ndata: {\"text\":\"sentinel suffix\",\"count\":3}\n\n"), s))
		for _, v := range []string{"private-", "sentinel"} {
			if strings.Contains(resp, v) {
				t.Fatal("split prompt echoed", resp)
			}
		}
		for _, v := range []string{"prefix", "suffix", "count", "event: delta"} {
			if !strings.Contains(resp, v) {
				t.Fatal("lost unrelated evidence", resp)
			}
		}
	}
}

func TestTraceExportUnknownRequestAndResponse(t *testing.T) {
	profile := t.TempDir()
	q := protocol.TraceExportQuery{Session: protocol.NewSessionID(), Mode: "shareable", Scope: "all"}
	dir := filepath.Join(profile, "traces", q.Session.String())
	if err := os.MkdirAll(filepath.Join(dir, "bodies"), 0700); err != nil {
		t.Fatal(err)
	}
	for _, suffix := range []string{"req", "res"} {
		if err := os.WriteFile(filepath.Join(dir, "bodies", "unknown."+suffix+".gz"), compressed(t, []byte("opaque private data")), 0600); err != nil {
			t.Fatal(err)
		}
	}
	line := fmt.Sprintf(`{"v":1,"session":%q,"request":{"body":"bodies/unknown.req.gz"},"response":{"body":"bodies/unknown.res.gz"}}`, q.Session.String())
	if err := os.WriteFile(filepath.Join(dir, "trace.jsonl"), []byte(line), 0600); err != nil {
		t.Fatal(err)
	}
	var out bytes.Buffer
	if err := Export(t.Context(), profile, q, &out); err != nil {
		t.Fatal(err)
	}
	archive, err := zip.NewReader(bytes.NewReader(out.Bytes()), int64(out.Len()))
	if err != nil {
		t.Fatal(err)
	}
	checked := 0
	for _, f := range archive.File {
		if !strings.HasPrefix(f.Name, "bodies/") {
			continue
		}
		r, err := f.Open()
		if err != nil {
			t.Fatal(err)
		}
		g, err := gzip.NewReader(r)
		if err != nil {
			t.Fatal(err)
		}
		data, err := io.ReadAll(g)
		g.Close()
		r.Close()
		if err != nil || string(data) != marker("body omitted", []byte("opaque private data")) {
			t.Fatal("unsafe fallback", string(data), err)
		}
		checked++
	}
	if checked != 2 {
		t.Fatal("missing fallback sidecars", checked)
	}
}
