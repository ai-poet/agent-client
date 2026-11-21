package main

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/ai-poet/agent-client/internal/client"
	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/preferences"
	"github.com/ai-poet/agent-client/internal/desktop/shell"
	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/migrate"
	"github.com/ai-poet/agent-client/internal/node"
	"github.com/ai-poet/agent-client/internal/node/store"
	"github.com/egoist/mygo/ui"
	_ "modernc.org/sqlite"
)

const testLegacySecretToken = "live-legacy-access-token-must-never-be-copied"

func seedLegacyCheapRouterDir(t *testing.T, legacyDir string) map[string][]byte {
	t.Helper()
	if err := os.MkdirAll(legacyDir, 0o700); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("sqlite", filepath.Join(legacyDir, "app.db"))
	if err != nil {
		t.Fatal(err)
	}
	schema := `
CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT NOT NULL,position INTEGER NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE sessions(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,title TEXT NOT NULL,auto_title TEXT,provider TEXT NOT NULL,model TEXT,status TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,last_reply_at INTEGER);
CREATE TABLE messages(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,turn_id TEXT,position INTEGER NOT NULL,role TEXT NOT NULL,content TEXT NOT NULL,display_content TEXT,attachments TEXT NOT NULL DEFAULT '[]',created_at INTEGER NOT NULL,streaming INTEGER NOT NULL);
CREATE TABLE session_details(session_id TEXT PRIMARY KEY,data TEXT NOT NULL);`
	if _, err := db.Exec(schema); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO projects VALUES('11111111-1111-1111-1111-111111111111','Legacy App','/tmp/legacy-app',0,100)"); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO sessions VALUES('22222222-2222-2222-2222-222222222222','11111111-1111-1111-1111-111111111111','Legacy chat',NULL,'native',NULL,'idle',100,120,120)"); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO messages VALUES('44444444-4444-4444-4444-444444444444','22222222-2222-2222-2222-222222222222','33333333-3333-3333-3333-333333333333',0,'user','hello from main',NULL,'[]',100,0)"); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO session_details VALUES('22222222-2222-2222-2222-222222222222','{}')"); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	appJSON := `{"analytics_enabled":false,"theme":"dark","language":"simplified-chinese","favorite_models":["gpt-5"]}`
	if err := os.WriteFile(filepath.Join(legacyDir, "app.json"), []byte(appJSON), 0o600); err != nil {
		t.Fatal(err)
	}
	creds := `{"access_token":"` + testLegacySecretToken + `","refresh_token":"legacy-refresh-secret","endpoint":"https://cheaprouter.cc/","email":"user@example.com"}`
	if err := os.WriteFile(filepath.Join(legacyDir, "cloud-account.json"), []byte(creds), 0o600); err != nil {
		t.Fatal(err)
	}
	before := map[string][]byte{}
	for _, name := range []string{"app.db", "app.json", "cloud-account.json"} {
		raw, err := os.ReadFile(filepath.Join(legacyDir, name))
		if err != nil {
			t.Fatal(err)
		}
		before[name] = raw
	}
	return before
}

func startTestBackend(t *testing.T, ctx context.Context, profile string) *desktopBackend {
	t.Helper()
	backend := &desktopBackend{Path: profile, Preferences: preferences.ForStartup(profile)}
	var err error
	if backend.Node, err = node.StartWithOptions(ctx, profile, node.Options{}); err != nil {
		t.Fatal(err)
	}
	backend.Migration = newDesktopMigration(backend.Node, profile)
	backend.Client = client.New(link.NewLocal(backend.Node.ID(), backend.Node.ID(), backend.Node))
	if err := backend.Client.Start(ctx); err != nil {
		_ = backend.Node.Stop(context.Background())
		t.Fatal(err)
	}
	return backend
}

func TestFirstLaunchSilentAutoMigrationAndIdempotency(t *testing.T) {
	home := t.TempDir()
	for _, name := range []string{"HOME", "USERPROFILE", "LOCALAPPDATA", "XDG_DATA_HOME"} {
		t.Setenv(name, home)
	}
	t.Setenv("SUB2API_DATA_DIR_NAME", "")
	legacyDir := filepath.Join(home, ".cheaprouter")
	legacyBefore := seedLegacyCheapRouterDir(t, legacyDir)

	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	profile := filepath.Join(t.TempDir(), "profile")

	// First launch: silent auto-migration runs with no confirmation popup.
	backend := startTestBackend(t, ctx, profile)
	runtime, err := liveWindow(ctx, backend, true)
	if err != nil {
		t.Fatal(err)
	}
	tester := ui.NewTesterWith(runtime.App.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	confirmLabel := i18n.In("zh-CN", "migration_confirm")

	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		tester.Frame()
		if runtime.App.Settings.Migration.Open || tester.HasText(confirmLabel) {
			t.Fatalf("first launch showed migration confirmation dialog: open=%v texts=%q", runtime.App.Settings.Migration.Open, tester.Texts())
		}
		state := runtime.App.Settings.Migration
		if state.Known && !state.Checking && !state.Running && state.Status != "" {
			break
		}
		time.Sleep(15 * time.Millisecond)
	}
	state := runtime.App.Settings.Migration
	if state.Status != migrate.Partial && state.Status != migrate.Completed {
		t.Fatalf("expected silent auto-migration to complete, got state=%+v", state)
	}
	if state.Failure != "" || state.Open {
		t.Fatalf("unexpected failure or modal state: %+v", state)
	}

	// Verify legacy directory remained strictly read-only.
	for name, want := range legacyBefore {
		got, err := os.ReadFile(filepath.Join(legacyDir, name))
		if err != nil || !bytes.Equal(got, want) {
			t.Fatalf("legacy file %s changed or unreadable: err=%v", name, err)
		}
	}

	// Verify backup exists and excludes cloud-account.json, and no secret token
	// appears anywhere in the new profile.
	backups, err := filepath.Glob(filepath.Join(profile, "migration-backup-*"))
	if err != nil || len(backups) != 1 {
		t.Fatalf("backups = %v, err=%v; want 1 backup directory", backups, err)
	}
	if _, err := os.Stat(filepath.Join(backups[0], "cloud-account.json")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("backup must not contain cloud-account.json: %v", err)
	}
	if err := filepath.WalkDir(profile, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil || d.IsDir() {
			return walkErr
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		if bytes.Contains(raw, []byte(testLegacySecretToken)) {
			t.Fatalf("profile file %s contains legacy secret token", path)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	// Verify app.json theme, language (simplified-chinese -> chinese -> zh-CN),
	// and telemetry (analytics_enabled: false) migrated cleanly.
	prefs := preferences.Open(filepath.Join(profile, "desktop", "preferences.json"))
	if !prefs.Readable || prefs.Data.Appearance.Mode != "dark" {
		t.Fatalf("migrated preferences = %+v (readable=%v)", prefs.Data, prefs.Readable)
	}
	if prefs.Data.Language == nil || *prefs.Data.Language != "chinese" || preferenceLocale(prefs.Data.Language) != "zh-CN" {
		t.Fatalf("migrated language = %v (locale=%q), want chinese/zh-CN", prefs.Data.Language, preferenceLocale(prefs.Data.Language))
	}

	nodeID := backend.Node.ID()
	markerBytesBefore, err := os.ReadFile(filepath.Join(profile, migrate.MarkerFilename))
	if err != nil {
		t.Fatal(err)
	}
	_ = runtime.Close()
	_ = backend.Close()

	// Verify telemetry setting persisted in node.turso.
	storage, err := store.Open(ctx, profile, nodeID)
	if err != nil {
		t.Fatal(err)
	}
	if err := storage.Node.Read(ctx, func(db *sql.Conn) error {
		telemetry, err := store.ReadTelemetry(ctx, db)
		if err != nil {
			return err
		}
		if telemetry.Errors || telemetry.Metrics || telemetry.Revision != 1 {
			t.Fatalf("telemetry not migrated from app.json: %+v", telemetry)
		}
		return nil
	}); err != nil {
		_ = storage.Close()
		t.Fatal(err)
	}
	_ = storage.Close()

	// Second launch: idempotent (does not re-run migration or create a second backup).
	backend2 := startTestBackend(t, ctx, profile)
	defer backend2.Close()
	runtime2, err := liveWindow(ctx, backend2, true)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime2.Close()
	tester2 := ui.NewTesterWith(runtime2.App.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester2.SetLocale("zh-CN")
	deadline = time.Now().Add(10 * time.Second)
	for time.Now().Before(deadline) {
		tester2.Frame()
		st := runtime2.App.Settings.Migration
		if st.Known && !st.Checking && !st.Running {
			break
		}
		time.Sleep(15 * time.Millisecond)
	}
	backupsAfter, err := filepath.Glob(filepath.Join(profile, "migration-backup-*"))
	if err != nil || len(backupsAfter) != 1 {
		t.Fatalf("repeat launch created extra backup: %v", backupsAfter)
	}
	markerBytesAfter, err := os.ReadFile(filepath.Join(profile, migrate.MarkerFilename))
	if err != nil || !bytes.Equal(markerBytesBefore, markerBytesAfter) {
		t.Fatal("repeat launch rewrote migration.json")
	}
}

func TestFirstLaunchMigrationFailureEntersWorkspaceWithDiagnosticsAndLogs(t *testing.T) {
	home := t.TempDir()
	for _, name := range []string{"HOME", "USERPROFILE", "LOCALAPPDATA", "XDG_DATA_HOME"} {
		t.Setenv(name, home)
	}
	t.Setenv("SUB2API_DATA_DIR_NAME", "")
	legacyDir := filepath.Join(home, ".cheaprouter")
	if err := os.MkdirAll(legacyDir, 0o700); err != nil {
		t.Fatal(err)
	}
	// Valid app.json + corrupted app.db triggers detect/import failure.
	if err := os.WriteFile(filepath.Join(legacyDir, "app.db"), append([]byte("SQLite format 3\x00"), make([]byte, 32)...), 0o600); err != nil {
		t.Fatal(err)
	}

	var logBuf bytes.Buffer
	prevLogger := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&logBuf, nil)))
	t.Cleanup(func() { slog.SetDefault(prevLogger) })

	ctx, cancel := context.WithTimeout(t.Context(), 3*time.Minute)
	defer cancel()
	profile := filepath.Join(t.TempDir(), "profile")

	backend := startTestBackend(t, ctx, profile)
	defer backend.Close()
	runtime, err := liveWindow(ctx, backend, true)
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	tester := ui.NewTesterWith(runtime.App.View, 1280, 820, ui.TesterOptions{Preferences: ui.Preferences{ReduceTransparency: true}})
	tester.SetLocale("zh-CN")
	text := func(key string) string { return i18n.In("zh-CN", key) }

	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		tester.Frame()
		st := runtime.App.Settings.Migration
		if st.Known && !st.Checking && !st.Running && st.Failure != "" {
			break
		}
		time.Sleep(15 * time.Millisecond)
	}
	if runtime.App.Settings.Migration.Open {
		t.Fatal("failed migration blocked startup with a modal dialog")
	}
	if runtime.App.Settings.Migration.Failure != "migration_failed" || runtime.App.Settings.Migration.Status != migrate.Failed {
		t.Fatalf("expected failed migration state, got %+v", runtime.App.Settings.Migration)
	}
	if !strings.Contains(logBuf.String(), "legacy migration") {
		t.Fatalf("expected migration failure log, got %q", logBuf.String())
	}
	marker, readErr := migrate.ReadMarker(profile)
	if readErr != nil || marker == nil || marker.Status != migrate.Failed || marker.Error == "" {
		t.Fatalf("expected failed migration.json marker, got %+v, err=%v", marker, readErr)
	}

	// Main workspace remains accessible despite migration failure.
	if runtime.App.Welcome.Open {
		if err := tester.Click(text("close")); err != nil {
			t.Fatal(err)
		}
		tester.Frame()
	}
	_, composer := tester.Access(text("composer"))
	if runtime.App.Page != shell.ConversationPage || !composer {
		t.Fatalf("main workspace blocked after migration failure: page=%v composer=%v texts=%q", runtime.App.Page, composer, tester.Texts())
	}

	// Settings › General displays failure state and Retry action.
	if err := tester.Click(text("settings")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText(text("migration_failed")) || !tester.HasText(text("migration_failed_retry")) || !tester.HasText(text("migration_retry")) {
		t.Fatalf("Settings › General missing migration failure state: %q", tester.Texts())
	}

	// Settings › CheapRouter Account also displays migration failure state.
	if err := tester.Click(text("cheaprouter_account")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if !tester.HasText(text("migration_retry")) || runtime.App.Settings.CheapRouter.MigrationFailure != "migration_failed" {
		t.Fatalf("CheapRouter account page missing migration failure state: state=%+v texts=%q", runtime.App.Settings.CheapRouter, tester.Texts())
	}
}
