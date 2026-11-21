package migrate

import (
	"context"
	"database/sql"
	"encoding/json"
	"sort"
	"strings"
)

type legacyProject struct {
	ID        string
	Name      string
	Path      string
	Position  int64
	CreatedAt int64
}

type legacySession struct {
	ID          string
	ProjectID   string
	Title       string
	AutoTitle   sql.NullString
	Provider    string
	Model       sql.NullString
	Status      string
	CreatedAt   int64
	UpdatedAt   int64
	LastReplyAt sql.NullInt64
}

type legacyMessage struct {
	ID             string
	SessionID      string
	TurnID         sql.NullString
	Position       int64
	Role           string
	Content        string
	DisplayContent sql.NullString
	Attachments    string
	CreatedAt      int64
	Streaming      bool
}

type legacyData struct {
	projects []legacyProject
	sessions []legacySession
	messages []legacyMessage
}

type legacyQuerier interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
}

func readLegacy(ctx context.Context, db legacyQuerier) (legacyData, error) {
	var data legacyData
	projects, err := db.QueryContext(ctx, "SELECT id,name,path,position,created_at FROM projects ORDER BY position,id")
	if err != nil {
		return data, fail("projects", "read legacy projects", err)
	}
	if err := scanRows(projects, func() error {
		var value legacyProject
		if err := projects.Scan(&value.ID, &value.Name, &value.Path, &value.Position, &value.CreatedAt); err != nil {
			return err
		}
		data.projects = append(data.projects, value)
		return nil
	}); err != nil {
		return data, fail("projects", "decode legacy projects", err)
	}

	sessions, err := db.QueryContext(ctx, "SELECT id,project_id,title,auto_title,provider,model,status,created_at,updated_at,last_reply_at FROM sessions ORDER BY created_at,id")
	if err != nil {
		return data, fail("sessions", "read legacy sessions", err)
	}
	if err := scanRows(sessions, func() error {
		var value legacySession
		if err := sessions.Scan(&value.ID, &value.ProjectID, &value.Title, &value.AutoTitle, &value.Provider, &value.Model, &value.Status, &value.CreatedAt, &value.UpdatedAt, &value.LastReplyAt); err != nil {
			return err
		}
		data.sessions = append(data.sessions, value)
		return nil
	}); err != nil {
		return data, fail("sessions", "decode legacy sessions", err)
	}

	messages, err := db.QueryContext(ctx, "SELECT id,session_id,turn_id,position,role,content,display_content,attachments,created_at,streaming FROM messages ORDER BY session_id,position,id")
	if err != nil {
		return data, fail("messages", "read legacy messages", err)
	}
	if err := scanRows(messages, func() error {
		var value legacyMessage
		if err := messages.Scan(&value.ID, &value.SessionID, &value.TurnID, &value.Position, &value.Role, &value.Content, &value.DisplayContent, &value.Attachments, &value.CreatedAt, &value.Streaming); err != nil {
			return err
		}
		data.messages = append(data.messages, value)
		return nil
	}); err != nil {
		return data, fail("messages", "decode legacy messages", err)
	}
	return data, nil
}

func scanRows(rows *sql.Rows, scan func() error) error {
	defer rows.Close()
	for rows.Next() {
		if err := scan(); err != nil {
			return err
		}
	}
	return rows.Err()
}

func readUnmappedDetails(ctx context.Context, db legacyQuerier) ([]UnmappedField, error) {
	rows, err := db.QueryContext(ctx, "SELECT session_id,data FROM session_details ORDER BY session_id")
	if err != nil {
		return nil, fail("sessions", "read legacy session details", err)
	}
	defer rows.Close()
	fields := []UnmappedField{}
	for rows.Next() {
		var id, raw string
		if err := rows.Scan(&id, &raw); err != nil {
			return nil, fail("sessions", "decode legacy session details", err)
		}
		var detail map[string]json.RawMessage
		if err := json.Unmarshal([]byte(raw), &detail); err != nil {
			fields = append(fields, UnmappedField{Entity: "session_details", ID: id, Field: "data", Reason: "invalid session detail JSON"})
			continue
		}
		for _, name := range []string{"workspace", "runtime_mode", "interaction_mode", "reasoning_effort", "service_tier", "context_window", "agent_preset", "provider_cursor", "available_commands", "thread_goal", "todos", "context_usage", "runtime_event_cursor", "provider_session_id", "transcript_blocks", "turns", "queued_messages"} {
			value, found := detail[name]
			if !found || isEmptyJSON(value) {
				continue
			}
			fields = append(fields, UnmappedField{Entity: "session_details", ID: id, Field: name, Reason: "no lossless Sailry history mapping"})
		}
		known := map[string]bool{}
		for _, name := range []string{"id", "title", "auto_title", "project_id", "workspace", "provider", "model", "runtime_mode", "interaction_mode", "reasoning_effort", "service_tier", "context_window", "agent_preset", "status", "created_at", "updated_at", "last_reply_at", "provider_cursor", "available_commands", "thread_goal", "todos", "context_usage", "runtime_event_cursor", "provider_session_id", "messages", "transcript_blocks", "turns", "queued_messages", "detail_loaded"} {
			known[name] = true
		}
		unknown := []string{}
		for name, value := range detail {
			if !known[name] && !isEmptyJSON(value) {
				unknown = append(unknown, name)
			}
		}
		sort.Strings(unknown)
		for _, name := range unknown {
			fields = append(fields, UnmappedField{Entity: "session_details", ID: id, Field: name, Reason: "unknown legacy session detail field"})
		}
	}
	return fields, rows.Err()
}

func isEmptyJSON(raw json.RawMessage) bool {
	text := strings.TrimSpace(string(raw))
	return text == "" || text == "null" || text == `""` || text == "[]" || text == "{}" || text == "false" || text == "0"
}
