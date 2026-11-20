package migrate

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/preferences"
	"github.com/ai-poet/agent-client/desktop/internal/node/store"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

const (
	testProject = "11111111-1111-1111-1111-111111111111"
	testSession = "22222222-2222-2222-2222-222222222222"
	testTurn    = "33333333-3333-3333-3333-333333333333"
	testUser    = "44444444-4444-4444-4444-444444444444"
	testReply   = "55555555-5555-5555-5555-555555555555"
	testLater   = "66666666-6666-6666-6666-666666666666"
	testToken   = "legacy-access-token-plaintext-must-not-be-copied"
)

func TestRunMigratesBacksUpAndSkipsCompleted(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, legacy)
	before := readFiles(t, legacy)
	var stages []Progress
	result, err := Run(context.Background(), legacy, profile, Options{Progress: func(p Progress) {
		stages = append(stages, p)
	}})
	if err != nil {
		t.Fatalf("first migration: %v", err)
	}
	if result.Status != Partial {
		t.Fatalf("unmapped legacy columns must be reported as partial: %+v", result)
	}
	if result.Counts != (Counts{Projects: 1, Sessions: 1, Messages: 2}) {
		t.Fatalf("inserted counts: %+v", result.Counts)
	}
	marker := readJSON[Marker](t, result.MarkerPath)
	if marker.Version != Version || marker.Source != legacy || marker.Counts != result.Counts || marker.MigratedAt.IsZero() {
		t.Fatalf("marker: %+v", marker)
	}
	report := readJSON[Report](t, result.ReportPath)
	if len(report.Unmapped) == 0 || !report.Credentials.Migrated || !report.Credentials.ReloginRequired || report.Credentials.SecretsMigrated {
		t.Fatalf("report: %+v", report)
	}
	if report.Credentials.AccountID != "legacy-login-identity" || report.Credentials.Endpoint != "https://legacy.example.com" {
		t.Fatalf("account metadata: %+v", report.Credentials)
	}
	assertBackupEqual(t, legacy, result.BackupDir)
	assertHistory(t, profile)
	assertNoToken(t, profile, result)

	preferencesPath := filepath.Join(profile, "desktop", "preferences.json")
	prefs := preferences.Open(preferencesPath)
	if prefs.Data.Appearance.Mode != "dark" || prefs.Data.Language == nil || *prefs.Data.Language != "chinese" {
		t.Fatalf("Desktop preferences: %+v", prefs.Data)
	}
	if !slices.ContainsFunc(stages, func(p Progress) bool { return p.Stage == "messages" && p.Processed == 2 }) {
		t.Fatalf("progress: %+v", stages)
	}
	if after := readFiles(t, legacy); !mapsEqual(before, after) {
		t.Fatal("legacy source files changed")
	}

	markerBefore := readBytes(t, result.MarkerPath)
	reportBefore := readBytes(t, result.ReportPath)
	second, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatalf("second migration: %v", err)
	}
	if second.Status != AlreadyDone {
		t.Fatalf("second status: %+v", second)
	}
	if !bytes.Equal(markerBefore, readBytes(t, result.MarkerPath)) || !bytes.Equal(reportBefore, readBytes(t, result.ReportPath)) {
		t.Fatal("AlreadyDone rewrote migration documents")
	}
	assertTargetCounts(t, profile, Counts{Projects: 1, Sessions: 1, Messages: 2})
}

func TestRunFillMissingOnlyAddsAbsentRows(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, legacy)
	first, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatal(err)
	}
	assertTargetCounts(t, profile, Counts{Projects: 1, Sessions: 1, Messages: 2})
	legacyDB, err := sql.Open("sqlite", filepath.Join(legacy, "app.db"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := legacyDB.Exec("INSERT INTO messages VALUES(?1,?2,?3,2,'assistant','added later',NULL,'[]',102,0)", testLater, testSession, testTurn); err != nil {
		t.Fatal(err)
	}
	if err := legacyDB.Close(); err != nil {
		t.Fatal(err)
	}
	storage := openTarget(t, profile)
	if err := storage.Node.Write(context.Background(), func(db *sql.Conn) error {
		_, err := db.ExecContext(context.Background(), "UPDATE projects SET name='New name' WHERE id=?1", testProject)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	storage.Close()
	second, err := Run(context.Background(), legacy, profile, Options{FillMissing: true})
	if err != nil {
		t.Fatal(err)
	}
	if second.Status == AlreadyDone || second.Counts.Messages != 1 || second.Skipped.Messages != 2 {
		t.Fatalf("fill missing result: %+v", second)
	}
	assertTargetCounts(t, profile, Counts{Projects: 1, Sessions: 1, Messages: 3})
	storage = openTarget(t, profile)
	defer storage.Close()
	if err := storage.Node.Read(context.Background(), func(db *sql.Conn) error {
		var name string
		if err := db.QueryRowContext(context.Background(), "SELECT name FROM projects WHERE id=?1", testProject).Scan(&name); err != nil {
			return err
		}
		if name != "New name" {
			t.Fatalf("fill missing overwrote project: %q", name)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if first.BackupDir == second.BackupDir {
		t.Fatal("fill missing did not create a new backup")
	}
}

func TestRunSupportsSplitReleaseDatabase(t *testing.T) {
	legacy := t.TempDir()
	databaseRoot := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, databaseRoot)
	result, err := Run(context.Background(), legacy, profile, Options{LegacyDatabasePath: filepath.Join(databaseRoot, "app.db")})
	if err != nil {
		t.Fatal(err)
	}
	if result.Counts != (Counts{Projects: 1, Sessions: 1, Messages: 2}) {
		t.Fatalf("split Release counts: %+v", result.Counts)
	}
	marker := readJSON[Marker](t, result.MarkerPath)
	if marker.Source != legacy || marker.DatabasePath != filepath.Join(databaseRoot, "app.db") {
		t.Fatalf("split Release marker: %+v", marker)
	}
	if !bytes.Equal(readBytes(t, filepath.Join(databaseRoot, "app.db")), readBytes(t, filepath.Join(result.BackupDir, "app.db"))) {
		t.Fatal("split Release database backup differs")
	}
}

func TestDetectMissingDatabaseReturnsEmptyInventory(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "settings.json"), []byte(`{"theme":"light"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := Detect(root)
	if err != nil {
		t.Fatal(err)
	}
	if state.HasDatabase || state.Counts != (Counts{}) || !state.HasSettings {
		t.Fatalf("state: %+v", state)
	}
	missing, err := Detect(filepath.Join(root, "absent"))
	if err != nil || !missing.Empty() {
		t.Fatalf("missing root: %+v, %v", missing, err)
	}
}

func TestTruncatedDatabaseRecordsFailureMarkerLogsAndAllowsRetry(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	if err := os.WriteFile(filepath.Join(legacy, "app.db"), append([]byte("SQLite format 3\x00"), make([]byte, 32)...), 0o600); err != nil {
		t.Fatal(err)
	}
	var logBuf bytes.Buffer
	prevLogger := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&logBuf, nil)))
	t.Cleanup(func() { slog.SetDefault(prevLogger) })

	result, err := Run(context.Background(), legacy, profile, Options{})
	if err == nil || result != nil {
		t.Fatalf("truncated database: result=%+v err=%v", result, err)
	}
	var structured *Error
	if !errors.As(err, &structured) || structured.Stage == "" {
		t.Fatalf("structured error: %T %v", err, err)
	}
	if !strings.Contains(logBuf.String(), "legacy migration failed") {
		t.Fatalf("expected failure log, got %q", logBuf.String())
	}
	marker, readErr := ReadMarker(profile)
	if readErr != nil || marker == nil {
		t.Fatalf("failed migration did not write diagnostic marker: %+v, %v", marker, readErr)
	}
	if marker.Status != Failed || marker.Stage == "" || marker.Error == "" || marker.MigratedAt.IsZero() {
		t.Fatalf("diagnostic failure marker = %+v", marker)
	}

	// Replacing the broken database with a valid fixture allows retry without
	// being blocked by the failed marker.
	if err := os.Remove(filepath.Join(legacy, "app.db")); err != nil {
		t.Fatal(err)
	}
	createLegacyFixture(t, legacy)
	retried, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil || retried == nil || retried.Status == Failed || retried.Status == AlreadyDone {
		t.Fatalf("retry after failure = %+v, %v", retried, err)
	}
	recovered, err := ReadMarker(profile)
	if err != nil || recovered == nil || recovered.Status == Failed {
		t.Fatalf("recovered marker = %+v, %v", recovered, err)
	}
	third, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil || third.Status != AlreadyDone {
		t.Fatalf("post-recovery rerun = %+v, %v", third, err)
	}
}

func TestAppJSONMigrationTelemetryThemeAndSimplifiedChinese(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, legacy)
	// Simulate current main layout: settings.json belongs to the daemon without
	// desktop keys (or has older values), while app.json holds user choices.
	if err := os.WriteFile(filepath.Join(legacy, "settings.json"), []byte(`{"theme":"light","language":"english","analytics_enabled":true}`), 0o600); err != nil {
		t.Fatal(err)
	}
	appJSON := `{"analytics_enabled":false,"theme":"dark","language":"simplified-chinese","favorite_models":["claude-sonnet"]}`
	if err := os.WriteFile(filepath.Join(legacy, "app.json"), []byte(appJSON), 0o600); err != nil {
		t.Fatal(err)
	}

	state, err := Detect(legacy)
	if err != nil || !state.HasAppSettings {
		t.Fatalf("Detect app.json = %+v, %v", state, err)
	}
	result, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if !bytes.Equal(readBytes(t, filepath.Join(legacy, "app.json")), readBytes(t, filepath.Join(result.BackupDir, "app.json"))) {
		t.Fatal("app.json was not backed up")
	}
	if _, statErr := os.Stat(filepath.Join(result.BackupDir, legacyCredentialName)); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("backup directory must not copy %s: %v", legacyCredentialName, statErr)
	}

	prefs := preferences.Open(filepath.Join(profile, "desktop", "preferences.json"))
	if !prefs.Readable || prefs.Err != nil {
		t.Fatalf("preferences not readable after app.json migration: %v", prefs.Err)
	}
	if prefs.Data.Appearance.Mode != "dark" {
		t.Fatalf("theme = %q, want dark", prefs.Data.Appearance.Mode)
	}
	if prefs.Data.Language == nil || *prefs.Data.Language != "chinese" {
		t.Fatalf("language = %v, want chinese (mapped from simplified-chinese)", prefs.Data.Language)
	}
	pref, locale, ok := NormalizeLegacyLanguage("simplified-chinese")
	if !ok || pref != "chinese" || locale != "zh-CN" {
		t.Fatalf("NormalizeLegacyLanguage(simplified-chinese) = (%q, %q, %v)", pref, locale, ok)
	}

	storage := openTarget(t, profile)
	defer storage.Close()
	if err := storage.Node.Read(context.Background(), func(db *sql.Conn) error {
		telemetry, err := store.ReadTelemetry(context.Background(), db)
		if err != nil {
			return err
		}
		if telemetry.Errors || telemetry.Metrics || telemetry.Revision != 1 {
			t.Fatalf("telemetry = %+v, want Errors=false Metrics=false Revision=1", telemetry)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func TestAppJSONRejectsIllegalLanguageValue(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	if err := os.WriteFile(filepath.Join(legacy, "app.json"), []byte(`{"analytics_enabled":false,"theme":"light","language":"not-a-valid-lang"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	result, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatal(err)
	}
	prefs := preferences.Open(filepath.Join(profile, "desktop", "preferences.json"))
	if !prefs.Readable || prefs.Data.Language != nil || prefs.Data.Appearance.Mode != "light" {
		t.Fatalf("illegal language was not rejected cleanly: readable=%v data=%+v", prefs.Readable, prefs.Data)
	}
	report := readJSON[Report](t, result.ReportPath)
	if !slices.ContainsFunc(report.Unmapped, func(f UnmappedField) bool { return f.Entity == "settings" && f.Field == "language" }) {
		t.Fatalf("invalid language not recorded in unmapped: %+v", report.Unmapped)
	}
}

func TestInterruptedMigrationRecoversWithoutDuplicates(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, legacy)
	first, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatal(err)
	}
	// Simulate crash before migration.json marker was written while backup and
	// imported DB rows already landed.
	if err := os.Remove(first.MarkerPath); err != nil {
		t.Fatal(err)
	}
	recovered, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatalf("recovery rerun: %v", err)
	}
	if recovered.Status == AlreadyDone || recovered.Counts != (Counts{}) || recovered.Skipped != (Counts{Projects: 1, Sessions: 1, Messages: 2}) {
		t.Fatalf("recovery counts = %+v", recovered)
	}
	assertTargetCounts(t, profile, Counts{Projects: 1, Sessions: 1, Messages: 2})
	third, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil || third.Status != AlreadyDone {
		t.Fatalf("idempotent third run = %+v, %v", third, err)
	}
}

func TestCredentialSecretsStayOutOfReportAndTarget(t *testing.T) {
	legacy := t.TempDir()
	profile := t.TempDir()
	createLegacyFixture(t, legacy)
	result, err := Run(context.Background(), legacy, profile, Options{})
	if err != nil {
		t.Fatal(err)
	}
	assertNoToken(t, profile, result)
	account := readBytes(t, filepath.Join(profile, AccountFilename))
	for _, secret := range []string{testToken, "legacy-refresh-token-plaintext", "legacy-derived-api-key"} {
		if bytes.Contains(account, []byte(secret)) {
			t.Fatalf("account metadata contains %q", secret)
		}
	}
}

func createLegacyFixture(t *testing.T, root string) {
	t.Helper()
	db, err := sql.Open("sqlite", filepath.Join(root, "app.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	schema := `
CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT NOT NULL,position INTEGER NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE sessions(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,title TEXT NOT NULL,auto_title TEXT,provider TEXT NOT NULL,model TEXT,status TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,last_reply_at INTEGER);
CREATE TABLE messages(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,turn_id TEXT,position INTEGER NOT NULL,role TEXT NOT NULL,content TEXT NOT NULL,display_content TEXT,attachments TEXT NOT NULL DEFAULT '[]',created_at INTEGER NOT NULL,streaming INTEGER NOT NULL);
CREATE TABLE session_details(session_id TEXT PRIMARY KEY,data TEXT NOT NULL);`
	if _, err := db.Exec(schema); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO projects VALUES(?1,'Legacy project','/tmp/legacy-project',0,100)", testProject); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec("INSERT INTO sessions VALUES(?1,?2,'New task','Imported title','native',NULL,'idle',100,120,120)", testSession, testProject); err != nil {
		t.Fatal(err)
	}
	for _, row := range []struct {
		id, role, content string
		position          int
	}{{testUser, "user", "hello", 0}, {testReply, "assistant", "world", 1}} {
		if _, err := db.Exec("INSERT INTO messages VALUES(?1,?2,?3,?4,?5,?6,NULL,'[]',?7,0)", row.id, testSession, testTurn, row.position, row.role, row.content, 100+row.position); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := db.Exec("INSERT INTO session_details VALUES(?1,'{}')", testSession); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	settings := `{"theme":"dark","language":"chinese","analytics_enabled":false}`
	if err := os.WriteFile(filepath.Join(root, "settings.json"), []byte(settings), 0o600); err != nil {
		t.Fatal(err)
	}
	credentials := `{"access_token":"` + testToken + `","refresh_token":"legacy-refresh-token-plaintext","api_key":"legacy-derived-api-key","session_id":"legacy-login-identity","endpoint":"https://legacy.example.com/"}`
	if err := os.WriteFile(filepath.Join(root, "cloud-account.json"), []byte(credentials), 0o600); err != nil {
		t.Fatal(err)
	}
}

func assertBackupEqual(t *testing.T, legacy, backup string) {
	t.Helper()
	for _, name := range []string{"app.db", "settings.json"} {
		if !bytes.Equal(readBytes(t, filepath.Join(legacy, name)), readBytes(t, filepath.Join(backup, name))) {
			t.Fatalf("backup differs: %s", name)
		}
	}
	if _, err := os.Stat(filepath.Join(backup, legacyCredentialName)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("backup directory must not contain %s: %v", legacyCredentialName, err)
	}
}

func assertHistory(t *testing.T, profile string) {
	t.Helper()
	storage := openTarget(t, profile)
	defer storage.Close()
	if err := storage.Node.Read(context.Background(), func(db *sql.Conn) error {
		history, err := store.ReadConversation(context.Background(), db, protocol.SessionID(mappedUUIDMust("session", testSession)), nil, 100)
		if err != nil {
			return err
		}
		if len(history.Page.Entries) != 2 || len(history.Page.Runs) != 1 {
			t.Fatalf("history entries/runs: %+v", history.Page)
		}
		ids := []string{history.Page.Entries[0].ID, history.Page.Entries[1].ID}
		if !slices.Equal(ids, []string{testUser, testReply}) {
			t.Fatalf("history IDs: %v", ids)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func assertNoToken(t *testing.T, profile string, result *Result) {
	t.Helper()
	if _, err := os.Stat(filepath.Join(result.BackupDir, legacyCredentialName)); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("backup directory contains %s: %v", legacyCredentialName, err)
	}
	err := filepath.WalkDir(profile, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil || d.IsDir() {
			return walkErr
		}
		raw := readBytes(t, path)
		for _, secret := range []string{testToken, "legacy-refresh-token-plaintext", "legacy-derived-api-key"} {
			if bytes.Contains(raw, []byte(secret)) {
				t.Fatalf("%s contains credential secret %q", path, secret)
			}
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}

func assertTargetCounts(t *testing.T, profile string, want Counts) {
	t.Helper()
	storage := openTarget(t, profile)
	defer storage.Close()
	if err := storage.Node.Read(context.Background(), func(db *sql.Conn) error {
		var got Counts
		for table, target := range map[string]*int{"projects": &got.Projects, "sessions": &got.Sessions, "agent_events": &got.Messages} {
			if err := db.QueryRowContext(context.Background(), "SELECT COUNT(*) FROM "+table).Scan(target); err != nil {
				return err
			}
		}
		if got != want {
			t.Fatalf("target counts got %+v want %+v", got, want)
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

func openTarget(t *testing.T, profile string) *store.Store {
	t.Helper()
	storage, err := store.Open(context.Background(), profile, protocol.NodeID{1})
	if err != nil {
		t.Fatal(err)
	}
	return storage
}

func readJSON[T any](t *testing.T, path string) T {
	t.Helper()
	var value T
	if err := json.Unmarshal(readBytes(t, path), &value); err != nil {
		t.Fatal(err)
	}
	return value
}

func readBytes(t *testing.T, path string) []byte {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func readFiles(t *testing.T, root string) map[string][]byte {
	t.Helper()
	values := map[string][]byte{}
	for _, name := range []string{"app.db", "settings.json", "cloud-account.json"} {
		values[name] = readBytes(t, filepath.Join(root, name))
	}
	return values
}

func mapsEqual(a, b map[string][]byte) bool {
	if len(a) != len(b) {
		return false
	}
	for key, value := range a {
		if !bytes.Equal(value, b[key]) {
			return false
		}
	}
	return true
}
