//go:build integration

package git

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Repositories native Git reads and the Git service must read too: sparse and
// linked checkouts that enable extensions, and a checkout another account owns.
func TestNativeRepositoryAccess(t *testing.T) {
	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("Git executable is required")
	}
	ctx := context.Background()
	isolated := func(t *testing.T) []string {
		home := t.TempDir()
		env := []string{}
		for _, pair := range os.Environ() {
			key, _, _ := strings.Cut(pair, "=")
			if key != "HOME" && key != "USERPROFILE" && key != "XDG_CONFIG_HOME" && !strings.HasPrefix(strings.ToUpper(key), "GIT_") {
				env = append(env, pair)
			}
		}
		return append(env, "HOME="+home, "USERPROFILE="+home, "XDG_CONFIG_HOME="+filepath.Join(home, ".config"))
	}
	git := func(t *testing.T, env []string, root string, args ...string) ([]byte, error) {
		t.Helper()
		cmd := exec.Command("git", args...)
		cmd.Dir, cmd.Env = root, env
		return cmd.CombinedOutput()
	}
	t.Run("sparse_and_linked_checkouts", func(t *testing.T) {
		root, _, commit := fixture(t)
		env := isolated(t)
		linked, err := filepath.EvalSymlinks(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		linked = filepath.Join(linked, "linked")
		for _, args := range [][]string{{"sparse-checkout", "init", "--cone"}, {"worktree", "add", "-q", "-b", "linked", linked}} {
			if out, err := git(t, env, root, args...); err != nil {
				t.Fatalf("git %v: %v %s", args, err, out)
			}
		}
		if body, _ := os.ReadFile(filepath.Join(root, ".git", "config")); !strings.Contains(strings.ToLower(string(body)), "worktreeconfig") {
			t.Fatalf("sparse checkout did not enable worktreeConfig:\n%s", body)
		}
		s := New(Options{Environment: env})
		for _, tree := range []string{root, linked} {
			status, err := s.Status(ctx, tree)
			if err != nil || status.Kind != protocol.RepositoryKindReady || status.Head == nil || *status.Head != commit {
				t.Fatalf("status of %s = %+v, %v", tree, status, err)
			}
			if _, err := s.Branches(ctx, tree); err != nil {
				t.Fatalf("branches of %s: %v", tree, err)
			}
		}
	})
	t.Run("another_owner", func(t *testing.T) {
		root, _, _ := fixture(t)
		env := isolated(t)
		// Git's own test switch reports every path as owned by another account.
		owned := append(append([]string(nil), env...), "GIT_TEST_ASSUME_DIFFERENT_OWNER=1")
		if out, err := git(t, owned, root, "status"); err == nil || !strings.Contains(string(out), "dubious ownership") {
			t.Fatalf("Git did not refuse another owner's repository: %v %s", err, out)
		}
		forms := []string{root}
		if runtime.GOOS == "windows" {
			// A root registered with a lowercase drive letter and backslashes.
			forms = append(forms, strings.ToLower(root[:1])+root[1:])
		}
		for _, form := range forms {
			if out, err := git(t, owned, root, "-c", "safe.directory="+safeDirectory(form), "status"); err != nil {
				t.Fatalf("safe.directory=%s for %s was refused: %v %s", safeDirectory(form), form, err, out)
			}
		}
		// The runner drops inherited GIT_ variables, so the switch cannot reach
		// it; the configuration it passes shows the same exemption.
		capture, err := New(Options{Environment: env}).command(ctx, root, []string{"config", "--get-all", "safe.directory"}, nil, true)
		if err != nil || strings.TrimSpace(string(capture.Stdout)) != safeDirectory(root) {
			t.Fatalf("runner safe.directory = %q, %v; want %q", capture.Stdout, err, safeDirectory(root))
		}
	})
}
