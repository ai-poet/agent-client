package live

import (
	"errors"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) readTraceSettings() {
	m := r.App.Settings
	if m.Trace.Pending {
		return
	}
	current := r.settingsCurrent()
	m.Trace.Pending = true
	r.async(func() {
		out, err := r.controller.Do(r.ctx, protocol.Command{Kind: "read_trace_settings"})
		r.enqueue(func(c *ui.Context) {
			if !current() {
				return
			}
			m.Trace.Pending = false
			value, ok := detail[protocol.TraceSettings](out.Data)
			if err != nil || !ok || out.Kind != "trace_settings" {
				m.TraceReady = false
				return
			}
			options := settings.TraceOptions{Enabled: value.Enabled, Gateway: value.GatewayEnabled}
			if m.Trace.Draft == m.Trace.Saved {
				m.Trace.Draft = options
			}
			m.Trace.Saved, m.Trace.Revision, m.TraceReady = options, value.Revision, true
		})
	})
}

func (r *Runtime) saveTraceSettings() {
	m := r.App.Settings
	if !m.TraceReady || m.Trace.Pending {
		return
	}
	current := r.settingsCurrent()
	draft := m.Trace.Draft
	value := protocol.TraceSettings{Revision: m.Trace.Revision, Enabled: draft.Enabled, GatewayEnabled: draft.Gateway}
	m.Trace.Pending = true
	r.execute(protocol.Command{Kind: "save_trace_settings", Data: value}, func(c *ui.Context, out protocol.Output, err error) {
		if !current() {
			return
		}
		m.Trace.Pending = false
		saved, ok := detail[protocol.TraceSettings](out.Data)
		if err == nil && (!ok || out.Kind != "trace_settings" || saved.Revision <= value.Revision || saved.Enabled != draft.Enabled || saved.GatewayEnabled != draft.Gateway) {
			err = errors.New("unexpected trace settings response")
		}
		if err != nil {
			r.Report(err)
			r.readTraceSettings()
			return
		}
		m.Trace.Saved, m.Trace.Revision = draft, saved.Revision
		r.localRuntime().traceBadges = nil
		kit.Toast(c, kit.Done, i18n.T(c, "trace_settings_saved"))
	})
}

func (r *Runtime) clearAllTraces() {
	m := r.App.Settings
	if !m.TraceReady || m.Trace.Pending {
		return
	}
	current := r.settingsCurrent()
	m.Trace.Pending = true
	r.execute(protocol.Command{Kind: "clear_all_traces"}, func(c *ui.Context, out protocol.Output, err error) {
		if current() {
			m.Trace.Pending = false
		}
		if err == nil && out.Kind != "all_traces_cleared" {
			err = errors.New("unexpected trace clear response")
		}
		if err != nil {
			r.Report(err)
			return
		}
		root := r.localRuntime()
		root.traceBadges = nil
		if v := root.tracePanel; v != nil && v.owner == r {
			root.clearTracePanel()
		}
		kit.Toast(c, kit.Done, i18n.T(c, "trace_all_cleared"))
	})
}
