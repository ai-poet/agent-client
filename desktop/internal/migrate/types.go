// Package migrate imports the previous CheapRouter desktop's local data into
// a Sailry profile without modifying the legacy installation.
package migrate

import (
	"context"
	"fmt"
	"time"
)

const (
	// Version is the migration document and field-mapping version.
	Version = 1

	MarkerFilename       = "migration.json"
	ReportFilename       = "migration-report.json"
	AccountFilename      = "cheaprouter-legacy-account.json"
	legacyDatabaseName   = "app.db"
	legacySettingsName   = "settings.json"
	legacyCredentialName = "cloud-account.json"
)

// Status describes the durable outcome of a migration run.
type Status string

const (
	Completed   Status = "completed"
	Partial     Status = "partial"
	AlreadyDone Status = "already_done"
)

// Counts contains source counts for detection and inserted counts for a run.
type Counts struct {
	Projects int `json:"projects"`
	Sessions int `json:"sessions"`
	Messages int `json:"messages"`
}

// LegacyState is a read-only inventory of the previous desktop's data.
type LegacyState struct {
	Root           string `json:"root"`
	DatabasePath   string `json:"database_path,omitempty"`
	HasDatabase    bool   `json:"has_database"`
	HasSettings    bool   `json:"has_settings"`
	HasCredentials bool   `json:"has_credentials"`
	Counts         Counts `json:"counts"`
}

// Empty reports whether there is anything to offer to the migration wizard.
func (s *LegacyState) Empty() bool {
	return s == nil || !s.HasDatabase && !s.HasSettings && !s.HasCredentials
}

// Progress reports completed work. Processed is cumulative within Stage.
type Progress struct {
	Stage     string `json:"stage"`
	Processed int    `json:"processed"`
}

// Options controls a migration run. The zero value performs a first run and
// returns AlreadyDone when a marker exists. FillMissing is reserved for the
// explicit settings-page retry: it inserts absent IDs but never updates rows.
type Options struct {
	Progress    func(Progress)
	FillMissing bool
	// ApplyPreferences runs on the Desktop preference owner's UI thread. Nil
	// uses an isolated Store at the profile's standard preferences path.
	ApplyPreferences func(theme, language string) error
	// LegacyDatabasePath supports Release installs whose daemon database was
	// beside the platform application-support directory instead of Root.
	LegacyDatabasePath string

	now func() time.Time
}

// Result summarizes one run. AlreadyDone results change no files.
type Result struct {
	Status       Status `json:"status"`
	Counts       Counts `json:"counts"`
	Skipped      Counts `json:"skipped"`
	BackupDir    string `json:"backup_dir,omitempty"`
	ReportPath   string `json:"report_path,omitempty"`
	MarkerPath   string `json:"marker_path,omitempty"`
	WarningCount int    `json:"warning_count"`
}

// Marker is written only after a complete or partially mapped run commits.
type Marker struct {
	Version      int       `json:"version"`
	Source       string    `json:"source"`
	DatabasePath string    `json:"database_path,omitempty"`
	MigratedAt   time.Time `json:"migrated_at"`
	Status       Status    `json:"status"`
	Counts       Counts    `json:"counts"`
	Skipped      Counts    `json:"skipped"`
	BackupDir    string    `json:"backup_dir"`
	ReportPath   string    `json:"report_path"`
}

// UnmappedField records a source field that has no lossless target mapping.
// Values are deliberately omitted: they can contain private transcript data.
type UnmappedField struct {
	Entity string `json:"entity"`
	ID     string `json:"id,omitempty"`
	Field  string `json:"field"`
	Reason string `json:"reason"`
}

// Warning is a non-fatal, structured migration issue.
type Warning struct {
	Stage   string `json:"stage"`
	Message string `json:"message"`
}

// CredentialReport documents the non-secret account metadata mapping.
type CredentialReport struct {
	Found           bool   `json:"found"`
	Migrated        bool   `json:"migrated"`
	AccountID       string `json:"account_id,omitempty"`
	Endpoint        string `json:"endpoint,omitempty"`
	Destination     string `json:"destination,omitempty"`
	ReloginRequired bool   `json:"relogin_required"`
	SecretsMigrated bool   `json:"secrets_migrated"`
}

// Report is safe to display in the UI. It contains counts, field names and
// account metadata, never transcript bodies or secret credential values.
type Report struct {
	Version      int              `json:"version"`
	Source       string           `json:"source"`
	DatabasePath string           `json:"database_path,omitempty"`
	StartedAt    time.Time        `json:"started_at"`
	CompletedAt  time.Time        `json:"completed_at"`
	Status       Status           `json:"status"`
	Counts       Counts           `json:"counts"`
	Skipped      Counts           `json:"skipped"`
	BackupDir    string           `json:"backup_dir"`
	Unmapped     []UnmappedField  `json:"unmapped"`
	Warnings     []Warning        `json:"warnings"`
	Credentials  CredentialReport `json:"credentials"`
}

// Error identifies the failed migration stage without requiring the UI to
// parse an error string. Callers should allow startup to continue.
type Error struct {
	Stage string
	Op    string
	Err   error
}

func (e *Error) Error() string {
	if e == nil {
		return "migration failed"
	}
	if e.Err == nil {
		return fmt.Sprintf("migration %s: %s", e.Stage, e.Op)
	}
	return fmt.Sprintf("migration %s: %s: %v", e.Stage, e.Op, e.Err)
}

func (e *Error) Unwrap() error { return e.Err }

func fail(stage, op string, err error) error {
	return &Error{Stage: stage, Op: op, Err: err}
}

// Run migrates legacy data into profile. See Options for retry semantics.
func Run(ctx context.Context, legacyRoot, profile string, opts Options) (*Result, error) {
	return run(ctx, legacyRoot, profile, opts)
}

func (o Options) clock() time.Time {
	if o.now != nil {
		return o.now().UTC()
	}
	return time.Now().UTC()
}
