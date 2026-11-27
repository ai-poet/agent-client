package node

import (
	"charm.land/fantasy"
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/engine"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/operations"
	"github.com/ai-poet/cheaprouter-harness/internal/node/plugins"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func trustPackageFixture(t *testing.T, node *Node, name, version, tools string) protocol.PluginInfo {
	t.Helper()
	source := t.TempDir()
	const entry = "dev.sailry.platform/host/main.js"
	if err := os.MkdirAll(filepath.Dir(filepath.Join(source, entry)), 0o700); err != nil {
		t.Fatal(err)
	}
	manifest := `{"$schema":"` + plugins.ManifestSchema + `","name":"` + name + `","version":"` + version + `","extensions":{"dev.sailry.platform":{"api_version":"v1","instructions":"Package instructions.","actions":["browser.read"],"host":{"entry":"` + entry + `","resources":["` + entry + `"],"handlers":["read"]},"tools":[` + tools + `]}}}`
	for path, value := range map[string]string{"plugin.json": manifest, entry: `export function read(input) { return {source: "package", input}; }`} {
		if err := os.WriteFile(filepath.Join(source, path), []byte(value), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	info, err := node.plugins.manager.Install(t.Context(), source, "", name)
	if err != nil || info.Extension == nil || info.Extension.Host == nil || len(info.Issues) != 0 {
		t.Fatalf("fixture %s: %+v, %v", name, info, err)
	}
	return info
}

func recordTrustPackage(t *testing.T, node *Node, ctx context.Context, info protocol.PluginInfo, revision uint64) protocol.PluginInfo {
	t.Helper()
	if err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		info, err = store.InstallPlugin(ctx, db, info, revision, true)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	return info
}

func TestScriptToolBindsOnlyItsCapturedOperation(t *testing.T) {
	t.Parallel()
	node, err := StartWithOptions(t.Context(), filepath.Join(t.TempDir(), "profile"), Options{MCP: fixtureMCPOptions()})
	if err != nil {
		t.Fatal(err)
	}
	defer node.Stop(context.Background())
	info := trustPackageFixture(t, node, "fixture-browser", "0.1.10", `{"name":"snapshot","description":"Package-owned snapshot","handler":{"name":"read","operation":"browser.read","parameters":{"type":"object"}}}`)
	ctx := t.Context()
	info = recordTrustPackage(t, node, ctx, info, 0)
	owner := plugins.PackageReference(info)
	alias := operations.Alias(owner.Name, "snapshot")
	inv := &store.AgentInvocation{Turn: protocol.DefaultQueuedTurn(), Run: protocol.DefaultConversationRun()}
	inv.Turn.Plugins = []protocol.PluginReference{owner}
	for _, row := range []struct {
		name      string
		owner     protocol.PluginReference
		operation protocol.ToolOperation
		want      bool
	}{
		{"another digest", protocol.PluginReference{Name: owner.Name, Digest: strings.Repeat("e", 64)}, browserReadOperation, false},
		{"another operation", owner, browserControlOperation, false},
		{"same captured operation", owner, browserReadOperation, true},
	} {
		t.Run(row.name, func(t *testing.T) {
			native := engine.Tool{Definition: fantasy.FunctionTool{Name: alias, Description: "Fixture adapter", InputSchema: map[string]any{"type": "object"}},
				Binding: &store.ApprovalBinding{Owner: row.owner, Tool: alias, Operation: &row.operation},
				Execute: func(context.Context, canonical.ToolRequest) engine.ToolResult {
					return engine.ToolResult{Value: "fixture"}
				}}
			tools, err := node.pluginTools(ctx, inv, []engine.Tool{native})
			if err != nil {
				t.Fatal(err)
			}
			if !row.want {
				if len(tools) != 0 {
					t.Fatal("a foreign or mismatched adapter was offered")
				}
				return
			}
			if len(tools) != 1 || tools[0].Prepare == nil || tools[0].Binding != native.Binding {
				t.Fatal("the script did not keep its own declared adapter and approval binding")
			}
		})
	}
}
