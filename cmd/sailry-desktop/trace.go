package main

import (
	"context"
	"errors"
	"io"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo"
)

func bindTrace(ctx context.Context, entry *desktopWindow, pause func() func()) {
	r := entry.live
	r.SetTraceExport(func(q protocol.TraceExportQuery) {
		controller, ok := r.SessionClient(q.Session.String())
		if !ok {
			r.ReportTraceExport(errors.New("trace session owner unavailable"))
			return
		}
		title := i18n.In(i18n.Match(mygo.App.Locale()), "trace_export")
		resume := callPause(pause)
		file, err := mygo.Dialog.Save(mygo.SaveDialogOptions{
			Parent: entry.window, Title: title, DefaultPath: "trace-" + q.Session.String() + ".zip",
			Filters: []mygo.FileFilter{{Name: "ZIP", Extensions: []string{"zip"}}}, CreateDirectories: true,
		})
		resume()
		if err != nil {
			r.ReportTraceExport(err)
			return
		}
		if file == "" {
			return
		}
		go func() {
			r.ReportTraceExport(publishExport(file, func(w io.Writer) error {
				if err := controller.DownloadTrace(ctx, q, w); err != nil {
					return err
				}
				return ctx.Err()
			}))
		}()
	})
}
