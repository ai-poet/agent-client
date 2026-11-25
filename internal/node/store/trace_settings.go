package store

import (
	"context"
	"database/sql"
)

// TraceEnabled snapshots the profile's committed setting without taking a DB lease.
func (h *Handle) TraceEnabled() bool { return h != nil && h.traceEnabled.Load() }

// SetTraceEnabled serializes commits with publication of their in-memory state.
func (h *Handle) SetTraceEnabled(ctx context.Context, enabled bool) error {
	h.traceSettingsMu.Lock()
	defer h.traceSettingsMu.Unlock()
	err := h.Write(ctx, func(db *sql.Conn) error {
		_, err := db.ExecContext(ctx, "UPDATE node SET trace_enabled=? WHERE singleton=1", enabled)
		return err
	})
	if err == nil {
		h.traceEnabled.Store(enabled)
	}
	return err
}
