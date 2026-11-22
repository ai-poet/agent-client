package migrate

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preferences"
	"github.com/ai-poet/cheaprouter-harness/internal/node/agent/canonical"
	"github.com/ai-poet/cheaprouter-harness/internal/node/store"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/google/uuid"
)

type targetContext struct {
	db     *sql.Conn
	node   protocol.NodeID
	user   protocol.UserID
	report *Report
	result *Result
	opts   Options

	projects map[string]projectMapping
	sessions map[string]sessionMapping
	turns    map[string]protocol.TurnID
}

type projectMapping struct {
	id       protocol.ProjectID
	worktree protocol.WorktreeID
}

type sessionMapping struct {
	id       protocol.SessionID
	worktree protocol.WorktreeID
}

func run(ctx context.Context, legacyRoot, profile string, opts Options) (*Result, error) {
	if !opts.FillMissing {
		done, err := existingMarker(profile)
		if err != nil {
			return nil, err
		}
		if done {
			return &Result{Status: AlreadyDone, MarkerPath: filepath.Join(profile, MarkerFilename), ReportPath: filepath.Join(profile, ReportFilename)}, nil
		}
	}
	// This entry point is for isolated/profile-opening callers. A running Node
	// uses RunWithHandle so its existing writer and profile lock remain owners.
	storage, err := store.Open(ctx, profile, protocol.NodeID{1})
	if err != nil {
		openErr := fail("storage", "open Sailry profile", err)
		recordFailure(profile, legacyRoot, opts.LegacyDatabasePath, "", opts.clock(), openErr)
		return nil, openErr
	}
	defer storage.Close()
	return RunWithHandle(ctx, legacyRoot, profile, storage.Node, storage.NodeID, opts)
}

// RunWithHandle is the running Node's entry point. It performs all target
// mutations in the handle's existing writer transaction.
func RunWithHandle(ctx context.Context, legacyRoot, profile string, target *store.Handle, node protocol.NodeID, opts Options) (*Result, error) {
	if target == nil {
		err := fail("storage", "open Sailry profile", errors.New("missing target store"))
		recordFailure(profile, legacyRoot, opts.LegacyDatabasePath, "", opts.clock(), err)
		return nil, err
	}
	if opts.ApplyPreferences == nil {
		prefs := preferences.Open(filepath.Join(profile, "desktop", "preferences.json"))
		opts.ApplyPreferences = func(theme, language string) error {
			return ApplyDesktopPreferences(prefs, theme, language)
		}
	}
	if !opts.FillMissing {
		done, err := existingMarker(profile)
		if err != nil {
			recordFailure(profile, legacyRoot, opts.LegacyDatabasePath, "", opts.clock(), err)
			return nil, err
		}
		if done {
			return &Result{Status: AlreadyDone, MarkerPath: filepath.Join(profile, MarkerFilename), ReportPath: filepath.Join(profile, ReportFilename)}, nil
		}
	}
	sourceRoot := legacyRoot
	databasePath := opts.LegacyDatabasePath
	backupDir := ""
	failWith := func(err error) (*Result, error) {
		recordFailure(profile, sourceRoot, databasePath, backupDir, opts.clock(), err)
		return nil, err
	}
	state, err := detect(ctx, legacyRoot)
	if err != nil {
		return failWith(err)
	}
	sourceRoot = state.Root
	databasePath = state.DatabasePath
	if opts.LegacyDatabasePath != "" {
		overridePath, err := absoluteClean(opts.LegacyDatabasePath)
		if err != nil || filepath.Base(overridePath) != legacyDatabaseName {
			return failWith(fail("detect", "resolve legacy database override", errors.New("legacy database override must name app.db")))
		}
		databaseState, err := detect(ctx, filepath.Dir(overridePath))
		if err != nil {
			return failWith(err)
		}
		state.HasDatabase = databaseState.HasDatabase
		state.Counts = databaseState.Counts
		state.DatabasePath = overridePath
		databasePath = overridePath
	}
	started := opts.clock()
	backup, err := backupSources(ctx, state, profile, started)
	if err != nil {
		return failWith(err)
	}
	backupDir = backup
	progress(opts, "backup", 1)
	result := &Result{Status: Completed, BackupDir: backup, MarkerPath: filepath.Join(profile, MarkerFilename), ReportPath: filepath.Join(profile, ReportFilename)}
	report := &Report{Version: Version, Source: state.Root, DatabasePath: state.DatabasePath, StartedAt: started, Status: Completed, BackupDir: backup, Unmapped: []UnmappedField{}, Warnings: []Warning{}}

	if state.HasDatabase {
		if err := importDatabase(ctx, filepath.Join(backup, legacyDatabaseName), target, node, result, report, opts); err != nil {
			return failWith(err)
		}
	}
	if state.HasAppSettings || state.HasSettings {
		if err := importSettings(ctx, profile, backup, state.HasAppSettings, state.HasSettings, target, report, opts); err != nil {
			return failWith(err)
		}
	}
	if state.HasCredentials {
		// Read only allowlisted account metadata directly from the read-only legacy
		// directory; cloud-account.json is never copied into the backup directory.
		credentials, warnings, err := migrateCredentials(profile, filepath.Join(state.Root, legacyCredentialName))
		report.Credentials = credentials
		report.Warnings = append(report.Warnings, warnings...)
		if err != nil {
			return failWith(err)
		}
		progress(opts, "credentials", 1)
	}
	finishReport(result, report, opts)
	if err := writeJSONAtomic(result.ReportPath, report); err != nil {
		return failWith(fail("finalize", "write migration report", err))
	}
	marker := Marker{Version: Version, Source: state.Root, DatabasePath: state.DatabasePath, MigratedAt: report.CompletedAt, Status: result.Status, Counts: result.Counts, Skipped: result.Skipped, BackupDir: backup, ReportPath: result.ReportPath}
	if err := writeJSONAtomic(result.MarkerPath, marker); err != nil {
		return failWith(fail("finalize", "write migration marker", err))
	}
	progress(opts, "finalize", 1)
	return result, nil
}

// RecordFailure logs a migration error and writes a diagnostic failed marker
// to profile/migration.json without modifying legacyRoot.
func RecordFailure(profile, sourceRoot, databasePath, backupDir string, err error) {
	recordFailure(profile, sourceRoot, databasePath, backupDir, time.Now().UTC(), err)
}

func recordFailure(profile, sourceRoot, databasePath, backupDir string, at time.Time, err error) {
	if err == nil {
		return
	}
	stage := "unknown"
	var structured *Error
	if errors.As(err, &structured) && structured.Stage != "" {
		stage = structured.Stage
	}
	slog.Error("legacy migration failed", "stage", stage, "source", sourceRoot, "error", err.Error())
	if profile == "" {
		return
	}
	if sourceRoot != "" {
		if cleanSource, cleanErr := absoluteClean(sourceRoot); cleanErr == nil {
			if ensureSeparateRoots(cleanSource, profile) != nil {
				return
			}
			sourceRoot = cleanSource
		}
	}
	marker := Marker{
		Version:      Version,
		Source:       sourceRoot,
		DatabasePath: databasePath,
		MigratedAt:   at.UTC(),
		Status:       Failed,
		Stage:        stage,
		Error:        err.Error(),
		BackupDir:    backupDir,
	}
	if writeErr := writeJSONAtomic(filepath.Join(profile, MarkerFilename), marker); writeErr != nil {
		slog.Error("legacy migration failure marker write failed", "error", writeErr.Error())
	}
}

func importDatabase(ctx context.Context, databasePath string, target *store.Handle, node protocol.NodeID, result *Result, report *Report, opts Options) error {
	source, err := openLegacy(ctx, databasePath)
	if err != nil {
		return err
	}
	defer source.Close()
	tx, err := source.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return fail("detect", "start legacy snapshot", err)
	}
	defer tx.Rollback()
	data, err := readLegacy(ctx, tx)
	if err != nil {
		return err
	}
	unmapped, err := readUnmappedDetails(ctx, tx)
	if err != nil {
		return err
	}
	report.Unmapped = append(report.Unmapped, unmapped...)
	if err := tx.Commit(); err != nil {
		return fail("detect", "finish legacy snapshot", err)
	}
	return target.Write(ctx, func(db *sql.Conn) error {
		user, err := store.CurrentUser(ctx, db)
		if err != nil {
			return fail("storage", "resolve local user", err)
		}
		migration := &targetContext{db: db, node: node, user: user.ID, report: report, result: result, opts: opts, projects: map[string]projectMapping{}, sessions: map[string]sessionMapping{}, turns: map[string]protocol.TurnID{}}
		if err := migration.importProjects(ctx, data.projects); err != nil {
			return err
		}
		if err := migration.importSessions(ctx, data.sessions); err != nil {
			return err
		}
		return migration.importMessages(ctx, data.messages)
	})
}

func (m *targetContext) importProjects(ctx context.Context, projects []legacyProject) error {
	appearance, _ := json.Marshal(protocol.DefaultProjectsAppearance())
	for _, source := range projects {
		if err := ctx.Err(); err != nil {
			return fail("projects", "cancel", err)
		}
		id, err := mappedUUID("project", source.ID)
		if err != nil {
			return fail("projects", "map identifier", err)
		}
		projectID := protocol.ProjectID(id)
		if source.ID != projectID.String() {
			m.unmapped("projects", source.ID, "id", "invalid UUID replaced with a stable identifier")
		}
		name := strings.TrimSpace(source.Name)
		if name == "" {
			name = "Imported project"
			m.unmapped("projects", source.ID, "name", "empty name replaced")
		}
		path := filepath.Clean(strings.TrimSpace(source.Path))
		if path == "." || !filepath.IsAbs(path) {
			m.result.Skipped.Projects++
			m.unmapped("projects", source.ID, "path", "project path is not absolute")
			progress(m.opts, "projects", m.result.Counts.Projects+m.result.Skipped.Projects)
			continue
		}
		m.unmapped("projects", source.ID, "position", "Sailry uses its own project ordering")
		m.unmapped("projects", source.ID, "created_at", "Sailry projects do not store creation time")
		var existingID, existingPath string
		err = m.db.QueryRowContext(ctx, "SELECT id,path FROM projects WHERE id=?1 OR path=?2", projectID.String(), path).Scan(&existingID, &existingPath)
		if err == nil {
			parsed, parseErr := protocol.ParseProjectID(existingID)
			if parseErr != nil {
				return fail("projects", "read existing project", parseErr)
			}
			if existingID != projectID.String() {
				m.unmapped("projects", source.ID, "id", "path already exists; reused the existing project")
				m.unmapped("projects", source.ID, "name", "existing project metadata was preserved")
			} else if existingPath != path {
				m.unmapped("projects", source.ID, "path", "identifier already belongs to another project; source project was not merged")
				m.result.Skipped.Projects++
				progress(m.opts, "projects", m.result.Counts.Projects+m.result.Skipped.Projects)
				continue
			}
			m.result.Skipped.Projects++
			worktree, worktreeErr := m.mainWorktree(ctx, parsed)
			if worktreeErr != nil {
				return fail("projects", "read existing worktree", worktreeErr)
			}
			m.projects[source.ID] = projectMapping{id: parsed, worktree: worktree}
			progress(m.opts, "projects", m.result.Counts.Projects+m.result.Skipped.Projects)
			continue
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return fail("projects", "check existing project", err)
		}
		worktree := protocol.WorktreeID(stableUUID("worktree", source.ID))
		if _, err := m.db.ExecContext(ctx, "INSERT INTO projects(id,name,path,appearance,owner,visibility) VALUES(?1,?2,?3,?4,?5,'shared')", projectID.String(), name, path, appearance, m.user.String()); err != nil {
			return fail("projects", "insert project", err)
		}
		if _, err := m.db.ExecContext(ctx, "INSERT INTO worktrees(id,project,owner,path,main) VALUES(?1,?2,?3,?4,1)", worktree.String(), projectID.String(), m.user.String(), path); err != nil {
			return fail("projects", "insert main worktree", err)
		}
		m.projects[source.ID] = projectMapping{id: projectID, worktree: worktree}
		m.result.Counts.Projects++
		progress(m.opts, "projects", m.result.Counts.Projects+m.result.Skipped.Projects)
	}
	return nil
}

func (m *targetContext) mainWorktree(ctx context.Context, project protocol.ProjectID) (protocol.WorktreeID, error) {
	var id string
	err := m.db.QueryRowContext(ctx, "SELECT id FROM worktrees WHERE project=?1 AND main=1", project.String()).Scan(&id)
	if err != nil {
		return protocol.WorktreeID{}, err
	}
	return protocol.ParseWorktreeID(id)
}

func (m *targetContext) importSessions(ctx context.Context, sessions []legacySession) error {
	defaultConfig := protocol.DefaultSessionConfig()
	defaultConfig.Provider = protocol.ProviderID(stableUUID("provider", "legacy-history"))
	defaultConfig.Mode, defaultConfig.Permission = protocol.WorkModeCode, protocol.PermissionProject
	config, err := json.Marshal(defaultConfig)
	if err != nil {
		return fail("sessions", "encode historical config", err)
	}
	roles, err := json.Marshal(protocol.DefaultRoleSnapshot())
	if err != nil {
		return fail("sessions", "encode historical roles", err)
	}
	for _, source := range sessions {
		if err := ctx.Err(); err != nil {
			return fail("sessions", "cancel", err)
		}
		project, found := m.projects[source.ProjectID]
		if !found {
			m.result.Skipped.Sessions++
			m.unmapped("sessions", source.ID, "project_id", "referenced project was not migrated")
			progress(m.opts, "sessions", m.result.Counts.Sessions+m.result.Skipped.Sessions)
			continue
		}
		id := protocol.SessionID(mappedUUIDMust("session", source.ID))
		if source.ID != id.String() {
			m.unmapped("sessions", source.ID, "id", "invalid UUID replaced with a stable identifier")
		}
		var existingProject, existingWorktree string
		err := m.db.QueryRowContext(ctx, "SELECT project,worktree FROM sessions WHERE id=?1", id.String()).Scan(&existingProject, &existingWorktree)
		if err == nil {
			m.result.Skipped.Sessions++
			if existingProject != project.id.String() {
				m.unmapped("sessions", source.ID, "project_id", "existing session belongs to another project; messages were not merged")
				progress(m.opts, "sessions", m.result.Counts.Sessions+m.result.Skipped.Sessions)
				continue
			}
			worktree, parseErr := protocol.ParseWorktreeID(existingWorktree)
			if parseErr != nil {
				return fail("sessions", "read existing worktree", parseErr)
			}
			m.sessions[source.ID] = sessionMapping{id: id, worktree: worktree}
			progress(m.opts, "sessions", m.result.Counts.Sessions+m.result.Skipped.Sessions)
			continue
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return fail("sessions", "check existing session", err)
		}
		sequence, err := store.NextCounter(ctx, m.db, store.CounterSessions)
		if err != nil {
			return fail("sessions", "allocate session order", err)
		}
		if _, err := m.db.ExecContext(ctx, "INSERT INTO sessions(rowid,id,state,project,owner,worktree,revision,history_revision) VALUES(?1,?2,'active',?3,?4,?5,1,1)", sequence, id.String(), project.id.String(), m.user.String(), project.worktree.String()); err != nil {
			return fail("sessions", "insert session", err)
		}
		if _, err := m.db.ExecContext(ctx, "INSERT INTO session_revisions(session,revision,worktree,config,profile,roles) VALUES(?1,1,?2,?3,NULL,?4)", id.String(), project.worktree.String(), config, roles); err != nil {
			return fail("sessions", "insert session revision", err)
		}
		if _, err := m.db.ExecContext(ctx, "INSERT INTO session_media(session,body) VALUES(?1,'{}')", id.String()); err != nil {
			return fail("sessions", "insert session media", err)
		}
		title := strings.TrimSpace(source.Title)
		if (title == "" || title == "New task") && source.AutoTitle.Valid {
			title = strings.TrimSpace(source.AutoTitle.String)
		}
		if title != "" && title != "New task" {
			body, _ := json.Marshal(title)
			if _, err := m.db.ExecContext(ctx, "INSERT INTO agent_state(owner,key,body) VALUES(?1,'sailry.session_title',?2) ON CONFLICT(owner,key) DO NOTHING", id.String(), body); err != nil {
				return fail("sessions", "insert session title", err)
			}
		}
		m.sessions[source.ID] = sessionMapping{id: id, worktree: project.worktree}
		m.result.Counts.Sessions++
		for _, field := range []string{"provider", "model", "status", "created_at", "updated_at", "last_reply_at"} {
			m.unmapped("sessions", source.ID, field, "historical Sailry sessions do not retain this legacy column")
		}
		if source.AutoTitle.Valid && strings.TrimSpace(source.AutoTitle.String) != title {
			m.unmapped("sessions", source.ID, "auto_title", "only one title is retained")
		}
		progress(m.opts, "sessions", m.result.Counts.Sessions+m.result.Skipped.Sessions)
	}
	return nil
}

func (m *targetContext) importMessages(ctx context.Context, messages []legacyMessage) error {
	for _, source := range messages {
		if err := ctx.Err(); err != nil {
			return fail("messages", "cancel", err)
		}
		session, found := m.sessions[source.SessionID]
		if !found {
			m.result.Skipped.Messages++
			m.unmapped("messages", source.ID, "session_id", "referenced session was not migrated")
			progress(m.opts, "messages", m.result.Counts.Messages+m.result.Skipped.Messages)
			continue
		}
		var role canonical.Role
		switch source.Role {
		case "user":
			role = canonical.User
		case "assistant":
			role = canonical.Assistant
		default:
			m.result.Skipped.Messages++
			m.unmapped("messages", source.ID, "role", "Sailry canonical history has no equivalent role")
			progress(m.opts, "messages", m.result.Counts.Messages+m.result.Skipped.Messages)
			continue
		}
		id := stableOrOriginalUUID("message", source.ID)
		var exists bool
		if err := m.db.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM agent_events WHERE session=?1 AND id=?2)", session.id.String(), id.String()).Scan(&exists); err != nil {
			return fail("messages", "check existing message", err)
		}
		if exists {
			m.result.Skipped.Messages++
			progress(m.opts, "messages", m.result.Counts.Messages+m.result.Skipped.Messages)
			continue
		}
		turn, err := m.messageTurn(ctx, session, source)
		if err != nil {
			return err
		}
		content := source.Content
		if source.DisplayContent.Valid {
			content = source.DisplayContent.String
			if content != source.Content {
				m.unmapped("messages", source.ID, "content", "visible display_content is retained instead")
			}
		}
		message := canonical.NewMessage(role, canonical.Text(0, content))
		record := canonical.NewRecord(turn, "message", message, legacyTime(source.CreatedAt))
		record.ID = id.String()
		record.Provenance.Author = string(role)
		if _, _, err := store.AppendCanonical(ctx, m.db, session.id, record); err != nil {
			return fail("messages", "append canonical message", err)
		}
		m.result.Counts.Messages++
		if source.Attachments != "" && strings.TrimSpace(source.Attachments) != "[]" {
			m.unmapped("messages", source.ID, "attachments", "legacy attachment references have no imported binary mapping")
		}
		if source.Streaming {
			m.unmapped("messages", source.ID, "streaming", "imported history is always settled")
		}
		progress(m.opts, "messages", m.result.Counts.Messages+m.result.Skipped.Messages)
	}
	return nil
}

func (m *targetContext) messageTurn(ctx context.Context, session sessionMapping, source legacyMessage) (protocol.TurnID, error) {
	key := source.ID
	if source.TurnID.Valid && strings.TrimSpace(source.TurnID.String) != "" {
		key = source.TurnID.String
	}
	turnKey := source.SessionID + "\x00" + key
	if turn, found := m.turns[turnKey]; found {
		return turn, nil
	}
	turn := protocol.TurnID(stableUUID("turn", turnKey))
	var owner string
	err := m.db.QueryRowContext(ctx, "SELECT session FROM turns WHERE id=?1", turn.String()).Scan(&owner)
	if err == nil {
		if owner != session.id.String() {
			return protocol.TurnID{}, fail("messages", "check existing turn", errors.New("historical turn belongs to another session"))
		}
		m.turns[turnKey] = turn
		return turn, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return protocol.TurnID{}, fail("messages", "check existing turn", err)
	}
	request := protocol.RequestID(stableUUID("request", turnKey))
	created := legacyTime(source.CreatedAt)
	ledger, _ := json.Marshal(protocol.Request{Version: 1, ID: request, Target: m.node, Command: protocol.Command{Kind: "snapshot"}})
	if _, err := m.db.ExecContext(ctx, "INSERT INTO requests(caller,id,body,status,result,at) VALUES(?1,?2,?3,'unknown',NULL,?4) ON CONFLICT(caller,id) DO NOTHING", m.node[:], request.String(), ledger, created.Unix()); err != nil {
		return protocol.TurnID{}, fail("messages", "insert historical request", err)
	}
	sequence, err := store.NextCounter(ctx, m.db, store.CounterTurns)
	if err != nil {
		return protocol.TurnID{}, fail("messages", "allocate turn order", err)
	}
	plugins, _ := json.Marshal([]string{})
	roles, _ := json.Marshal(protocol.DefaultRoleSnapshot())
	if _, err := m.db.ExecContext(ctx, "INSERT INTO turns(rowid,id,kind,plugins,roles,instructions,session,revision,request,caller,user) VALUES(?1,?2,'task',?3,?4,?5,?6,1,?7,?8,?9)", sequence, turn.String(), plugins, roles, []byte("[]"), session.id.String(), request.String(), m.node[:], m.user.String()); err != nil {
		return protocol.TurnID{}, fail("messages", "insert historical turn", err)
	}
	milliseconds := created.UnixMilli()
	run := protocol.DefaultConversationRun()
	run.Worktree, run.Turn, run.Kind, run.Session = session.worktree, turn, protocol.ConversationRunKindTask, session.id
	run.Sequence, run.Revision, run.Status = sequence, 1, protocol.ConversationStatusCompleted
	run.StartedMs, run.FinishedMs = &milliseconds, &milliseconds
	body, _ := json.Marshal(run)
	hooks, _ := json.Marshal(protocol.DefaultHooksSettings())
	if _, err := m.db.ExecContext(ctx, "INSERT INTO agent_runs(turn,ready,body,provider,hooks,compaction_percent,input,media) VALUES(?1,0,?2,NULL,?3,80,NULL,NULL)", turn.String(), body, hooks); err != nil {
		return protocol.TurnID{}, fail("messages", "insert historical run", err)
	}
	if _, err := m.db.ExecContext(ctx, "INSERT INTO conversation_turns(session,turn) VALUES(?1,?2)", session.id.String(), turn.String()); err != nil {
		return protocol.TurnID{}, fail("messages", "insert conversation turn", err)
	}
	m.turns[turnKey] = turn
	return turn, nil
}

func (m *targetContext) unmapped(entity, id, field, reason string) {
	m.report.Unmapped = append(m.report.Unmapped, UnmappedField{Entity: entity, ID: id, Field: field, Reason: reason})
}

func mappedUUID(kind, source string) (uuid.UUID, error) {
	if source == "" {
		return uuid.Nil, errors.New("empty legacy identifier")
	}
	return stableOrOriginalUUID(kind, source), nil
}

func mappedUUIDMust(kind, source string) uuid.UUID {
	return stableOrOriginalUUID(kind, source)
}

func stableOrOriginalUUID(kind, source string) uuid.UUID {
	if parsed, err := uuid.Parse(source); err == nil && parsed != uuid.Nil {
		return parsed
	}
	return stableUUID(kind, source)
}

func stableUUID(kind, source string) uuid.UUID {
	return uuid.NewSHA1(uuid.NameSpaceURL, []byte("cheaprouter-migration-v1\x00"+kind+"\x00"+source))
}

func legacyTime(seconds int64) time.Time {
	if seconds < 0 {
		seconds = 0
	}
	return time.Unix(seconds, 0).UTC()
}

func progress(opts Options, stage string, processed int) {
	if opts.Progress != nil {
		opts.Progress(Progress{Stage: stage, Processed: processed})
	}
}

func finishReport(result *Result, report *Report, opts Options) {
	report.Counts, report.Skipped = result.Counts, result.Skipped
	if len(report.Unmapped) > 0 || len(report.Warnings) > 0 {
		result.Status = Partial
	}
	report.Status = result.Status
	report.CompletedAt = opts.clock()
	result.WarningCount = len(report.Unmapped) + len(report.Warnings)
}

func importSettings(ctx context.Context, profile, root string, hasAppSettings, hasSettings bool, target *store.Handle, report *Report, opts Options) error {
	values, unmapped, warnings := migrateLegacySettings(root, hasAppSettings, hasSettings)
	report.Unmapped = append(report.Unmapped, unmapped...)
	report.Warnings = append(report.Warnings, warnings...)
	if enabled, found := values["analytics_enabled"].(bool); found {
		if err := target.Write(ctx, func(db *sql.Conn) error {
			current, err := store.ReadTelemetry(ctx, db)
			if err != nil {
				return err
			}
			if current.Revision != 0 {
				report.Unmapped = append(report.Unmapped, UnmappedField{Entity: "settings", Field: "analytics_enabled", Reason: "Sailry telemetry preferences were already changed"})
				return nil
			}
			current.Errors, current.Metrics = enabled, enabled
			current.Revision++
			body, err := json.Marshal(current)
			if err != nil {
				return err
			}
			_, err = db.ExecContext(ctx, "UPDATE telemetry_settings SET body=?1 WHERE singleton=1", body)
			return err
		}); err != nil {
			return fail("settings", "save telemetry preference", err)
		}
	}
	theme, _ := values["theme"].(string)
	language, _ := values["language"].(string)
	if opts.ApplyPreferences != nil && (theme != "" || language != "") {
		if err := opts.ApplyPreferences(theme, language); err != nil {
			report.Warnings = append(report.Warnings, Warning{Stage: "settings", Message: "desktop theme or language could not be saved; use fill missing to retry"})
		}
	}
	progress(opts, "settings", 1)
	return nil
}

// ApplyDesktopPreferences fills only untouched Desktop choices. It runs on the
// preference owner's UI thread when a live Runtime supplied the callback.
func ApplyDesktopPreferences(prefs *preferences.Store, theme, language string) error {
	if prefs == nil {
		return errors.New("missing Desktop preferences")
	}
	changed := false
	if theme != "" && prefs.Data.Appearance.Mode == "system" && theme != "system" {
		prefs.Data.Appearance.Mode = theme
		changed = true
	}
	if language != "" && prefs.Data.Language == nil {
		value := language
		prefs.Data.Language = &value
		changed = true
	}
	if !changed {
		return nil
	}
	return prefs.Save()
}

// ReadMarker loads the completion marker, or nil when migration has not run.
func ReadMarker(profile string) (*Marker, error) {
	raw, err := os.ReadFile(filepath.Join(profile, MarkerFilename))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fail("detect", "read migration marker", err)
	}
	var marker Marker
	if err := json.Unmarshal(raw, &marker); err != nil || marker.Version <= 0 {
		return nil, fail("detect", "decode migration marker", errors.New("invalid migration marker"))
	}
	return &marker, nil
}

// ReadReport loads a completed or partial report for the settings page.
func ReadReport(profile string) (*Report, error) {
	raw, err := os.ReadFile(filepath.Join(profile, ReportFilename))
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, fail("detect", "read migration report", err)
	}
	var report Report
	if err := json.Unmarshal(raw, &report); err != nil {
		return nil, fail("detect", "decode migration report", err)
	}
	return &report, nil
}

// DefaultLegacyRoot resolves the previous client's directory. The build-time
// Rust override cannot be read by Go, so the same environment variable is
// honored for source builds and managed installations.
func DefaultLegacyRoot() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	name := strings.TrimSpace(os.Getenv("SUB2API_DATA_DIR_NAME"))
	if name == "" {
		name = ".cheaprouter"
	}
	if filepath.IsAbs(name) {
		return filepath.Clean(name), nil
	}
	return filepath.Join(home, name), nil
}

// LegacyApplicationSupportDatabase is the Release daemon's historical
// StateStore::default_path. Settings and cloud-account.json remain in the
// DATA_DIR_NAME root returned by DefaultLegacyRoot.
func LegacyApplicationSupportDatabase() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	brand := strings.TrimSpace(os.Getenv("SUB2API_BRAND_NAME"))
	if brand == "" {
		brand = "CheapRouter"
	}
	var directory string
	switch runtime.GOOS {
	case "darwin":
		directory = filepath.Join(home, "Library", "Application Support", brand)
	case "windows":
		directory = os.Getenv("LOCALAPPDATA")
		if directory == "" {
			directory = filepath.Join(home, "AppData", "Local")
		}
		directory = filepath.Join(directory, brand)
	default:
		directory = os.Getenv("XDG_DATA_HOME")
		if directory == "" || !filepath.IsAbs(directory) {
			directory = filepath.Join(home, ".local", "share")
		}
		directory = filepath.Join(directory, brand)
	}
	return filepath.Join(directory, legacyDatabaseName), nil
}
