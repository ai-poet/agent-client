// Package process owns execution-Node command lifetime and bounded output.
package process

import (
	"context"
	"errors"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// ErrCompletionStorageFull marks a refused completion write that needs space
// before it can be attempted again.
var ErrCompletionStorageFull = errors.New("command completion storage is full")

// ErrCompletionNotRegistered means the completion has no admitted record.
var ErrCompletionNotRegistered = errors.New("background command was not registered before execution")

type Launch struct {
	ID             protocol.RequestID
	Session        protocol.SessionID
	Turn           protocol.TurnID
	Worktree       protocol.WorktreeID
	Root           string
	Profile        string
	Command        string
	Access         protocol.ProcessAccess
	ReadOnly       bool // Foreground workspace inspection; only private scratch is writable.
	Cwd            string
	TimeoutMs      uint64
	Pty            bool
	Background     bool
	FollowUp       bool // Report background progress for this launch, including after promotion.
	// Service keeps a background command running through a user stop.
	Service        bool
	Scratch        string
	Inputs         string
	Environment    map[string]string
	Release        func()
	CompletionGate <-chan struct{}
	// promoted closes when a foreground command moves to session ownership.
	promoted <-chan struct{}
	// expired, when set, replaces the runner's own TimeoutMs timer: the
	// service closes it when a timed-out command cannot move to the session.
	expired <-chan struct{}
}

// Finished is persisted by the existing Node writer before completion publishes.
type Finished struct {
	Snapshot  protocol.ProcessSnapshot
	ElapsedMs *uint64
	Explicit  bool
	// Byte totals are present when comparable to the captured UTF-8 text.
	StdoutBytes *uint64
	StderrBytes *uint64
}

// StallWindow is how long a running background command may stay silent before
// its session is asked to decide about it.
const StallWindow = 15 * time.Minute

// Stalled is one silent period of a still-running background command.
type Stalled struct {
	Snapshot  protocol.ProcessSnapshot
	ElapsedMs uint64
	SilenceMs uint64
	// Byte totals cover output observed as of the snapshot.
	StdoutBytes *uint64
	StderrBytes *uint64
}

type Options struct {
	Context     context.Context
	Profile     string
	Environment Environment
	Now         func() time.Time
	After       func(time.Duration) <-chan time.Time
	Finished    func(context.Context, Finished) error
	// Stalled, when set, receives each silent period of a running background
	// command once; output starts the next period.
	Stalled        func(context.Context, Stalled) error
	Changed        func(protocol.WorktreeID)
	CaptureManaged func(string) (string, error)
	// Working, when set, brackets every running command.
	Working func() func()
	Run     func(context.Context, Launch, func(protocol.ProcessOutputOutputStream, []byte), <-chan []byte) (protocol.ProcessCompletion, error)
}
