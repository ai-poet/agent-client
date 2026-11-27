package git

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/go-git/go-git/v5/config"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestOpenRepositoryExtensions(t *testing.T) {
	configure := func(t *testing.T, version string, extensions ...string) string {
		t.Helper()
		root, _, _ := fixture(t)
		path := filepath.Join(root, ".git", "config")
		body, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		text := strings.Replace(string(body), "repositoryformatversion = 0", "repositoryformatversion = "+version, 1)
		text += "[extensions]\n"
		for _, extension := range extensions {
			text += "\t" + extension + "\n"
		}
		if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
			t.Fatal(err)
		}
		return root
	}
	for _, tc := range []struct {
		name, version string
		extensions    []string
	}{
		{"sparse checkout", "0", []string{"worktreeConfig = true"}},
		{"worktree config", "1", []string{"worktreeConfig = true"}},
		{"filtered clone", "1", []string{"partialClone = origin"}},
		{"defaults", "1", []string{"objectFormat = sha1", "refStorage = files", "preciousObjects = true"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := configure(t, tc.version, tc.extensions...)
			r, err := open(root)
			if err != nil {
				t.Fatalf("open: %v", err)
			}
			if h, err := head(r); err != nil || h.Branch == nil || *h.Branch != "main" {
				t.Fatalf("head = %+v, %v", h, err)
			}
			if _, err := gitDir(root, r); err != nil {
				t.Fatalf("the opened repository lost its file storage: %v", err)
			}
			// Branch rename and deletion write the configuration through
			// go-git; the extensions must survive that write.
			settings, err := r.Config()
			if err != nil {
				t.Fatal(err)
			}
			settings.Branches["topic"] = &config.Branch{Name: "topic", Remote: "origin", Merge: "refs/heads/topic"}
			if err := r.Storer.SetConfig(settings); err != nil {
				t.Fatal(err)
			}
			written, err := os.ReadFile(filepath.Join(root, ".git", "config"))
			if err != nil {
				t.Fatal(err)
			}
			for _, extension := range tc.extensions {
				key, value, _ := strings.Cut(extension, " = ")
				if !strings.Contains(string(written), "[extensions]") || !strings.Contains(string(written), key+" = "+value) {
					t.Fatalf("configuration write dropped %s:\n%s", extension, written)
				}
			}
			if !strings.Contains(string(written), `[branch "topic"]`) {
				t.Fatalf("configuration write was not stored:\n%s", written)
			}
		})
	}
	for _, extension := range []string{"refStorage = reftable", "objectFormat = sha256", "futureExtension = true"} {
		t.Run("unsupported "+extension, func(t *testing.T) {
			root := configure(t, "1", "worktreeConfig = true", extension)
			before, err := os.ReadFile(filepath.Join(root, ".git", "config"))
			if err != nil {
				t.Fatal(err)
			}
			_, err = open(root)
			var value *protocol.Fault
			if !errors.As(err, &value) || value.Code != protocol.ErrorCodeInvalidRequest || value.Message != UnsupportedExtension {
				t.Fatalf("open = %v, want %q", err, UnsupportedExtension)
			}
			if after, _ := os.ReadFile(filepath.Join(root, ".git", "config")); string(after) != string(before) {
				t.Fatal("a refused open changed the configuration")
			}
		})
	}
}
