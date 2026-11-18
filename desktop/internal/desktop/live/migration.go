package live

import (
	"context"

	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/migrate"
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
// read-only state check. Only promptFirst may open the startup wizard.
func (r *Runtime) SetMigrationService(service MigrationService, promptFirst bool) {
	if service == nil {
		return
	}
	r.migration = &migrationRuntime{service: service}
	r.App.Settings.MigrationActions = &settings.MigrationActions{Run: r.runMigration}
	r.checkMigration(promptFirst)
}

func (r *Runtime) checkMigration(prompt bool) {
	if r.migration == nil || r.migration.checking || r.migration.running {
		return
	}
	r.migration.checking = true
	r.App.Settings.Migration.Checking = true
	r.App.Settings.Migration.Known = true
	r.async(func() {
		state, detectErr := r.migration.service.Detect(r.ctx)
		marker, report, readErr := r.migration.service.ReadState(r.ctx)
		r.enqueue(func(c *ui.Context) {
			migration := &r.App.Settings.Migration
			r.migration.checking = false
			migration.Checking = false
			migration.Report = report
			if report != nil {
				migration.Status = report.Status
			}
			if detectErr != nil || readErr != nil {
				migration.Failure = "migration_failed"
				migration.Open = prompt
				return
			}
			migration.Found = !state.Empty()
			migration.Counts = state.Counts
			migration.Failure = ""
			if marker != nil {
				migration.Status = marker.Status
				migration.ReportPath = marker.ReportPath
			}
			migration.Open = prompt && migration.Found && marker == nil
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
	state.Status = ""
	state.Progress = migrate.Progress{}
	state.Open = true
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
					done <- migrate.ApplyDesktopPreferences(r.Preferences, theme, language)
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
		}
		r.enqueue(func(c *ui.Context) {
			r.migration.running = false
			state := &r.App.Settings.Migration
			state.Running = false
			if err != nil {
				state.Failure = "migration_failed"
				state.Open = true
				return
			}
			if readErr != nil {
				state.Failure = "migration_failed"
				state.Open = true
				return
			}
			state.Failure = ""
			state.Status = result.Status
			state.Counts = result.Counts
			state.Report = report
			state.ReportPath = result.ReportPath
			if marker != nil {
				state.Status = marker.Status
			}
			state.Open = true
		})
	})
}
