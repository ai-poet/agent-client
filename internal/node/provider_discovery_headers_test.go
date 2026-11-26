package node

import (
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/logging"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestDiscoverModelsHeadersRemainPrivate(t *testing.T) {
	t.Setenv("SAILRY_TELEMETRY", "off")
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal("resolving the discovery fixture directory failed")
	}
	profile := filepath.Join(root, "profile")
	closeLog, err := logging.Install(profile, "host")
	if err != nil {
		t.Fatal("installing the discovery fixture log failed")
	}
	t.Cleanup(closeLog)
	node, err := StartWithOptions(t.Context(), profile, Options{MCP: fixtureMCPOptions()})
	if err != nil {
		t.Fatal("starting the discovery fixture Node failed")
	}
	t.Cleanup(func() { _ = node.Stop(context.Background()) })
	const marker = "private-discovery-fixture"
	var reached atomic.Bool
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		reached.Store(request.Header.Get("X-Tenant-Id") == marker)
		writer.WriteHeader(http.StatusServiceUnavailable)
		_, _ = writer.Write([]byte(marker))
	}))
	defer server.Close()
	secret := protocol.NewSecret("fixture-key")
	request := protocol.NewRequest(node.ID(), protocol.Command{Kind: "discover_models", Data: protocol.ConversationDiscoverySource{Kind: "draft", Data: protocol.ConversationDiscoveryDraft{
		Provider: protocol.NewProviderID(), Api: protocol.ConversationModelApiResponses, Endpoint: server.URL, Secret: &secret, Headers: map[string]string{"X-Tenant-Id": marker},
	}}})
	// Discovery is a read-only command on this fork and need not emit a
	// report. Inspect every diagnostic that does get emitted.
	var disclosed atomic.Bool
	logging.SetReport(func(_ slog.Level, fields logging.Fields) {
		for _, values := range []logging.Fields{fields, logging.Remote(fields)} {
			encoded, err := json.Marshal(values)
			if err != nil || strings.Contains(string(encoded), marker) {
				disclosed.Store(true)
			}
		}
	})
	t.Cleanup(func() { logging.SetReport(nil) })
	// The local log is created lazily. Start it before the read-only request
	// so the file inspected below exists even when discovery logs nothing.
	logging.Emit(slog.LevelInfo, slog.String("operation", "discovery_fixture"))
	fault := dispatchFixture(t, node, node.ID(), request).Fault
	if !reached.Load() || fault == nil || fault.Code != protocol.ErrorCodeUnavailable {
		t.Fatal("the gateway failure did not retain its safe discovery outcome")
	}
	encoded, err := json.Marshal(fault)
	if err != nil || strings.Contains(string(encoded), marker) {
		t.Fatal("the discovery fault disclosed a private header value")
	}
	if disclosed.Load() {
		t.Fatal("discovery diagnostics disclosed a private header value")
	}
	path, err := logging.CurrentFile()
	if err != nil || path == "" {
		t.Fatal("locating the discovery fixture log failed")
	}
	data, err := os.ReadFile(path)
	if err != nil || strings.Contains(string(data), marker) {
		t.Fatal("the local discovery log disclosed a private header value")
	}
}
