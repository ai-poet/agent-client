package appresources

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadUserResource(t *testing.T) {
	directory := t.TempDir()
	if err := os.MkdirAll(filepath.Join(directory, "providers"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, "providers", "fixture.json"), []byte("fixture"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("SAILRY_RESOURCES_DIR", directory)
	raw, err := Read("providers/fixture.json")
	if err != nil || string(raw) != "fixture" {
		t.Fatalf("Read() = %q, %v", raw, err)
	}
}

func TestReadRejectsTraversal(t *testing.T) {
	if _, err := Read("../go.mod"); err == nil {
		t.Fatal("resource traversal was accepted")
	}
}
