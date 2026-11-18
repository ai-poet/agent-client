package update

import (
	"context"
	"errors"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

// Update is a newer release returned by MyGo. Install remains bound to the
// source and build-time trust configuration that produced it.
type Update struct {
	Version string
	Notes   string
	Date    time.Time
	native  *nativeRelease
}

// Install downloads, verifies and installs through MyGo, forwarding its byte
// progress unchanged. No relaunch is implicit.
func (u *Update) Install(ctx context.Context, progress func(downloaded, total int64)) error {
	if u == nil || u.native == nil || u.native.install == nil {
		return fmt.Errorf("%w: no checked update is available", ErrUnavailable)
	}
	if err := u.native.install(ctx, progress); err != nil {
		return classifyInstall(err)
	}
	return nil
}

// Client performs checks for one running version and configuration snapshot.
type Client struct {
	config  Config
	version string
	native  nativeUpdater
}

// NewClient creates a client over the official MyGo updater.
func NewClient(config Config, version string) *Client {
	return &Client{config: config, version: version, native: &mygoNative{}}
}

// Enabled reports whether the user has supplied and enabled a complete source.
// A packaged build must also support MyGo installation before Check can run.
func (c *Client) Enabled() bool { return c != nil && c.config.Configured() }

// NativeEnabled reports whether this build and installation can self-update.
func (c *Client) NativeEnabled() bool {
	return c != nil && c.native != nil && c.native.Enabled()
}

// Check returns only a version newer than the running application. MyGo owns
// manifest parsing and semantic-version comparison; nil means up to date.
func (c *Client) Check(ctx context.Context) (*Update, error) {
	if !c.Enabled() {
		return nil, ErrDisabled
	}
	if !c.native.Enabled() {
		return nil, ErrUnavailable
	}
	source := &Source{FeedURL: strings.TrimSpace(c.config.FeedURL), Version: c.version}
	release, err := c.native.check(ctx, source, c.config.Prereleases)
	if err != nil {
		return nil, classifyCheck(err)
	}
	if release == nil {
		return nil, nil
	}
	return &Update{Version: release.Version, Notes: release.Notes, Date: release.Date, native: release}, nil
}

type nativeRelease struct {
	Version string
	Notes   string
	Date    time.Time
	install func(context.Context, func(int64, int64)) error
}

type nativeUpdater interface {
	Enabled() bool
	check(context.Context, mygo.UpdateSource, bool) (*nativeRelease, error)
}

// mygoNative serializes changes to MyGo's process-wide source and prerelease
// settings. An installed release retains its own source after Check returns.
type mygoNative struct{}

// mygoMu protects MyGo's process-wide source and prerelease settings across
// every Client instance.
var mygoMu sync.Mutex

func (m *mygoNative) Enabled() bool { return mygo.Updater.Enabled() }

func (m *mygoNative) check(ctx context.Context, source mygo.UpdateSource, prereleases bool) (*nativeRelease, error) {
	mygoMu.Lock()
	defer mygoMu.Unlock()
	mygo.Updater.SetSource(source)
	mygo.Updater.SetPrereleases(prereleases)
	found, err := mygo.Updater.Check(ctx)
	if err != nil || found == nil {
		return nil, err
	}
	return &nativeRelease{Version: found.Version, Notes: found.Notes, Date: found.Date, install: found.Install}, nil
}

func classifyCheck(err error) error {
	if err == nil {
		return nil
	}
	for _, class := range []error{ErrDisabled, ErrUnavailable, ErrConfiguration, ErrNetwork, ErrNotFound, ErrManifest} {
		if errors.Is(err, class) {
			return err
		}
	}
	if errors.Is(err, mygo.ErrUpdatesDisabled) {
		return fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return fmt.Errorf("%w: %v", ErrNetwork, err)
	}
	// Once Source succeeded, MyGo's remaining check failures concern decoding
	// or validating its manifest.
	return fmt.Errorf("%w: %v", ErrManifest, err)
}

func classifyInstall(err error) error {
	if err == nil {
		return nil
	}
	if errors.Is(err, ErrSignature) {
		return err
	}
	for _, class := range []error{ErrNetwork, ErrNotFound, ErrConfiguration, ErrInstall} {
		if errors.Is(err, class) {
			return err
		}
	}
	message := strings.ToLower(err.Error())
	if strings.Contains(message, "not signed") || strings.Contains(message, "signature") || strings.Contains(message, "invalid update public key") {
		return fmt.Errorf("%w: %v", ErrSignature, err)
	}
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return fmt.Errorf("%w: %v", ErrNetwork, err)
	}
	return fmt.Errorf("%w: %v", ErrInstall, err)
}
