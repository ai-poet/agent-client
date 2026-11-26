package settings

import (
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/egoist/mygo/ui"
)

type TraceOptions struct{ Enabled, Gateway bool }

func traceSettings(c *ui.Context, m *Model) {
	t := &m.Trace
	disabled := t.Pending || m.Live && !m.TraceReady
	kit.Group(c, "trace-settings", i18n.T(c, "trace_title"), "", nil, func() {
		kit.Card(c, func() {
			save := func() {
				if m.SaveTrace != nil {
					m.SaveTrace()
				} else if !m.Live {
					t.Commit()
				}
			}
			if settingSwitch(c, "capture", i18n.T(c, "trace_enable_capture"), i18n.T(c, "trace_capture_privacy"), &t.Draft.Enabled, disabled) {
				save()
			}
			if settingSwitch(c, "gateway", i18n.T(c, "trace_gateway_header"), i18n.T(c, "trace_gateway_allowlist"), &t.Draft.Gateway, disabled || !t.Draft.Enabled) {
				save()
			}
			title := i18n.T(c, "trace_clear_all")
			kit.Row(c, "clear", title, "", func() {
				small(c, "clear", title).Disabled(disabled).OnClick(func() { m.traceClearConfirm = true })
			})
			if m.traceClearConfirm && kit.Confirm(c.Key("confirm-clear"), &m.traceClearConfirm, kit.Confirmation{
				Title: title, Message: i18n.T(c, "trace_clear_consequence"), Buttons: []string{i18n.T(c, "settings_cancel"), title}, Default: 0, Destructive: []int{1},
			}) == 1 {
				m.traceClearConfirm = false
				if !disabled && m.ClearAllTraces != nil {
					m.ClearAllTraces()
				} else if !m.Live {
					notice(c)
				}
			}
		})
	})
}
