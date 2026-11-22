package live

import (
	"context"
	"errors"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preferences"
	"github.com/ai-poet/cheaprouter-harness/internal/migrate"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

type stubMigrationService struct {
	detect    func(context.Context) (*migrate.LegacyState, error)
	run       func(context.Context, migrate.Options) (*migrate.Result, error)
	readState func(context.Context) (*migrate.Marker, *migrate.Report, error)
}

func (s *stubMigrationService) Detect(ctx context.Context) (*migrate.LegacyState, error) {
	if s.detect != nil {
		return s.detect(ctx)
	}
	return &migrate.LegacyState{}, nil
}

func (s *stubMigrationService) Run(ctx context.Context, opts migrate.Options) (*migrate.Result, error) {
	if s.run != nil {
		return s.run(ctx, opts)
	}
	return &migrate.Result{Status: migrate.Completed}, nil
}

func (s *stubMigrationService) ReadState(ctx context.Context) (*migrate.Marker, *migrate.Report, error) {
	if s.readState != nil {
		return s.readState(ctx)
	}
	return nil, nil, nil
}

func welcomeTestSnapshot() protocol.Snapshot {
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node = protocol.NodeID{1}
	return snapshot
}

func newWelcomeTester(r *Runtime) *ui.Tester {
	tester := ui.NewTesterWith(r.App.View, 1280, 820, ui.TesterOptions{
		Preferences: ui.Preferences{ReduceMotion: true, ReduceTransparency: true},
	})
	tester.SetLocale("en")
	return tester
}

func TestWelcomeOnboardingRoundTrip(t *testing.T) {
	profile := t.TempDir()
	prefsPath := filepath.Join(profile, "desktop", "preferences.json")

	r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
	if err != nil {
		t.Fatal(err)
	}
	tester := newWelcomeTester(r)
	tester.Frame()
	if !r.App.Welcome.Open || r.App.Welcome.Seen {
		t.Fatalf("first launch welcome = %+v, want open and unseen", r.App.Welcome)
	}
	initial := preferences.Open(prefsPath)
	if initial.Data.WelcomeSeen == nil || *initial.Data.WelcomeSeen {
		t.Fatalf("initial preferences WelcomeSeen = %v, want false", initial.Data.WelcomeSeen)
	}
	if err := tester.Click(i18n.In("en", "close")); err != nil {
		t.Fatal(err)
	}
	tester.Frame()
	if r.App.Welcome.Open || !r.App.Welcome.Seen {
		t.Fatalf("after close button click: welcome = %+v, want closed and seen", r.App.Welcome)
	}
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}

	reloaded := preferences.Open(prefsPath)
	if !reloaded.Readable || reloaded.Data.WelcomeSeen == nil || !*reloaded.Data.WelcomeSeen {
		t.Fatalf("persisted preferences WelcomeSeen = %v (readable=%v), want true", reloaded.Data.WelcomeSeen, reloaded.Readable)
	}

	r2, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
	if err != nil {
		t.Fatal(err)
	}
	defer r2.Close()
	tester2 := newWelcomeTester(r2)
	tester2.Frame()
	if r2.App.Welcome.Open || !r2.App.Welcome.Seen || tester2.HasText(i18n.In("en", "onboarding_title")) {
		t.Fatalf("restarted app reopened welcome guide: state=%+v", r2.App.Welcome)
	}
}

func TestWelcomeOnboardingFlushOnCloseBeforeNextFrame(t *testing.T) {
	profile := t.TempDir()
	prefsPath := filepath.Join(profile, "desktop", "preferences.json")

	r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
	if err != nil {
		t.Fatal(err)
	}
	if !r.App.Welcome.Open || r.App.Welcome.Seen {
		t.Fatalf("first launch welcome = %+v, want open and unseen", r.App.Welcome)
	}
	// Dismiss onboarding and quit immediately before another afterFrame runs
	// (such as Cmd+Q on macOS).
	r.App.Welcome.Open, r.App.Welcome.Seen = false, true
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}

	reloaded := preferences.ForStartup(profile)
	if !reloaded.Readable || reloaded.Data.WelcomeSeen == nil || !*reloaded.Data.WelcomeSeen {
		t.Fatalf("preferences after immediate close: WelcomeSeen = %v (readable=%v), want true", reloaded.Data.WelcomeSeen, reloaded.Readable)
	}
	if onDisk := preferences.Open(prefsPath); onDisk.Data.WelcomeSeen == nil || !*onDisk.Data.WelcomeSeen {
		t.Fatalf("on-disk preferences after immediate close: WelcomeSeen = %v, want true", onDisk.Data.WelcomeSeen)
	}
}

func TestWelcomeOnboardingSurvivesMigrationApplyPreferencesAndFailure(t *testing.T) {
	t.Run("migration ApplyPreferences does not reopen or reset dismissed guide", func(t *testing.T) {
		profile := t.TempDir()
		r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
		if err != nil {
			t.Fatal(err)
		}
		var applied atomic.Bool
		service := &stubMigrationService{
			detect: func(context.Context) (*migrate.LegacyState, error) {
				return &migrate.LegacyState{HasAppSettings: true}, nil
			},
			run: func(_ context.Context, opts migrate.Options) (*migrate.Result, error) {
				if opts.ApplyPreferences != nil {
					if err := opts.ApplyPreferences("dark", "chinese"); err != nil {
						return nil, err
					}
					applied.Store(true)
				}
				return &migrate.Result{Status: migrate.Completed}, nil
			},
			readState: func(context.Context) (*migrate.Marker, *migrate.Report, error) {
				if !applied.Load() {
					return nil, nil, nil
				}
				return &migrate.Marker{Version: migrate.Version, Status: migrate.Completed}, nil, nil
			},
		}
		r.SetMigrationService(service, true)
		tester := newWelcomeTester(r)

		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			tester.Frame()
			st := r.App.Settings.Migration
			if st.Known && !st.Checking && !st.Running && st.Status == migrate.Completed {
				break
			}
			time.Sleep(5 * time.Millisecond)
		}
		if r.App.Settings.Migration.Status != migrate.Completed {
			t.Fatalf("migration did not complete: %+v", r.App.Settings.Migration)
		}

		// Dismiss the guide after migration updated preferences. A stale
		// prefsRevision must not cause drain() to re-apply WelcomeSeen=false.
		if err := tester.Click(i18n.In("en", "close")); err != nil {
			t.Fatal(err)
		}
		tester.Frame()
		tester.Frame()
		if r.App.Welcome.Open || !r.App.Welcome.Seen {
			t.Fatalf("welcome guide reopened after migration: %+v", r.App.Welcome)
		}
		if err := r.Close(); err != nil {
			t.Fatal(err)
		}

		reloaded := preferences.ForStartup(profile)
		if reloaded.Data.WelcomeSeen == nil || !*reloaded.Data.WelcomeSeen {
			t.Fatalf("reloaded WelcomeSeen = %v, want true", reloaded.Data.WelcomeSeen)
		}
		if reloaded.Data.Appearance.Mode != "dark" || reloaded.Data.Language == nil || *reloaded.Data.Language != "chinese" {
			t.Fatalf("migrated preferences lost: mode=%q lang=%v", reloaded.Data.Appearance.Mode, reloaded.Data.Language)
		}
	})

	t.Run("dismissal before ApplyPreferences runs is preserved", func(t *testing.T) {
		profile := t.TempDir()
		r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
		if err != nil {
			t.Fatal(err)
		}
		release := make(chan struct{})
		var applied atomic.Bool
		service := &stubMigrationService{
			detect: func(context.Context) (*migrate.LegacyState, error) {
				return &migrate.LegacyState{HasAppSettings: true}, nil
			},
			run: func(ctx context.Context, opts migrate.Options) (*migrate.Result, error) {
				select {
				case <-release:
				case <-ctx.Done():
					return nil, ctx.Err()
				}
				if opts.ApplyPreferences != nil {
					if err := opts.ApplyPreferences("dark", "chinese"); err != nil {
						return nil, err
					}
					applied.Store(true)
				}
				return &migrate.Result{Status: migrate.Completed}, nil
			},
			readState: func(context.Context) (*migrate.Marker, *migrate.Report, error) {
				if !applied.Load() {
					return nil, nil, nil
				}
				return &migrate.Marker{Version: migrate.Version, Status: migrate.Completed}, nil, nil
			},
		}
		r.SetMigrationService(service, true)
		tester := newWelcomeTester(r)
		tester.Frame()
		// Dismiss the guide before ApplyPreferences runs, without yet running afterFrame.
		r.App.Welcome.Open, r.App.Welcome.Seen = false, true
		close(release)

		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			tester.Frame()
			st := r.App.Settings.Migration
			if st.Known && !st.Checking && !st.Running && st.Status == migrate.Completed {
				break
			}
			time.Sleep(5 * time.Millisecond)
		}
		if r.App.Welcome.Open || !r.App.Welcome.Seen {
			t.Fatalf("ApplyPreferences reset already-dismissed welcome guide: %+v", r.App.Welcome)
		}
		if err := r.Close(); err != nil {
			t.Fatal(err)
		}
		reloaded := preferences.ForStartup(profile)
		if reloaded.Data.WelcomeSeen == nil || !*reloaded.Data.WelcomeSeen {
			t.Fatalf("reloaded WelcomeSeen = %v, want true", reloaded.Data.WelcomeSeen)
		}
		if reloaded.Data.Appearance.Mode != "dark" || reloaded.Data.Language == nil || *reloaded.Data.Language != "chinese" {
			t.Fatalf("migrated preferences lost: mode=%q lang=%v", reloaded.Data.Appearance.Mode, reloaded.Data.Language)
		}
	})

	t.Run("migration failure does not reset closed onboarding", func(t *testing.T) {
		profile := t.TempDir()
		r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
		if err != nil {
			t.Fatal(err)
		}
		failService := &stubMigrationService{
			detect: func(context.Context) (*migrate.LegacyState, error) {
				return nil, errors.New("corrupt legacy database")
			},
			readState: func(context.Context) (*migrate.Marker, *migrate.Report, error) {
				return &migrate.Marker{Version: migrate.Version, Status: migrate.Failed, Error: "corrupt legacy database"}, nil, nil
			},
		}
		r.SetMigrationService(failService, true)
		tester := newWelcomeTester(r)
		tester.Frame()
		if err := tester.Click(i18n.In("en", "close")); err != nil {
			t.Fatal(err)
		}
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			tester.Frame()
			st := r.App.Settings.Migration
			if st.Known && !st.Checking && !st.Running && st.Status == migrate.Failed {
				break
			}
			time.Sleep(5 * time.Millisecond)
		}
		if r.App.Settings.Migration.Status != migrate.Failed {
			t.Fatalf("expected migration failure, got %+v", r.App.Settings.Migration)
		}
		if r.App.Welcome.Open || !r.App.Welcome.Seen {
			t.Fatalf("migration failure reset welcome state: %+v", r.App.Welcome)
		}
		if err := r.Close(); err != nil {
			t.Fatal(err)
		}

		r2, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
		if err != nil {
			t.Fatal(err)
		}
		defer r2.Close()
		r2.SetMigrationService(failService, true)
		tester2 := newWelcomeTester(r2)
		tester2.Frame()
		if r2.App.Welcome.Open || !r2.App.Welcome.Seen {
			t.Fatalf("restart after migration failure reopened welcome guide: %+v", r2.App.Welcome)
		}
	})

	t.Run("disconnected backend still persists closed onboarding", func(t *testing.T) {
		profile := t.TempDir()
		controller := ownershipClient(t, welcomeTestSnapshot())
		r, err := NewWithPreferences(t.Context(), controller, preferences.ForStartup(profile))
		if err != nil {
			t.Fatal(err)
		}
		// Simulate backend becoming unavailable before the user dismisses the guide.
		_ = controller.Close()
		tester := newWelcomeTester(r)
		tester.Frame()
		if err := tester.Click(i18n.In("en", "close")); err != nil {
			t.Fatal(err)
		}
		tester.Frame()
		if err := r.Close(); err != nil {
			t.Fatal(err)
		}
		reloaded := preferences.ForStartup(profile)
		if reloaded.Data.WelcomeSeen == nil || !*reloaded.Data.WelcomeSeen {
			t.Fatalf("WelcomeSeen was not persisted while backend unavailable: %v", reloaded.Data.WelcomeSeen)
		}
	})
}

func TestWelcomeOnboardingDoesNotOpenWhenSeenTrue(t *testing.T) {
	profile := t.TempDir()
	store := preferences.Open(filepath.Join(profile, "desktop", "preferences.json"))
	seen := true
	store.Data.WelcomeSeen = &seen
	if err := store.Save(); err != nil {
		t.Fatal(err)
	}
	r, err := NewWithPreferences(t.Context(), ownershipClient(t, welcomeTestSnapshot()), preferences.ForStartup(profile))
	if err != nil {
		t.Fatal(err)
	}
	defer r.Close()
	tester := newWelcomeTester(r)
	tester.Frame()
	if r.App.Welcome.Open || !r.App.Welcome.Seen || tester.HasText(i18n.In("en", "onboarding_title")) {
		t.Fatalf("welcome guide opened despite WelcomeSeen=true: state=%+v", r.App.Welcome)
	}
}
