package trace

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/core"
)

const maxIdle = 180 * 24 * time.Hour

type sessionDisk struct {
	name string
	used time.Time
	size int64
}
type inventory struct {
	total    int64
	sessions []sessionDisk
}

// The index and its shared sidecars are one eviction unit. Index mtime is
// the last append; directory mtime also records explicit inspections.
func scanRetention(base string, now time.Time) (inventory, error) {
	var inv inventory
	if _, err := os.Lstat(base); errors.Is(err, fs.ErrNotExist) {
		return inv, nil
	} else if err != nil {
		return inv, err
	}
	if err := core.PrivateDirectory(base); err != nil {
		return inv, err
	}
	root, err := os.OpenRoot(base)
	if err != nil {
		return inv, err
	}
	defer root.Close()
	tree := root.FS()
	entries, err := fs.ReadDir(tree, ".")
	if err != nil {
		return inv, err
	}
	for _, e := range entries {
		info, err := e.Info()
		if err != nil {
			return inv, err
		}
		if !e.IsDir() {
			inv.total += info.Size()
			continue
		}
		name := e.Name()
		rec := sessionDisk{name: name, used: info.ModTime()}
		lastWrite := rec.used
		if idx, err := root.Lstat(filepath.Join(name, "trace.jsonl")); err == nil && idx.Mode().IsRegular() {
			lastWrite = idx.ModTime()
			if lastWrite.After(rec.used) {
				rec.used = lastWrite
			}
		} else if err != nil && !errors.Is(err, fs.ErrNotExist) {
			return inv, err
		}
		// Reads affect capacity eviction order, but cannot extend age retention.
		if lastWrite.Before(now.Add(-maxIdle)) {
			if err := root.RemoveAll(name); err != nil {
				return inv, err
			}
			continue
		}
		err = fs.WalkDir(tree, name, func(_ string, d fs.DirEntry, err error) error {
			if err != nil || d.IsDir() {
				return err
			}
			i, err := d.Info()
			if err == nil {
				rec.size += i.Size()
			}
			return err
		})
		if err != nil {
			return inv, err
		}
		inv.total += rec.size
		inv.sessions = append(inv.sessions, rec)
	}
	slices.SortFunc(inv.sessions, func(a, b sessionDisk) int {
		if n := a.used.Compare(b.used); n != 0 {
			return n
		}
		return strings.Compare(a.name, b.name)
	})
	return inv, nil
}
func (i inventory) sessionSize(name string) int64 {
	for _, s := range i.sessions {
		if s.name == name {
			return s.size
		}
	}
	return 0
}
func (i inventory) makeRoom(base, current string, limit int64) error {
	if i.total <= limit {
		return nil
	}
	root, err := os.OpenRoot(base)
	if err != nil {
		return err
	}
	defer root.Close()
	for _, s := range i.sessions {
		if s.name == current {
			continue
		}
		if err := root.RemoveAll(s.name); err != nil {
			return err
		}
		i.total -= s.size
		if i.total <= limit {
			return nil
		}
	}
	return errors.New("private trace quota unavailable")
}
func Prune(profile string) error {
	if profile == "" {
		return fs.ErrInvalid
	}
	publication.Lock()
	defer publication.Unlock()
	base := filepath.Join(profile, "traces")
	i, err := scanRetention(base, time.Now())
	if err != nil {
		return err
	}
	return i.makeRoom(base, "", maxProfile)
}

// ReadLease protects the selected index and sidecars against clear, expiry
// and capacity eviction until the reader has copied its selection.
func ReadLease(profile, session string) func() {
	publication.RLock()
	if profile != "" && component(session) {
		if root, err := os.OpenRoot(profile); err == nil {
			dir := filepath.Join("traces", session)
			if base, err := root.Lstat("traces"); err == nil && base.IsDir() {
				if s, err := root.Lstat(dir); err == nil && s.IsDir() {
					now := time.Now()
					_ = root.Chtimes(dir, now, now)
				}
			}
			root.Close()
		}
	}
	return sync.OnceFunc(publication.RUnlock)
}
