package store

import (
	"context"
	"database/sql"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// TraceEnabled snapshots the profile's committed setting without taking a DB lease.
func (h *Handle) TraceEnabled() bool { return h != nil && h.traceEnabled.Load() }

func (h *Handle) TraceGatewayEnabled() bool { return h != nil && h.traceGatewayEnabled.Load() }

func ReadTraceSettings(ctx context.Context, db *sql.Conn) (protocol.TraceSettings, error) {
	var value protocol.TraceSettings
	err := db.QueryRowContext(ctx, "SELECT trace_enabled,trace_gateway_enabled,trace_revision FROM node WHERE singleton=1").Scan(&value.Enabled, &value.GatewayEnabled, &value.Revision)
	return value, err
}

// SetTraceEnabled serializes commits with publication of their in-memory state.
func (h *Handle) SetTraceEnabled(ctx context.Context, enabled bool) error {
	_, err := h.updateTraceSettings(ctx, func(current *protocol.TraceSettings) error {
		current.Enabled = enabled
		return nil
	})
	return err
}

func (h *Handle) SaveTraceSettings(ctx context.Context, draft protocol.TraceSettings) (protocol.TraceSettings, error) {
	return h.updateTraceSettings(ctx, func(current *protocol.TraceSettings) error {
		if err := checkRevision(current.Revision, draft.Revision); err != nil {
			return err
		}
		*current = draft
		return nil
	})
}

func (h *Handle) updateTraceSettings(ctx context.Context, edit func(*protocol.TraceSettings) error) (protocol.TraceSettings, error) {
	h.traceSettingsMu.Lock()
	defer h.traceSettingsMu.Unlock()
	var value protocol.TraceSettings
	err := h.Write(ctx, func(db *sql.Conn) error {
		var err error
		value, err = ReadTraceSettings(ctx, db)
		if err != nil {
			return err
		}
		if err = edit(&value); err != nil {
			return err
		}
		value.Revision, err = nextRevision(value.Revision)
		if err != nil {
			return err
		}
		_, err = db.ExecContext(ctx, "UPDATE node SET trace_enabled=?, trace_gateway_enabled=?, trace_revision=? WHERE singleton=1", value.Enabled, value.GatewayEnabled, value.Revision)
		return err
	})
	if err == nil {
		h.traceGatewayEnabled.Store(value.GatewayEnabled)
		h.traceEnabled.Store(value.Enabled)
	}
	return value, err
}
