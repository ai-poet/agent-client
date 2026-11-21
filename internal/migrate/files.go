package migrate

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func backupSources(ctx context.Context, state *LegacyState, profile string, now time.Time) (string, error) {
	if err := ensureSeparateRoots(state.Root, profile); err != nil {
		return "", fail("backup", "validate destination", err)
	}
	if state.HasDatabase && state.DatabasePath != "" {
		if err := ensureSeparateRoots(filepath.Dir(state.DatabasePath), profile); err != nil {
			return "", fail("backup", "validate database destination", err)
		}
	}
	base := "migration-backup-" + now.UTC().Format("20060102T150405Z")
	backup := filepath.Join(profile, base)
	for suffix := 1; ; suffix++ {
		err := os.Mkdir(backup, 0o700)
		if err == nil {
			break
		}
		if !errors.Is(err, os.ErrExist) {
			if errors.Is(err, os.ErrNotExist) {
				if err := os.MkdirAll(profile, 0o700); err != nil {
					return "", fail("backup", "create profile", err)
				}
				continue
			}
			return "", fail("backup", "create backup directory", err)
		}
		backup = filepath.Join(profile, fmt.Sprintf("%s-%02d", base, suffix))
	}

	// Active credentials (cloud-account.json) are never copied into the backup
	// directory: the legacy profile stays untouched and read-only on disk, and
	// the new Sailry profile must not retain plaintext legacy tokens.
	for _, name := range []string{legacyDatabaseName, legacyDatabaseName + "-wal", legacyAppSettingsName, legacySettingsName} {
		if err := ctx.Err(); err != nil {
			return "", fail("backup", "cancel", err)
		}
		source := filepath.Join(state.Root, name)
		if name == legacyDatabaseName && state.DatabasePath != "" {
			source = state.DatabasePath
		} else if name == legacyDatabaseName+"-wal" && state.DatabasePath != "" {
			source = state.DatabasePath + "-wal"
		}
		present, err := regularFile(source)
		if err != nil {
			return "", fail("backup", "inspect "+name, err)
		}
		if !present {
			continue
		}
		if err := copyPrivate(source, filepath.Join(backup, name)); err != nil {
			return "", fail("backup", "copy "+name, err)
		}
	}
	return backup, nil
}

func ensureSeparateRoots(legacyRoot, profile string) error {
	destination, err := absoluteClean(profile)
	if err != nil {
		return err
	}
	relative, err := filepath.Rel(legacyRoot, destination)
	if err != nil {
		return err
	}
	if relative == "." || relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator)) && !filepath.IsAbs(relative) {
		return errors.New("the Sailry profile must not be the legacy directory or its child")
	}
	return nil
}

func copyPrivate(source, destination string) (returnErr error) {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer input.Close()
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	defer func() {
		if err := output.Close(); returnErr == nil && err != nil {
			returnErr = err
		}
	}()
	if _, err := io.Copy(output, input); err != nil {
		return err
	}
	return output.Sync()
}

func writeJSONAtomic(path string, value any) error {
	encoded, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	temporary, err := os.CreateTemp(filepath.Dir(path), ".migration-*")
	if err != nil {
		return err
	}
	name := temporary.Name()
	defer os.Remove(name)
	defer temporary.Close()
	if err := temporary.Chmod(0o600); err != nil {
		return err
	}
	if _, err := temporary.Write(encoded); err != nil {
		return err
	}
	if err := temporary.Sync(); err != nil {
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}
	if err := os.Rename(name, path); err == nil {
		return nil
	}
	// Windows cannot atomically replace an existing report during fill-missing.
	// Only the profile-owned destination is removed, never a legacy source.
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return err
	}
	return os.Rename(name, path)
}

func existingMarker(profile string) (bool, error) {
	path := filepath.Join(profile, MarkerFilename)
	raw, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	if err != nil {
		return false, fail("detect", "read migration marker", err)
	}
	var marker Marker
	if err := json.Unmarshal(raw, &marker); err != nil || marker.Version <= 0 {
		return false, fail("detect", "decode migration marker", errors.New("invalid migration marker"))
	}
	if marker.Status == Failed {
		return false, nil
	}
	return true, nil
}
