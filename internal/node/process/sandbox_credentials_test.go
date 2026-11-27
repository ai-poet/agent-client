//go:build linux || darwin

package process

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
)

// Use actual files below credential directories as well as credential files.
// Keep this independent of the policy: dropping an entry must fail the tests.
var credentialFixtures = []string{
	".ssh/key", ".gnupg/key", ".aws/credentials", ".azure/token", ".kube/config",
	".docker/config.json", ".gitconfig", ".git-credentials", ".npmrc", ".netrc",
	".pypirc", ".cargo/credentials", ".cargo/credentials.toml", ".config/gh/hosts.yml",
	".config/gcloud/key", ".config/op/key",
}

func TestSandboxDeniedHomeCredentials(t *testing.T) {
	home, root := t.TempDir(), t.TempDir()
	home, err := filepath.EvalSymlinks(home)
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range credentialFixtures {
		path := filepath.Join(home, name)
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte("fixture"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	policy, release, err := prepareSandbox(Launch{Root: root, Profile: t.TempDir()}, NewEnvironment([]string{"HOME=" + home}), func(string) (string, error) { return "", nil })
	if err != nil {
		t.Fatal(err)
	}
	defer release()
	for _, name := range credentialFixtures {
		path := filepath.Join(home, name)
		if !slices.ContainsFunc(policy.deniedReads, func(deny string) bool {
			return path == deny || strings.HasPrefix(path, deny+string(filepath.Separator))
		}) {
			t.Errorf("missing read deny for %s", name)
		}
	}
}
