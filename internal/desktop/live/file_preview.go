package live

import (
	"bytes"
	"context"
	"errors"
	"unicode/utf8"

	"github.com/egoist/mygo/ui"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/conversation"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/fileview"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// filePreview is the window's file or image preview, read by the captured
// source owner. A newer preview replaces it and cancels its pending read.
type filePreview struct {
	owner     *Runtime
	worktree  protocol.WorktreeID
	open      bool
	file      conversation.File
	view      fileview.View
	callbacks fileview.Callbacks
	cancel    context.CancelFunc
}

func (r *Runtime) beginFilePreview(file conversation.File, callbacks fileview.Callbacks) (*filePreview, context.Context) {
	root := r.localRuntime()
	if previous := root.filePreview; previous != nil && previous.cancel != nil {
		previous.cancel()
	}
	ctx, cancel := context.WithCancel(r.ctx)
	preview := &filePreview{owner: r, open: true, file: file, callbacks: callbacks, cancel: cancel}
	root.filePreview = preview
	return preview, ctx
}

func (r *Runtime) invalidateFilePreview(worktree *protocol.WorktreeID) {
	root := r.localRuntime()
	preview := root.filePreview
	if preview == nil || r.parent != nil && preview.owner != r || worktree != nil && preview.worktree != *worktree {
		return
	}
	if preview.cancel != nil {
		preview.cancel()
	}
	preview.open = false
	preview.file = conversation.File{}
	preview.callbacks = fileview.Callbacks{}
	root.filePreview = nil
}

// errPreviewLarge is a read refused at fileview.Limit.
var errPreviewLarge = errors.New("file is past the preview limit")

// previewBuffer collects a read of at most fileview.Limit bytes.
type previewBuffer struct{ bytes.Buffer }

func (*previewBuffer) Close() error { return nil }

// openFilePreview shows path in the preview dialog, read by read, which
// reports its progress as it goes. A file no preview can draw is not read:
// the dialog names it.
func (r *Runtime) openFilePreview(path string, read func(context.Context, func(copied, size uint64)) ([]byte, error)) {
	root := r.localRuntime()
	file := conversation.File{ID: "file-preview", Path: path, Kind: attachmentFileKind(path, "")}
	preview, ctx := r.beginFilePreview(file, fileview.Callbacks{})
	if file.Kind == conversation.OtherFile {
		return
	}
	file.State = conversation.PreviewLoading
	preview.file = file
	if !root.async(func() {
		data, err := read(ctx, func(copied, size uint64) {
			if size == 0 {
				return
			}
			share := float64(copied) / float64(size)
			root.enqueue(func(*ui.Context) {
				if root.filePreview == preview && preview.open && ctx.Err() == nil {
					preview.view.Progress = share
				}
			})
		})
		loaded := file
		loaded.State = conversation.PreviewReady
		switch {
		case errors.Is(err, errPreviewLarge):
			loaded.State, err = conversation.PreviewLarge, nil
		case faultCode(err) == protocol.ErrorCodeRevisionConflict:
			loaded.State = conversation.PreviewChanged
		case err != nil:
			loaded.State = conversation.PreviewFailed
		case loaded.Kind == conversation.ImageFile:
			bitmap, decodeErr := decodeAttachmentBitmap(data)
			if decodeErr != nil {
				loaded.State, err = conversation.PreviewFailed, decodeErr
				break
			}
			width, height := bitmap.Size()
			loaded.Image = &conversation.Image{ID: loaded.ID, Name: loaded.Name(), Width: width, Height: height, Source: bitmap}
		case !utf8.Valid(data):
			// Named as text or code and holding something else: there is
			// no text to show.
			loaded.Kind = conversation.OtherFile
		default:
			loaded.Text = string(data)
		}
		root.enqueue(func(c *ui.Context) {
			if root.filePreview != preview || !preview.open || ctx.Err() != nil {
				return
			}
			if err != nil {
				root.downloadOutcome(c, err)
			}
			preview.file = loaded
		})
	}) {
		preview.cancel()
		preview.file.State = conversation.PreviewFailed
	}
}

// renderFilePreview draws the preview dialog while it is open; closing it
// stops a read still running.
func (r *Runtime) renderFilePreview(c *ui.Context) {
	preview := r.localRuntime().filePreview
	if preview == nil || !preview.open {
		return
	}
	fileview.Dialog(c, "file-preview", &preview.view, preview.file, &preview.open, preview.callbacks)
	if !preview.open && preview.cancel != nil {
		preview.cancel()
		preview.cancel = nil
	}
}
