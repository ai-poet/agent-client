package live

import (
	"context"
	"errors"
	"log"
	"sync"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/shell"
	clientupdate "github.com/ai-poet/cheaprouter-harness/internal/update"
	"github.com/egoist/mygo/ui"
)

const nativeUpdateDelay = 8 * time.Second

// NativeUpdates is the process's MyGo update service. All network, download
// and installation work stays off the UI thread; windows only read snapshots.
type NativeUpdates struct {
	store         *clientupdate.Store
	version       string
	relaunch      func()
	nativeEnabled bool

	mu       sync.Mutex
	state    settings.UpdateState
	found    *clientupdate.Update
	foundVer string
	copied   int64
	total    int64
	failure  *settings.UpdateFailure
	current  bool
	notices  []nativeUpdateNotice
	cancel   context.CancelFunc
	sequence uint64
	closed   bool
	started  bool
	done     chan struct{}
	workers  sync.WaitGroup
	changed  func()
}

type nativeUpdateNotice struct {
	key     string
	version string
	failure bool
}

// NewNativeUpdates creates a client updater without starting network work.
// MyGo's native support is probed once, never from a frame.
func NewNativeUpdates(store *clientupdate.Store, version string, relaunch func()) *Updates {
	updates := &Updates{}
	native := &NativeUpdates{store: store, version: version, relaunch: relaunch, done: make(chan struct{})}
	native.nativeEnabled = clientupdate.NewClient(store.Config(), version).NativeEnabled()
	native.changed = updates.changed
	updates.Native = native
	return updates
}

// Close stops startup, checks and downloads, then waits for their workers.
func (u *Updates) Close() {
	if u == nil {
		return
	}
	if u.Service != nil {
		u.Service.Close()
	}
	if u.Native != nil {
		u.Native.Close()
	}
}

// Startup runs the legacy receipt/watch service or one quiet MyGo launch check.
func (u *Updates) Startup() {
	if u == nil {
		return
	}
	if u.Service != nil {
		u.Service.Startup()
	}
	if u.Native != nil {
		u.Native.Startup()
	}
}

// Startup schedules exactly one quiet check when both config switches allow it.
func (n *NativeUpdates) Startup() {
	n.mu.Lock()
	if n.started || n.closed {
		n.mu.Unlock()
		return
	}
	n.started = true
	config := n.store.Config()
	if !config.Enabled || !config.Automatic || !config.Configured() || !n.nativeEnabled {
		n.mu.Unlock()
		return
	}
	n.workers.Add(1)
	n.mu.Unlock()
	go func() {
		defer n.workers.Done()
		timer := time.NewTimer(nativeUpdateDelay)
		defer timer.Stop()
		select {
		case <-n.done:
			return
		case <-timer.C:
			n.check(true)
		}
	}()
}

func (n *NativeUpdates) Close() {
	n.mu.Lock()
	if !n.closed {
		n.closed = true
		close(n.done)
	}
	if n.cancel != nil {
		n.cancel()
	}
	n.sequence++
	n.mu.Unlock()
	n.workers.Wait()
}

// Check starts a manual check whose outcome is visible in Settings and toasts.
func (n *NativeUpdates) Check() { n.check(false) }

func (n *NativeUpdates) check(quiet bool) {
	n.mu.Lock()
	if n.closed || n.state == settings.UpdateChecking || n.state == settings.UpdateDownloading || n.state == settings.UpdateRestarting {
		n.mu.Unlock()
		return
	}
	config := n.store.Config()
	if !config.Configured() || !n.nativeEnabled {
		if !quiet {
			cause := clientupdate.ErrUnavailable
			if !config.Configured() {
				if err := config.Validate(); err != nil {
					cause = err
				} else {
					cause = clientupdate.ErrDisabled
				}
			}
			n.failure = nativeFailure(cause)
			n.notices = append(n.notices, nativeUpdateNotice{key: n.failure.Key, failure: true})
		}
		n.mu.Unlock()
		n.changed()
		return
	}
	if n.cancel != nil {
		n.cancel()
	}
	ctx, cancel := context.WithCancel(context.Background())
	n.cancel = cancel
	n.sequence++
	sequence := n.sequence
	n.state = settings.UpdateChecking
	if !quiet {
		n.failure = nil
		n.current = false
	}
	n.workers.Add(1)
	n.mu.Unlock()
	n.changed()
	go func() {
		defer n.workers.Done()
		defer cancel()
		found, err := clientupdate.NewClient(config, n.version).Check(ctx)
		n.mu.Lock()
		if sequence != n.sequence {
			n.mu.Unlock()
			return
		}
		switch {
		case err != nil:
			n.state = n.restingLocked()
			if quiet {
				log.Printf("desktop update check: %v", err)
			} else {
				n.failLocked(err)
			}
		case found == nil:
			n.found, n.foundVer = nil, ""
			n.state, n.failure, n.current = settings.UpdateIdle, nil, true
			if !quiet {
				n.notices = append(n.notices, nativeUpdateNotice{key: "updates_current"})
			}
		default:
			n.found, n.foundVer = found, found.Version
			n.state, n.failure, n.current = settings.UpdateAvailable, nil, false
			n.notices = append(n.notices, nativeUpdateNotice{key: "updates_available", version: found.Version})
		}
		n.mu.Unlock()
		n.changed()
	}()
}

// Install downloads, verifies and installs the update found by Check.
func (n *NativeUpdates) Install() {
	n.mu.Lock()
	if n.closed || n.state != settings.UpdateAvailable || n.found == nil {
		n.mu.Unlock()
		return
	}
	if n.cancel != nil {
		n.cancel()
	}
	ctx, cancel := context.WithCancel(context.Background())
	n.cancel = cancel
	n.sequence++
	sequence := n.sequence
	found := n.found
	n.state, n.failure = settings.UpdateDownloading, nil
	n.copied, n.total = 0, 0
	n.workers.Add(1)
	n.mu.Unlock()
	n.changed()
	go func() {
		defer n.workers.Done()
		defer cancel()
		err := found.Install(ctx, func(downloaded, total int64) {
			n.mu.Lock()
			if sequence == n.sequence {
				n.copied, n.total = downloaded, total
			}
			n.mu.Unlock()
			n.changed()
		})
		n.mu.Lock()
		if sequence != n.sequence {
			n.mu.Unlock()
			return
		}
		if err != nil {
			n.state = settings.UpdateAvailable
			n.failLocked(err)
		} else {
			n.state = settings.UpdateReady
			n.notices = append(n.notices, nativeUpdateNotice{key: "updates_ready", version: found.Version})
		}
		n.mu.Unlock()
		n.changed()
	}()
}

// Cancel abandons the current check or download and restores its prior state.
func (n *NativeUpdates) Cancel() {
	n.mu.Lock()
	if n.state == settings.UpdateChecking || n.state == settings.UpdateDownloading {
		if n.cancel != nil {
			n.cancel()
		}
		n.sequence++
		n.state = n.restingLocked()
		n.copied, n.total = 0, 0
	}
	n.mu.Unlock()
	n.changed()
}

// Restart launches the installed version. MyGo performs no implicit relaunch.
func (n *NativeUpdates) Restart() {
	n.mu.Lock()
	if n.state != settings.UpdateReady || n.relaunch == nil {
		n.mu.Unlock()
		return
	}
	n.state = settings.UpdateRestarting
	relaunch := n.relaunch
	n.mu.Unlock()
	n.changed()
	relaunch()
}

// SaveOptions persists both switches. A prerelease change invalidates a
// selection made from the previous channel.
func (n *NativeUpdates) SaveOptions(automatic, prereleases bool) {
	n.mu.Lock()
	config := n.store.Config()
	channelChanged := config.Prereleases != prereleases
	config.Automatic, config.Prereleases = automatic, prereleases
	if err := n.store.Save(config); err != nil {
		n.failure = &settings.UpdateFailure{Key: "updates_save_failed", Detail: err.Error()}
		n.notices = append(n.notices, nativeUpdateNotice{key: "updates_save_failed", failure: true})
	} else {
		n.failure = nil
		if channelChanged {
			if n.cancel != nil {
				n.cancel()
			}
			n.sequence++
			n.found, n.foundVer = nil, ""
			n.state = settings.UpdateIdle
			n.current = false
		}
	}
	n.mu.Unlock()
	n.changed()
}

func (n *NativeUpdates) restingLocked() settings.UpdateState {
	if n.found != nil {
		return settings.UpdateAvailable
	}
	return settings.UpdateIdle
}

func (n *NativeUpdates) failLocked(err error) {
	n.failure = nativeFailure(err)
	n.notices = append(n.notices, nativeUpdateNotice{key: n.failure.Key, failure: true})
}

func nativeFailure(err error) *settings.UpdateFailure {
	key := "updates_install_failed"
	switch {
	case errors.Is(err, clientupdate.ErrNotFound):
		key = "updates_not_found"
	case errors.Is(err, clientupdate.ErrNetwork):
		key = "updates_network_failed"
	case errors.Is(err, clientupdate.ErrManifest):
		key = "updates_manifest_invalid"
	case errors.Is(err, clientupdate.ErrSignature):
		key = "updates_signature_invalid"
	case errors.Is(err, clientupdate.ErrUnconfigured):
		key = "updates_not_configured"
	case errors.Is(err, clientupdate.ErrDisabled), errors.Is(err, clientupdate.ErrConfiguration):
		key = "updates_source_unready"
	case errors.Is(err, clientupdate.ErrUnavailable):
		key = "updates_build_unsupported"
	}
	return &settings.UpdateFailure{Key: key, Detail: err.Error()}
}

func (n *NativeUpdates) snapshot() (settings.UpdateState, string, int64, int64, *settings.UpdateFailure, bool) {
	n.mu.Lock()
	defer n.mu.Unlock()
	var failure *settings.UpdateFailure
	if n.failure != nil {
		copy := *n.failure
		failure = &copy
	}
	return n.state, n.foundVer, n.copied, n.total, failure, n.current
}

func (n *NativeUpdates) takeNotices() []nativeUpdateNotice {
	n.mu.Lock()
	defer n.mu.Unlock()
	notices := n.notices
	n.notices = nil
	return notices
}

// nativeAboutFrame projects the shared service and presents notices only in
// the focused window. Quiet launch failures and up-to-date results no-op.
func (r *Runtime) nativeAboutFrame(c *ui.Context, native *NativeUpdates) {
	m := r.App.Settings
	config := native.store.Config()
	state, version, copied, total, failure, current := native.snapshot()
	u := &m.Updater
	u.Native, u.Configured, u.NativeEnabled = true, config.Configured(), native.nativeEnabled
	u.Automatic, u.Prereleases = config.Automatic, config.Prereleases
	u.State, u.Version, u.Copied, u.Total, u.Failure = state, version, copied, total, failure
	u.Current = current
	if r.updatesFocused != nil && !r.updatesFocused() {
		return
	}
	about := func() {
		m.Open(settings.About)
		r.App.Navigate(shell.SettingsPage)
	}
	for _, notice := range native.takeNotices() {
		toast := ui.Toast{Type: "success", Title: i18n.T(c, notice.key, "version", notice.version), Timeout: kit.ToastDuration(c)}
		if notice.failure {
			toast.Type = "error"
		}
		if notice.version != "" || notice.failure {
			toast.Action, toast.OnAction = i18n.T(c, "updates_view"), about
		}
		c.AddToast(toast)
	}
}
