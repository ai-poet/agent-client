package live

import (
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) ReportTraceExport(err error) {
	r.enqueue(func(c *ui.Context) {
		if err == nil {
			kit.Toast(c, kit.Done, i18n.T(c, "trace_export_saved"))
			return
		}
		key := "conversation_export_failed"
		if f := commandFault(err); f != nil && f.Code == protocol.ErrorCodeInvalidRequest && f.Message == protocol.TraceExportTooLargeMessage {
			key = "trace_export_limit"
		}
		kit.Toast(c, kit.Failed, i18n.T(c, key))
	})
}

// ReportWindowFailure reports composition failure in the originating window.
func (r *Runtime) ReportWindowFailure() {
	r.enqueue(func(c *ui.Context) {
		kit.Toast(c, kit.Failed, i18n.T(c, "session_window_failed"))
	})
}

func (r *Runtime) ReportReaderFailure() {
	r.enqueue(func(c *ui.Context) { kit.Toast(c, kit.Failed, i18n.T(c, "reader_failed")) })
}
func (r *Runtime) ReportExport(err error) {
	r.enqueue(func(c *ui.Context) {
		if err != nil {
			kit.Toast(c, kit.Failed, i18n.T(c, "conversation_export_failed"))
		} else {
			kit.Toast(c, kit.Done, i18n.T(c, "conversation_exported"))
		}
	})
}
