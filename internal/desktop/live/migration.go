package live

import (
	"context"
	"errors"
	"log"
	"log/slog"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/migrate"
	"github.com/egoist/mygo/ui"
)

// MigrationService is the local Node/profile boundary used by Desktop. It is
// deliberately not a remote Client command: legacy data belongs to this profile.
type MigrationService interface {
	Detect(context.Context) (*migrate.LegacyState, error)
	Run(context.Context, migrate.Options) (*migrate.Result, error)
	ReadState(context.Context) (*migrate.Marker, *migrate.Report, error)
}

type migrationRuntime struct {
	service  MigrationService
	checking bool
	running  bool
}

// SetMigrationService attaches the local migration owner and starts one
// read-only state check. When autoStart is true (first launch window), any
// detected unmigrated legacy profile is backed up and migrated silently without
// blocking the main workspace with a confirmation dialog.
func (r *Runtime) SetMigrationService(service MigrationService, autoStart bool) {
	if service == nil {
		return
	}
	r.migration = &migrationRuntime{service: service}
	r.App.Settings.MigrationActions = &settings.MigrationActions{Run: r.runMigration}
	r.checkMigration(autoStart)
}

func (r *Runtime) syncMigrationAccountState() {
	if r.cheapAuth == nil || r.cheapAuth.state == nil {
		return
	}
	migration := &r.App.Settings.Migration
	if migration.Failure != "" {
		r.cheapAuth.state.MigrationFailure = migration.Failure
		r.cheapAuth.state.MigrationFailureDetail = migration.FailureDetail
		r.cheapAuth.state.RetryMigration = func() { r.runMigration(true) }
	} else {
		r.cheapAuth.state.MigrationFailure = ""
		r.cheapAuth.state.MigrationFailureDetail = ""
		r.cheapAuth.state.RetryMigration = nil
	}
}

func (r *Runtime) checkMigration(autoStart bool) {
	if r.migration == nil || r.migration.checking || r.migration.running {
		return
	}
	r.migration.checking = true
	r.App.Settings.Migration.Checking = true
	r.App.Settings.Migration.Known = true
	r.async(func() {
		state, detectErr := r.migration.service.Detect(r.ctx)
		marker, report, readErr := r.migration.service.ReadState(r.ctx)
		if err := errors.Join(detectErr, readErr); err != nil {
			slog.Error("legacy migration check failed", "error", err.Error())
			log.Printf("legacy migration check failed: %v", err)
		}
		r.enqueue(func(c *ui.Context) {
			migration := &r.App.Settings.Migration
			r.migration.checking = false
			migration.Checking = false
			migration.Open = false
			migration.Report = report
			if report != nil {
				migration.Status = report.Status
			}
			if err := errors.Join(detectErr, readErr); err != nil {
				migration.Status = migrate.Failed
				migration.Failure = "migration_failed"
				migration.FailureDetail = err.Error()
				r.syncMigrationAccountState()
				return
			}
			migration.Found = !state.Empty()
			migration.Counts = state.Counts
			migration.Failure = ""
			migration.FailureDetail = ""
			if marker != nil {
				migration.Status = marker.Status
				migration.ReportPath = marker.ReportPath
				if marker.Status == migrate.Failed {
					migration.Failure = "migration_failed"
					migration.FailureDetail = marker.Error
					r.syncMigrationAccountState()
					return
				}
				r.syncMigrationAccountState()
				return
			}
			r.syncMigrationAccountState()
			if autoStart && migration.Found {
				r.runMigration(false)
			}
		})
	})
}

func (r *Runtime) runMigration(fillMissing bool) {
	if r.migration == nil || r.migration.running {
		return
	}
	r.migration.running = true
	state := &r.App.Settings.Migration
	state.Running = true
	state.Failure = ""
	state.FailureDetail = ""
	state.Status = ""
	state.Progress = migrate.Progress{}
	state.Open = false
	r.syncMigrationAccountState()
	r.async(func() {
		lastStage := ""
		result, err := r.migration.service.Run(r.ctx, migrate.Options{
			FillMissing: fillMissing,
			Progress: func(progress migrate.Progress) {
				stageChanged := progress.Stage != lastStage
				lastStage = progress.Stage
				if !stageChanged && progress.Processed > 1 && progress.Processed%25 != 0 {
					return
				}
				r.enqueue(func(c *ui.Context) { r.App.Settings.Migration.Progress = progress })
			},
			ApplyPreferences: func(theme, language string) error {
				done := make(chan error, 1)
				r.enqueue(func(c *ui.Context) {
					applyErr := migrate.ApplyDesktopPreferences(r.Preferences, theme, language)
					if applyErr == nil {
						r.applyPreferences()
						if r.preferencesChanged != nil {
							r.preferencesChanged()
						}
					}
					done <- applyErr
				})
				select {
				case err := <-done:
					return err
				case <-r.ctx.Done():
					return r.ctx.Err()
				}
			},
		})
		var marker *migrate.Marker
		var report *migrate.Report
		var readErr error
		if err == nil {
			marker, report, readErr = r.migration.service.ReadState(r.ctx)
			_ = r.controller.Refresh(r.ctx)
			r.readTelemetry()
		} else {
			slog.Error("legacy migration failed", "error", err.Error())
			log.Printf("legacy migration failed: %v", err)
			marker, report, _ = r.migration.service.ReadState(r.ctx)
		}
		if readErr != nil {
			slog.Error("legacy migration state read failed", "error", readErr.Error())
			log.Printf("legacy migration state read failed: %v", readErr)
		}
		r.enqueue(func(c *ui.Context) {
			r.migration.running = false
			state := &r.App.Settings.Migration
			state.Running = false
			state.Open = false
			if err != nil {
				state.Status = migrate.Failed
				state.Failure = "migration_failed"
				state.FailureDetail = err.Error()
				state.Report = report
				r.syncMigrationAccountState()
				kit.Recover(c, kit.Failed, i18n.T(c, "migration_failed"), func() { r.runMigration(true) })
				return
			}
			if readErr != nil {
				state.Status = migrate.Failed
				state.Failure = "migration_failed"
				state.FailureDetail = readErr.Error()
				r.syncMigrationAccountState()
				kit.Recover(c, kit.Failed, i18n.T(c, "migration_failed"), func() { r.runMigration(true) })
				return
			}
			state.Failure = ""
			state.FailureDetail = ""
			state.Status = result.Status
			state.Counts = result.Counts
			state.Report = report
			state.ReportPath = result.ReportPath
			if marker != nil {
				state.Status = marker.Status
			}
			r.syncMigrationAccountState()
		})
	})
}
