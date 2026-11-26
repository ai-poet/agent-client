package client

import (
	"context"
	"io"

	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

type traceDestination struct{ io.Writer }

func (traceDestination) Close() error { return nil }

// DownloadTrace writes a verified ZIP. Publish the destination only on success.
func (c *Client) DownloadTrace(ctx context.Context, q protocol.TraceExportQuery, w io.Writer) error {
	out, err := c.read(ctx, protocol.Command{Kind: "export_trace", Data: q})
	if err != nil {
		return err
	}
	d, err := payload[protocol.TraceDownload](out.Data)
	if err != nil || out.Kind != "trace_download" || d.Query != q || d.Size > protocol.MaxTraceExportBytes {
		return fault(protocol.ErrorCodeInvalidRequest, "trace download does not match query")
	}
	dst, ok := w.(io.WriteCloser)
	if !ok {
		dst = traceDestination{w}
	}
	return c.Download(ctx, d.Stream, d.Size, d.Revision, dst, nil)
}
