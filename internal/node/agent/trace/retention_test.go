package trace

import (
	"errors"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func TestRetentionReadRecencyDoesNotExtendExpiry(t *testing.T) {
	profile := t.TempDir()
	base := filepath.Join(profile, "traces")
	now := time.Now()
	for name, age := range map[string]time.Duration{"expired": maxIdle + 24*time.Hour, "read": 3 * time.Hour, "cold": 2 * time.Hour, "recent": time.Hour} {
		dir := filepath.Join(base, name)
		if err := os.MkdirAll(dir, 0700); err != nil {
			t.Fatal(err)
		}
		index := filepath.Join(dir, "trace.jsonl")
		if err := os.WriteFile(index, []byte("data"), 0600); err != nil {
			t.Fatal(err)
		}
		for _, path := range []string{index, dir} {
			when := now.Add(-age)
			if err := os.Chtimes(path, when, when); err != nil {
				t.Fatal(err)
			}
		}
	}
	for _, s := range []string{"expired", "read"} {
		release := ReadLease(profile, s)
		release()
	}
	if err := Prune(profile); err != nil {
		t.Fatal(err)
	}
	inv, err := scanRetention(base, now)
	if err != nil {
		t.Fatal(err)
	}
	if err := inv.makeRoom(base, "", 8); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"cold", "expired"} {
		if _, err := os.Stat(filepath.Join(base, name)); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("not removed %s: %v", name, err)
		}
	}
	for _, name := range []string{"recent", "read"} {
		if _, err := os.Stat(filepath.Join(base, name, "trace.jsonl")); err != nil {
			t.Fatal(err)
		}
	}
}

func TestClearAllInvalidatesConcurrentAttemptsButNotAnotherProfile(t *testing.T) {
	p := t.TempDir()
	other := t.TempDir()
	keep := (&Scope{Profile: other, Session: "s"}).Begin("r", 1)
	var wg sync.WaitGroup
	gate := make(chan struct{})
	for i := range 12 {
		a := (&Scope{Profile: p, Session: "s"}).Begin("r", i)
		wg.Add(1)
		go func() { defer wg.Done(); <-gate; a.Finish(Facts{}) }()
	}
	close(gate)
	if err := ClearAll(p); err != nil {
		t.Fatal(err)
	}
	wg.Wait()
	if _, err := os.Stat(filepath.Join(p, "traces", "s")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("stale attempt: %v", err)
	}
	keep.Finish(Facts{})
	readFile(t, filepath.Join(other, "traces", "s", "trace.jsonl"))
	fresh := (&Scope{Profile: p, Session: "s"}).Begin("r", 1)
	fresh.Finish(Facts{})
	readFile(t, filepath.Join(p, "traces", "s", "trace.jsonl"))
}

func TestRetentionDoesNotFollowExternalLinks(t *testing.T) {
	profile := t.TempDir()
	outside := t.TempDir()
	file := filepath.Join(outside, "unrelated")
	if err := os.WriteFile(file, []byte("keep"), 0600); err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(profile, "traces", "expired")
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(dir, "bodies")); err != nil {
		t.Skip(err)
	}
	old := time.Now().Add(-2 * maxIdle)
	if err := os.Chtimes(dir, old, old); err != nil {
		t.Fatal(err)
	}
	if err := Prune(profile); err != nil {
		t.Fatal(err)
	}
	if string(readFile(t, file)) != "keep" {
		t.Fatal("external data changed")
	}
}
