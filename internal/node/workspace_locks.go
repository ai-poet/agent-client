package node

import (
	"context"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"

	"github.com/ai-poet/cheaprouter-harness/internal/node/files"
)

// workspaceLocks orders effects on overlapping files or whole worktrees.
// A registration excludes every effect while it changes the set of roots.
type workspaceLocks struct {
	registry sync.RWMutex
	// registrations counts the recorded ones, so an effect that waited can
	// tell that its roots may have changed.
	registrations atomic.Uint64
	mu            sync.Mutex
	held          map[workspacePath]chan struct{}
	// settings orders the saves whose refresh after the commit reads the
	// saved settings back and applies them, so the last save is the applied
	// one. They work in no root.
	settings sync.Mutex
}

type workspacePath struct {
	name string
	root bool
}

// Each file waits only for the preceding writer of that file and an operation
// on its entire worktree. A worktree removal must also drain its active files.
func (locks *workspaceLocks) reserve(keys []string, root bool) func() {
	if len(keys) == 0 {
		return func() {}
	}
	done := make(chan struct{})
	var wait []chan struct{}
	locks.mu.Lock()
	if locks.held == nil {
		locks.held = make(map[workspacePath]chan struct{})
	}
	wait = locks.predecessors(keys, root)
	for _, key := range keys {
		locks.held[workspacePath{name: key, root: root}] = done
	}
	locks.mu.Unlock()
	for _, prior := range wait {
		<-prior
	}
	return func() {
		locks.mu.Lock()
		for _, name := range keys {
			key := workspacePath{name: name, root: root}
			if locks.held[key] == done {
				delete(locks.held, key)
			}
		}
		close(done)
		locks.mu.Unlock()
	}
}

// predecessors snapshots the queues already registered for these targets.
// The caller holds mu, so a multi-file reservation is installed atomically.
func (locks *workspaceLocks) predecessors(keys []string, root bool) []chan struct{} {
	var wait []chan struct{}
	for _, key := range keys {
		if prior := locks.held[workspacePath{name: key}]; prior != nil {
			wait = append(wait, prior)
		}
		for parent := key; ; {
			if prior := locks.held[workspacePath{name: parent, root: true}]; prior != nil {
				wait = append(wait, prior)
			}
			dir := filepath.ToSlash(filepath.Dir(filepath.FromSlash(parent)))
			if dir == parent {
				break
			}
			parent = dir
		}
		if root {
			prefix := strings.TrimSuffix(key, "/") + "/"
			for held, prior := range locks.held {
				if strings.HasPrefix(held.name, prefix) {
					wait = append(wait, prior)
				}
			}
		}
	}
	return wait
}
func (locks *workspaceLocks) waitFiles(ctx context.Context, keys []string) error {
	locks.mu.Lock()
	wait := locks.predecessors(keys, false)
	locks.mu.Unlock()
	for _, prior := range wait {
		select {
		case <-prior:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return ctx.Err()
}

// A command that operates in no worktree has no root to reserve.
func (locks *workspaceLocks) roots(roots ...string) func() {
	var keys []string
	for _, root := range roots {
		if root == "" {
			continue
		}
		if key := workspaceFileKey(root, ""); key != "" && !slices.Contains(keys, key) {
			keys = append(keys, key)
		}
	}
	return locks.reserve(keys, true)
}

// Paths are relative to the actual filesystem root used to prepare the change,
// which can be a volume root on calls with external paths.
func preparedFileKeys(root string, prepared *files.Prepared) []string {
	if prepared == nil || prepared.Rejected != nil {
		return nil
	}
	keys := make([]string, 0, len(prepared.Changes))
	for _, change := range prepared.Changes {
		keys = append(keys, workspaceFileKey(root, change.Path))
		if change.Destination != nil {
			keys = append(keys, workspaceFileKey(root, *change.Destination))
		}
	}
	return keys
}

func (locks *workspaceLocks) fileEffect(root string, prepared *files.Prepared) func() {
	if keys := preparedFileKeys(root, prepared); len(keys) != 0 {
		return locks.joinEffect(locks.reserve(keys, false))
	}
	return locks.effect(root)
}

func (locks *workspaceLocks) effect(roots ...string) func() {
	return locks.joinEffect(locks.roots(roots...))
}

// Joining after the paths are available lets registrations pass queued effects.
func (locks *workspaceLocks) joinEffect(unlock func()) func() {
	locks.registry.RLock()
	return func() {
		locks.registry.RUnlock()
		unlock()
	}
}

// change is the effect that creates or removes a worktree: Git works on disk
// and the row follows. It holds its roots and joins the effects, from Git's
// work to the row, so a slow checkout stalls those roots and not every effect
// on the Node. A project command takes the registration and so still cannot
// move or remove the project between the two. Its release counts as a
// registration before the roots open, so an effect that waited for a removed
// root resolves it again and finds it gone.
//
// Rust marks the worktree and its project busy in a table of pending removals
// instead; the locks here already order the same commands. While a project
// command waits for a change to end, new effects wait behind it, as they do
// behind any running effect.
func (locks *workspaceLocks) change(roots ...string) (release func()) {
	unlock := locks.effect(roots...)
	return func() {
		locks.registrations.Add(1)
		unlock()
	}
}

// registration excludes every effect until it is released.
func (locks *workspaceLocks) registration() (release func()) {
	locks.registry.Lock()
	return func() {
		locks.registrations.Add(1)
		locks.registry.Unlock()
	}
}
