package files

import (
	"context"
	"io"

	"github.com/ai-poet/cheaprouter-harness/internal/node/trace"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// Prepare in private transfer staging: never expose the profile path or a
// partial archive. The ordinary single-use stream owns the handle and cleanup.
func (t *Transfers) DownloadTrace(ctx context.Context, caller protocol.NodeID, q protocol.TraceExportQuery) (protocol.TraceDownload, error) {
	out := protocol.TraceDownload{Query: q}
	e := &transferEntry{download: true, session: q.Session}
	var err error
	e.file, e.tempRoot, e.temporary, err = t.temporary()
	if err != nil {
		return out, err
	}
	defer func() {
		if err != nil {
			cleanupTransfer(e)
		}
	}()
	if err = trace.Export(ctx, t.profile, q, e.file); err != nil {
		return out, err
	}
	var size int64
	if size, err = e.file.Seek(0, io.SeekCurrent); err != nil {
		return out, err
	}
	if _, err = e.file.Seek(0, io.SeekStart); err != nil {
		return out, err
	}
	e.size = uint64(size)
	if e.revision, err = hashOpenFile(ctx, e.file, e.size); err != nil {
		return out, err
	}
	out.Size, out.Revision = e.size, e.revision
	out.Stream, err = t.reserve(caller, "", e)
	return out, err
}
