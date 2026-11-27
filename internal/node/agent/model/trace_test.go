package model

import (
	"bytes"
	"compress/gzip"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"charm.land/fantasy"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/trace"
)

func TestTraceSDKStream(t *testing.T) {
	for _, kind := range []string{"anthropic", "responses", "chat"} {
		t.Run(kind, func(t *testing.T) {
			wire := fixture(t, kind+"-text.sse")
			if kind == "responses" {
				// The decoder receives its synthetic separator, but the sidecar must not.
				wire = bytes.TrimRight(wire, "\n")
			}
			var sent []byte
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				sent, _ = io.ReadAll(r.Body)
				w.Header().Set("Content-Type", "text/event-stream")
				w.Header().Set("Content-Encoding", "gzip")
				w.Header().Set("X-Request-Id", "test-request")
				z := gzip.NewWriter(w)
				z.Write(wire)
				z.Close()
			}))
			defer server.Close()
			lm, err := newSDK(t.Context(), transportConfig{Kind: kind, Model: "fixture-model", Key: "test-credential", Endpoint: server.URL})
			if err != nil {
				t.Fatal(err)
			}
			profile := t.TempDir()
			call := requestCall(t, kind)
			if call.ProviderOptions == nil {
				call.ProviderOptions = fantasy.ProviderOptions{}
			}
			for _, mode := range []string{"disabled", "disk-error", "enabled"} {
				t.Run(mode, func(t *testing.T) {
					scope := &trace.Scope{Profile: profile, Session: "fixture-session", Turn: "fixture-turn", Purpose: "main"}
					if mode != "disabled" {
						call.ProviderOptions[trace.ProviderKey] = scope
					}
					root := filepath.Join(profile, "traces")
					if mode == "disk-error" {
						if err := os.WriteFile(root, []byte("not a directory"), 0600); err != nil {
							t.Fatal(err)
						}
						defer os.Remove(root)
					}
					var output strings.Builder
					var observed Attempt
					err := Run(t.Context(), lm, call, Main, &policyClock{}, func(a Attempt) error { observed = a; return nil }, func(p fantasy.StreamPart) bool {
						if p.Type == fantasy.StreamPartTypeTextDelta {
							output.WriteString(p.Delta)
						}
						return true
					}, nil)
					if err != nil || output.String() != "Read complete" || observed.Usage.Output == nil || *observed.Usage.Output != 3 || observed.GatewayRequestID == nil || *observed.GatewayRequestID != "test-request" {
						t.Fatalf("SDK result: output=%q attempt=%+v err=%v", output.String(), observed, err)
					}
					if mode == "disabled" {
						if observed.Verdict == nil || observed.Verdict.Code != "normal" {
							t.Fatalf("missing metadata-only diagnosis: %+v", observed.Verdict)
						}
						if _, err := os.Stat(root); !errors.Is(err, os.ErrNotExist) {
							t.Fatalf("disabled trace path: %v", err)
						}
					}
					if mode != "enabled" {
						return
					}
					dir := filepath.Join(root, scope.Session)
					line, err := os.ReadFile(filepath.Join(dir, "trace.jsonl"))
					if err != nil {
						t.Fatal(err)
					}
					var row struct {
						RequestID string `json:"request_id"`
						Attempt   uint64
						Request   struct{ Body string }
						Response  struct{ Body, End, Encoding string }
						Usage     struct{ Output uint64 }
					}
					if err := json.Unmarshal(line, &row); err != nil {
						t.Fatal(err)
					}
					if row.RequestID != observed.RequestID || row.Attempt != observed.Index+1 || row.Usage.Output != *observed.Usage.Output || row.Response.End != "completed" || row.Response.Encoding != "identity(decompressed)" {
						t.Fatalf("index: %s", line)
					}
					for path, want := range map[string][]byte{row.Request.Body: sent, row.Response.Body: wire} {
						f, err := os.Open(filepath.Join(dir, path))
						if err != nil {
							t.Fatal(err)
						}
						defer f.Close()
						z, err := gzip.NewReader(f)
						if err != nil {
							t.Fatal(err)
						}
						defer z.Close()
						got, err := io.ReadAll(z)
						if err != nil || !bytes.Equal(got, want) {
							t.Fatalf("sidecar %q differs from wire: %v (%d/%d bytes)", path, err, len(got), len(want))
						}
					}
				})
			}
		})
	}
}

func TestTraceFailedSDKAttempt(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, `{"error":{"message":"bad request"}}`, http.StatusBadRequest)
	}))
	defer server.Close()
	lm, err := newSDK(t.Context(), transportConfig{Kind: "chat", Model: "fixture", Endpoint: server.URL})
	if err != nil {
		t.Fatal(err)
	}
	s := &trace.Scope{Profile: t.TempDir(), Session: "session", Purpose: "main"}
	call := requestCall(t, "chat")
	call.ProviderOptions = fantasy.ProviderOptions{trace.ProviderKey: s}
	var a Attempt
	if err := Run(t.Context(), lm, call, Main, &policyClock{}, func(v Attempt) error { a = v; return nil }, nil, nil); err == nil {
		t.Fatal("expected model error")
	}
	raw, err := os.ReadFile(filepath.Join(s.Profile, "traces", s.Session, "trace.jsonl"))
	if err != nil {
		t.Fatal(err)
	}
	// A rejected route is resent once, so the trace holds one line per attempt;
	// the first line is the one this test inspects.
	if line, _, _ := strings.Cut(string(raw), "\n"); line != "" {
		raw = []byte(line)
	}
	var row struct {
		RequestID string `json:"request_id"`
		Outcome   string
		Status    int
		Response  struct{ Body, End string }
	}
	if err := json.Unmarshal(raw, &row); err != nil || row.RequestID != a.RequestID || row.Status != 400 || row.Outcome != "failed" || row.Response.End != "http-error" || row.Response.Body == "" {
		t.Fatalf("failed observation: %s %v", raw, err)
	}
}
