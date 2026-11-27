//go:build integration && (darwin || linux)

package process

import (
	"context"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ai-poet/cheaprouter-harness/internal/node/git"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

func TestNativeCommandLifecycle(t *testing.T) {
	ctx := context.Background()
	service, err := New(Options{Profile: t.TempDir(), Environment: NewEnvironment([]string{"PATH=/usr/bin:/bin", "HOME=" + t.TempDir()}), CaptureManaged: git.New(git.Options{}).CaptureManaged})
	if err != nil {
		t.Fatal(err)
	}
	defer service.Close()
	root := t.TempDir()
	launch := Launch{ID: protocol.NewRequestID(), Session: protocol.NewSessionID(), Turn: protocol.NewTurnID(), Root: root, Command: "printf stdout; printf stderr >&2; exit 7", Access: protocol.ProcessAccessUnrestricted, TimeoutMs: 10000}
	result, err := service.Execute(ctx, launch)
	if err != nil || result.Outcome.Kind != "exited" || result.Stdout.Text != "stdout" || result.Stderr.Text != "stderr" {
		t.Fatal(result, err)
	}
	code, ok := result.Outcome.Data.(int32)
	if !ok || code != 7 {
		t.Fatal(result.Outcome)
	}
	launch.ID, launch.Command, launch.Pty = protocol.NewRequestID(), "read value; printf 'received:%s' \"$value\"", true
	info, err := service.Start(launch)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = service.Write(info.Session, info.ID, "fixture\n", nil); err != nil {
		t.Fatal(err)
	}
	if _, err = service.Wait(ctx, info.Session, info.ID, 30000, "received:fixture"); err != nil {
		t.Fatal(err)
	}
	snapshot, err := service.Read(info.Session, info.ID)
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.Stdout.Text == "" {
		t.Fatal("native PTY returned no output")
	}
	launch.ID, launch.Pty, launch.Command = protocol.NewRequestID(), false, "printf inside > allowed; printf outside > "+shellQuote(filepath.Join(t.TempDir(), "denied"))+" 2>/dev/null; test $? -ne 0"
	launch.Access = protocol.ProcessAccessWorkspace
	result, err = service.Execute(ctx, launch)
	if err != nil || result.Outcome.Kind != "exited" {
		t.Fatal(result, err)
	}
	code, ok = result.Outcome.Data.(int32)
	if !ok || code != 0 {
		t.Fatal("sandbox allowed outside write", result)
	}
	if value, err := os.ReadFile(filepath.Join(root, "allowed")); err != nil || string(value) != "inside" {
		t.Fatal(string(value), err)
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	launch.ID, launch.Access = protocol.NewRequestID(), protocol.ProcessAccessUnrestricted
	launch.Command = fmt.Sprintf("nc -z -w 1 127.0.0.1 %d", listener.Addr().(*net.TCPAddr).Port)
	result, err = service.Execute(ctx, launch)
	if err != nil || result.Outcome.Kind != "exited" {
		t.Fatal("network fixture baseline failed", result, err)
	}
	if code, ok := result.Outcome.Data.(int32); !ok || code != 0 {
		t.Fatal("network fixture baseline did not connect", result)
	}
	launch.ID, launch.Access = protocol.NewRequestID(), protocol.ProcessAccessWorkspace
	result, err = service.Execute(ctx, launch)
	if err != nil || result.Outcome.Kind != "exited" {
		t.Fatal(result, err)
	}
	if code, ok := result.Outcome.Data.(int32); !ok || code == 0 {
		t.Fatal("sandbox allowed network access", result)
	}
}

func TestNativeSandboxDefaultReadAndCredentialDeny(t *testing.T) {
	ctx := context.Background()
	home, profile, external, root := t.TempDir(), t.TempDir(), t.TempDir(), t.TempDir()
	var credentialChecks strings.Builder
	for _, relative := range credentialFixtures {
		path := filepath.Join(home, relative)
		if err := os.MkdirAll(filepath.Dir(path), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte("credential-fixture"), 0600); err != nil {
			t.Fatal(err)
		}
		fmt.Fprintf(&credentialChecks, "test -z \"$(cat %s 2>/dev/null || :)\" || exit 12\n", shellQuote(path))
	}
	profileSecret := filepath.Join(profile, "node.db")
	if err := os.WriteFile(profileSecret, []byte("SAILRY-PROFILE-SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}
	externalAsset := filepath.Join(external, "cache.txt")
	if err := os.WriteFile(externalAsset, []byte("shared-cache-ok"), 0o644); err != nil {
		t.Fatal(err)
	}
	gitConfig := filepath.Join(root, ".git", "config")
	if err := os.MkdirAll(filepath.Dir(gitConfig), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(gitConfig, []byte("[core]\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	service, err := New(Options{Profile: profile, Environment: NewEnvironment([]string{"PATH=/usr/bin:/bin", "HOME=" + home}), CaptureManaged: git.New(git.Options{}).CaptureManaged})
	if err != nil {
		t.Fatal(err)
	}
	defer service.Close()

	cmd := fmt.Sprintf(`set -eu
# External non-sensitive paths are readable (default-allow read) but never writable.
test "$(cat %s)" = "shared-cache-ok"
if printf hacked > %s 2>/dev/null; then exit 11; fi
# User credentials and Sailry profile data are unreadable.
%s
if cat %s >/dev/null 2>&1; then exit 13; fi
# Existing .git metadata is protected from writes.
if printf bad >> %s 2>/dev/null; then exit 14; fi
# Workspace and TMPDIR remain writable.
printf ok > "$TMPDIR/scratch.txt"
test "$(cat "$TMPDIR/scratch.txt")" = "ok"
printf ok > work.txt
`, shellQuote(externalAsset), shellQuote(filepath.Join(external, "denied.txt")), credentialChecks.String(), shellQuote(profileSecret), shellQuote(gitConfig))

	result, err := service.Execute(ctx, Launch{
		ID: protocol.NewRequestID(), Session: protocol.NewSessionID(), Turn: protocol.NewTurnID(),
		Root: root, Command: cmd, Access: protocol.ProcessAccessWorkspace, TimeoutMs: 10000,
	})
	if err != nil || result.Outcome.Kind != "exited" {
		t.Fatalf("sandbox security check failed: %+v, %v", result, err)
	}
	if code, ok := result.Outcome.Data.(int32); !ok || code != 0 {
		t.Fatalf("sandbox security check exited %v: stdout=%q stderr=%q", result.Outcome.Data, result.Stdout.Text, result.Stderr.Text)
	}
}

func TestNativeSandboxReadOnlyDeniesWorkspaceWrites(t *testing.T) {
	ctx := context.Background()
	home, profile, root := t.TempDir(), t.TempDir(), t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "readme.txt"), []byte("hello-readonly"), 0o644); err != nil {
		t.Fatal(err)
	}
	service, err := New(Options{Profile: profile, Environment: NewEnvironment([]string{"PATH=/usr/bin:/bin", "HOME=" + home}), CaptureManaged: git.New(git.Options{}).CaptureManaged})
	if err != nil {
		t.Fatal(err)
	}
	defer service.Close()

	cmd := `set -eu
test "$(cat readme.txt)" = "hello-readonly"
printf scratch-ok > "$TMPDIR/scratch.txt"
test "$(cat "$TMPDIR/scratch.txt")" = "scratch-ok"
if printf mut > mutated.txt 2>/dev/null; then exit 21; fi
if printf mut >> readme.txt 2>/dev/null; then exit 22; fi
`
	result, err := service.Execute(ctx, Launch{
		ID: protocol.NewRequestID(), Session: protocol.NewSessionID(), Turn: protocol.NewTurnID(),
		Root: root, Command: cmd, Access: protocol.ProcessAccessWorkspace, ReadOnly: true, TimeoutMs: 10000,
	})
	if err != nil || result.Outcome.Kind != "exited" {
		t.Fatalf("read-only sandbox check failed: %+v, %v", result, err)
	}
	if code, ok := result.Outcome.Data.(int32); !ok || code != 0 {
		t.Fatalf("read-only sandbox check exited %v: stdout=%q stderr=%q", result.Outcome.Data, result.Stdout.Text, result.Stderr.Text)
	}
	if _, err := os.Stat(filepath.Join(root, "mutated.txt")); !os.IsNotExist(err) {
		t.Fatalf("read-only sandbox allowed creating mutated.txt: %v", err)
	}
}

func TestNativeSandboxLateXDGGitCredential(t *testing.T) {
	home, config, root := t.TempDir(), t.TempDir(), t.TempDir()
	env := NewEnvironment([]string{"HOME=" + home, "XDG_CONFIG_HOME=" + config, "PATH=/usr/bin:/bin"})
	path := filepath.Join(config, "git", "credentials")
	sibling := filepath.Join(config, "tools")
	if err := os.WriteFile(sibling, []byte("offline"), 0600); err != nil {
		t.Fatal(err)
	}
	launch := Launch{Root: root, Profile: t.TempDir(), Access: protocol.ProcessAccessWorkspace,
		Command: fmt.Sprintf(`set -eu; test "$(cat %s)" = offline; test -z "$(cat %s 2>/dev/null || :)"`, shellQuote(sibling), shellQuote(path))}
	cmd, release, err := prepareCommand(launch, env, root, "", git.New(git.Options{}).CaptureManaged)
	if err != nil {
		t.Fatal(err)
	}
	defer release()
	// Publish the credential after the mount arguments have been captured.
	if err := os.Mkdir(filepath.Dir(path), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("credential-fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("late Git credential check: %v: %s", err, output)
	}
}

func shellQuote(value string) string { return "'" + strings.ReplaceAll(value, "'", "'\\''") + "'" }
