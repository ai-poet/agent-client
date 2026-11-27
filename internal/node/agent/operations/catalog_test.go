package operations

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/plugins"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestInstalledCatalogKeepsPackageIdentity(t *testing.T) {
	t.Parallel()
	source, profile := t.TempDir(), t.TempDir()
	manifest := `{"$schema":"` + plugins.ManifestSchema + `","name":"files","version":"1.0.0","extensions":{"dev.sailry.platform":{"api_version":"v1","scope":"host","actions":["files.read"],"tools":[{"name":"read_file","operation":"files.read"}]}}}`
	if err := os.WriteFile(filepath.Join(source, "plugin.json"), []byte(manifest), 0o600); err != nil {
		t.Fatal(err)
	}
	ctx := t.Context()
	info, err := plugins.NewManager(profile).Install(ctx, source, "", "files")
	if err != nil {
		t.Fatal(err)
	}
	reference := plugins.PackageReference(info)
	reference.SettingsRevision = 8
	catalog, err := CaptureCatalog(ctx, profile, []protocol.PluginReference{reference})
	if err != nil {
		t.Fatal(err)
	}
	if len(catalog.Tools) != 1 || len(catalog.References) != 1 || catalog.References[0] != reference {
		t.Fatal("installed package declaration was not captured", catalog)
	}
	tool := catalog.Tools[0]
	if tool.Owner != reference || tool.Name != Alias("files", "read_file") || tool.Operation != protocol.ToolOperationReadFile {
		t.Fatal("installed tool lost its captured identity", tool)
	}
	if _, _, official := Official(reference, tool.Name); official {
		t.Fatal("an installed package gained built-in trust through its name")
	}
	reference.Digest = "0000000000000000000000000000000000000000000000000000000000000000"
	if _, err := CaptureCatalog(ctx, profile, []protocol.PluginReference{reference}); err == nil {
		t.Fatal("missing package capture fell back to a same-name default")
	}
	if _, err := CaptureResources(ctx, profile, []protocol.PluginReference{reference}); err == nil {
		t.Fatal("missing package resources fell back to a same-name default")
	}
}
