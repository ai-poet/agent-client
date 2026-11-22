package settings

import (
	"fmt"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/ai-poet/cheaprouter-harness/internal/migrate"
	"github.com/egoist/mygo/ui"
)

// MigrationState is shared by silent startup migration and Settings › General.
type MigrationState struct {
	Known         bool
	Open          bool
	ReportOpen    bool
	Checking      bool
	Running       bool
	Found         bool
	Counts        migrate.Counts
	Status        migrate.Status
	Progress      migrate.Progress
	Report        *migrate.Report
	ReportPath    string
	Failure       string
	FailureDetail string
}

// MigrationActions run against the local profile only.
type MigrationActions struct {
	Run func(fillMissing bool)
}

func migrationStatusKey(status migrate.Status) string {
	switch status {
	case migrate.Completed:
		return "migration_status_completed"
	case migrate.Partial:
		return "migration_status_partial"
	case migrate.AlreadyDone:
		return "migration_status_completed"
	case migrate.Failed:
		return "migration_failed"
	default:
		return "migration_status_pending"
	}
}

func migration(c *ui.Context, m *Model) {
	state := &m.Migration
	if !state.Known && m.MigrationActions == nil {
		return
	}
	p := theme.Of(c)
	kit.Group(c, "legacy-migration", i18n.T(c, "migration_title"), "", nil, func() {
		kit.Card(c, func() {
			status := state.Status
			if state.Failure != "" {
				status = migrate.Status("failed")
			}
			title := i18n.T(c, migrationStatusKey(status))
			if state.Checking {
				title = i18n.T(c, "migration_checking")
			} else if state.Failure != "" {
				title = i18n.T(c, "migration_failed")
			} else if !state.Found && status == "" {
				title = i18n.T(c, "migration_not_found")
			}
			kit.Row(c, "status", title, i18n.T(c, "migration_preserves_source"), func() {
				if state.Report != nil {
					kit.Secondary(c.Key("report"), i18n.T(c, "migration_view_report")).Height(28).OnClick(func() { state.ReportOpen = true })
				}
			})
			if state.Found {
				ui.Text(c, i18n.T(c, "migration_counts", "projects", state.Counts.Projects, "sessions", state.Counts.Sessions, "messages", state.Counts.Messages)).FontSize(13).TextColor(p.MutedForeground)
			}
			if state.Failure != "" {
				ui.Text(c, i18n.T(c, "migration_failed_retry")).FontSize(13).TextColor(p.Danger)
				if state.FailureDetail != "" {
					ui.Text(c, state.FailureDetail).FontSize(12).Selectable().TextColor(p.Danger)
				}
			}
			if state.Found || state.Failure != "" {
				kit.Row(c, "retry", i18n.T(c, "migration_fill_missing"), i18n.T(c, "migration_fill_missing_description"), func() {
					disabled := state.Running || state.Checking || m.MigrationActions == nil
					kit.SecondaryBusy(c.Key("run"), i18n.T(c, "migration_retry"), state.Running).Disabled(disabled).Height(28).OnClick(func() {
						if m.MigrationActions != nil {
							m.MigrationActions.Run(true)
						}
					})
				})
			}
		})
	})
}

// MigrationView is the startup wizard. Closing it defers migration; it neither
// writes a marker nor changes the separate welcome guide.
func MigrationView(c *ui.Context, m *Model) {
	state := &m.Migration
	if !state.Open {
		return
	}
	p := theme.Of(c)
	w, h := c.Size()
	title := i18n.T(c, "migration_title")
	kit.Dialog(c.Key("legacy-migration"), &state.Open, func() {
		ui.Text(c, title).FontSize(22).FontWeight(600)
		if state.Checking {
			ui.Text(c, i18n.T(c, "migration_checking")).FontSize(14)
			return
		}
		if state.Failure != "" {
			ui.Text(c, i18n.T(c, "migration_failed_retry")).FontSize(14).TextColor(p.Danger)
			ui.Row(c.Key("actions")).Gap(8).Justify(ui.End).Children(func() {
				kit.Secondary(c.Key("later"), i18n.T(c, "migration_later")).OnClick(func() { state.Open = false })
				kit.PrimaryBusy(c.Key("retry"), i18n.T(c, "migration_retry"), state.Running, state.Running).OnClick(func() {
					if m.MigrationActions != nil {
						m.MigrationActions.Run(true)
					}
				})
			})
			return
		}
		if state.Running {
			ui.Text(c, i18n.T(c, "migration_running", "stage", state.Progress.Stage, "count", state.Progress.Processed)).FontSize(14)
			return
		}
		if state.Status != "" {
			ui.Text(c, i18n.T(c, migrationStatusKey(state.Status))).FontSize(16).FontWeight(500)
			ui.Text(c, i18n.T(c, "migration_result_counts", "projects", state.ReportCounts().Projects, "sessions", state.ReportCounts().Sessions, "messages", state.ReportCounts().Messages)).FontSize(14)
			if state.Status == migrate.Partial {
				ui.Text(c, i18n.T(c, "migration_partial_description")).FontSize(13).TextColor(p.MutedForeground)
			}
			ui.Row(c.Key("actions")).Gap(8).Justify(ui.End).Children(func() {
				if state.Report != nil {
					kit.Secondary(c.Key("report"), i18n.T(c, "migration_view_report")).OnClick(func() { state.ReportOpen = true })
				}
				kit.Primary(c.Key("done"), i18n.T(c, "migration_done"), false).OnClick(func() { state.Open = false })
			})
			return
		}
		ui.Text(c, i18n.T(c, "migration_description")).FontSize(14)
		ui.Text(c, i18n.T(c, "migration_counts", "projects", state.Counts.Projects, "sessions", state.Counts.Sessions, "messages", state.Counts.Messages)).FontSize(16).FontWeight(500)
		ui.Text(c, i18n.T(c, "migration_preserves_source")).FontSize(13).TextColor(p.MutedForeground)
		ui.Text(c, i18n.T(c, "migration_credentials_note")).FontSize(13).TextColor(p.MutedForeground)
		ui.Row(c.Key("actions")).Gap(8).Justify(ui.End).Children(func() {
			kit.Secondary(c.Key("later"), i18n.T(c, "migration_later")).OnClick(func() { state.Open = false })
			kit.Primary(c.Key("confirm"), i18n.T(c, "migration_confirm"), false).OnClick(func() {
				if m.MigrationActions != nil {
					m.MigrationActions.Run(false)
				}
			})
		})
	}).Width(min(560, max(w-48, 0))).MaxHeight(h * 0.85).Label(title)
}

// ReportCounts returns migrated counts even before the report has loaded.
func (s *MigrationState) ReportCounts() migrate.Counts {
	if s.Report != nil {
		return s.Report.Counts
	}
	return s.Counts
}

// MigrationReport displays a bounded, read-only report without field values.
func MigrationReport(c *ui.Context, m *Model) {
	state := &m.Migration
	if !state.ReportOpen || state.Report == nil {
		return
	}
	w, h := c.Size()
	title := i18n.T(c, "migration_report_title")
	kit.Dialog(c.Key("legacy-migration-report"), &state.ReportOpen, func() {
		ui.Text(c, title).FontSize(20).FontWeight(600)
		ui.Text(c, state.ReportPath).FontSize(12).Selectable().TextColor(theme.Of(c).MutedForeground)
		ui.Scroll(c.Key("entries")).Grow(1).MinHeight(0).Children(func() {
			for _, warning := range state.Report.Warnings {
				ui.Text(c, fmt.Sprintf("%s: %s", warning.Stage, warning.Message)).FontSize(13).Padding(4, 0)
			}
			for _, field := range state.Report.Unmapped {
				label := field.Entity + "." + field.Field
				if field.ID != "" {
					label += " · " + field.ID
				}
				ui.Text(c, label).FontSize(13).FontWeight(500).Padding(4, 0, 0, 0)
				ui.Text(c, field.Reason).FontSize(12).TextColor(theme.Of(c).MutedForeground)
			}
			if len(state.Report.Warnings) == 0 && len(state.Report.Unmapped) == 0 {
				ui.Text(c, i18n.T(c, "migration_report_empty")).FontSize(14)
			}
		})
		ui.Row(c.Key("actions")).Justify(ui.End).Children(func() {
			kit.Primary(c.Key("close"), i18n.T(c, "migration_done"), false).OnClick(func() { state.ReportOpen = false })
		})
	}).Width(min(680, max(w-48, 0))).Height(min(560, max(h-96, 0))).Label(title)
}
