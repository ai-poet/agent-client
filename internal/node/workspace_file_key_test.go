package node

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestWorkspaceAliasLookup(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	// Ordering also works for names that do not exist until the patch creates them.
	one := workspaceFileKey(root, "Folder/caf\u00e9")
	if one != workspaceFileKey("", filepath.Join(root, "Folder", "caf\u00e9")) {
		t.Fatal("relative and absolute names split the file queue")
	}
	names := []string{"folder/caf\u00e9", "Folder/Caf\u00e9", "Folder/cafe\u0301", "Folder/other"}
	before := make([]string, len(names))
	for i, name := range names {
		before[i] = workspaceFileKey(root, name)
	}
	if err := os.Mkdir(filepath.Join(root, "Folder"), 0o700); err != nil {
		t.Fatal(err)
	}
	file := filepath.Join(root, "Folder", "caf\u00e9")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	written, err := os.Stat(file)
	if err != nil {
		t.Fatal(err)
	}
	if workspaceFileKey("", file) != one {
		t.Fatal("creating the file changed its queue")
	}
	for i, name := range names {
		found, err := os.Stat(filepath.Join(root, name))
		if err != nil && !os.IsNotExist(err) {
			t.Fatal(err)
		}
		alias := err == nil && os.SameFile(written, found)
		if (before[i] == one) != alias || workspaceFileKey(root, name) != before[i] {
			t.Errorf("%q did not follow the volume's lookup rules before and after creation (alias=%v)", name, alias)
		}
		observed := workspaceRevisions{worktree: root, paths: map[string]string{}}
		observed.observeRevision(observed.pathKey(file), "original")
		patch := rootRevisions(root, []string{name}, observed.paths)
		if (patch[name] == "original") != alias {
			t.Errorf("patch observation for %q does not follow lookup (alias=%v)", name, alias)
		}
		observed.invalidate(observed.pathKey(filepath.Join(root, name)))
		if _, present := observed.paths[one]; present == alias {
			t.Errorf("invalidating %q did not follow lookup (alias=%v)", name, alias)
		}
	}
	if runtime.GOOS != "windows" {
		alias := filepath.Join(t.TempDir(), "alias")
		if err := os.Symlink(root, alias); err != nil {
			t.Fatal(err)
		}
		if one != workspaceFileKey(alias, "Folder/caf\u00e9") {
			t.Fatal("parent alias split the file lock")
		}
	}
}
