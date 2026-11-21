package migrate

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"

	_ "modernc.org/sqlite"
)

// Detect inventories legacyRoot without creating, modifying or deleting any
// source file. A missing database is an empty inventory, not an error.
func Detect(legacyRoot string) (*LegacyState, error) {
	return detect(context.Background(), legacyRoot)
}

func detect(ctx context.Context, legacyRoot string) (*LegacyState, error) {
	root, err := absoluteClean(legacyRoot)
	if err != nil {
		return nil, fail("detect", "resolve legacy root", err)
	}
	state := &LegacyState{Root: root, DatabasePath: filepath.Join(root, legacyDatabaseName)}
	for name, destination := range map[string]*bool{
		legacyDatabaseName:    &state.HasDatabase,
		legacyAppSettingsName: &state.HasAppSettings,
		legacySettingsName:    &state.HasSettings,
		legacyCredentialName:  &state.HasCredentials,
	} {
		present, err := regularFile(filepath.Join(root, name))
		if err != nil {
			return nil, fail("detect", "inspect "+name, err)
		}
		*destination = present
	}
	if !state.HasDatabase {
		return state, nil
	}

	db, cleanup, err := openLegacySnapshot(ctx, filepath.Join(root, legacyDatabaseName))
	if err != nil {
		return nil, err
	}
	defer cleanup()
	defer db.Close()
	for table, destination := range map[string]*int{
		"projects": &state.Counts.Projects,
		"sessions": &state.Counts.Sessions,
		"messages": &state.Counts.Messages,
	} {
		// The table names are constants, never source-controlled SQL.
		if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM "+table).Scan(destination); err != nil {
			return nil, fail("detect", "count "+table, err)
		}
	}
	return state, nil
}

func regularFile(path string) (bool, error) {
	info, err := os.Stat(path)
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	if !info.Mode().IsRegular() {
		return false, fmt.Errorf("%s is not a regular file", filepath.Base(path))
	}
	return true, nil
}

func openLegacy(ctx context.Context, path string) (*sql.DB, error) {
	absolute, err := absoluteClean(path)
	if err != nil {
		return nil, fail("detect", "resolve database", err)
	}
	db, err := sql.Open("sqlite", legacyReadOnlyURI(absolute))
	if err != nil {
		return nil, fail("detect", "open read-only database", err)
	}
	db.SetMaxOpenConns(1)
	if err := db.PingContext(ctx); err != nil {
		db.Close()
		return nil, fail("detect", "open read-only database", err)
	}
	return db, nil
}

func legacyReadOnlyURI(path string) string {
	query := url.Values{}
	query.Set("mode", "ro")
	query.Add("_pragma", "query_only(1)")
	query.Add("_pragma", "busy_timeout(0)")
	uri := &url.URL{
		Scheme:   "file",
		Path:     normalizeSQLiteURIPath(path),
		RawQuery: query.Encode(),
	}
	return uri.String()
}

func normalizeSQLiteURIPath(path string) string {
	if isWindowsDrivePath(path) {
		return "/" + strings.ReplaceAll(path, `\`, "/")
	}
	if strings.HasPrefix(path, `\\`) {
		return strings.ReplaceAll(path, `\`, "/")
	}
	slashed := filepath.ToSlash(path)
	if strings.HasPrefix(slashed, "//") || strings.HasPrefix(slashed, "/") {
		return slashed
	}
	return "/" + slashed
}

func isWindowsDrivePath(path string) bool {
	if len(path) < 2 || path[1] != ':' {
		return false
	}
	drive := path[0]
	return (drive >= 'A' && drive <= 'Z') || (drive >= 'a' && drive <= 'z')
}

func openLegacySnapshot(ctx context.Context, path string) (*sql.DB, func(), error) {
	directory, err := os.MkdirTemp("", "cheaprouter-legacy-detect-")
	if err != nil {
		return nil, nil, fail("detect", "create read-only snapshot", err)
	}
	cleanup := func() { _ = os.RemoveAll(directory) }
	if err := copyPrivate(path, filepath.Join(directory, legacyDatabaseName)); err != nil {
		cleanup()
		return nil, nil, fail("detect", "copy database snapshot", err)
	}
	wal := path + "-wal"
	if present, err := regularFile(wal); err == nil && present {
		if err := copyPrivate(wal, filepath.Join(directory, legacyDatabaseName+"-wal")); err != nil {
			cleanup()
			return nil, nil, fail("detect", "copy database WAL snapshot", err)
		}
	} else if err != nil {
		cleanup()
		return nil, nil, fail("detect", "inspect database WAL", err)
	}
	if err := ctx.Err(); err != nil {
		cleanup()
		return nil, nil, fail("detect", "cancel", err)
	}
	db, err := openLegacy(ctx, filepath.Join(directory, legacyDatabaseName))
	if err != nil {
		cleanup()
		return nil, nil, err
	}
	return db, cleanup, nil
}

func absoluteClean(path string) (string, error) {
	if path == "" {
		return "", errors.New("empty path")
	}
	absolute, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	return filepath.Clean(absolute), nil
}
