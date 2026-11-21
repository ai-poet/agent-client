package live

import (
	"bytes"
	"context"
	"image"
	"io"
	"math"
	"strconv"
	"strings"

	"github.com/ai-poet/agent-client/internal/desktop/composer"
	"github.com/ai-poet/agent-client/internal/desktop/conversation"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
	"golang.org/x/image/draw"
	_ "golang.org/x/image/tiff"
)

func (r *Runtime) attachmentDescriptor(session, id string) (attachmentDescriptor, bool) {
	if r.attachments == nil {
		return attachmentDescriptor{}, false
	}
	value, ok := r.attachments.files[attachmentKey(session, id)]
	return value, ok
}
func (r *Runtime) registerConversationAttachment(session string, attachment protocol.Attachment) {
	id, err := protocol.ParseSessionID(session)
	if err != nil || r.attachments == nil {
		return
	}
	if current := r.attachments.files[attachmentKey(session, attachment.ID.String())]; current.image != nil && current.image.Attachment == attachment {
		return
	}
	value := attachment
	r.attachments.files[attachmentKey(session, attachment.ID.String())] = attachmentDescriptor{session: id, worktree: attachment.Spec.Worktree, name: attachment.Spec.Name, mime: attachment.Spec.MediaType, attachment: &value}
}
func (r *Runtime) registerWorkspaceFile(session, id string, worktree protocol.WorktreeID, path string, revision *string) {
	owner, err := protocol.ParseSessionID(session)
	if err != nil || r.attachments == nil {
		return
	}
	r.attachments.files[attachmentKey(session, id)] = attachmentDescriptor{session: owner, worktree: worktree, path: path, name: path, revision: cloneAttachmentRevision(revision)}
}
func cloneAttachmentRevision(value *string) *string {
	if value == nil {
		return nil
	}
	cloned := *value
	return &cloned
}
func (r *Runtime) bindAttachmentSources(session string, page protocol.ConversationPage) map[string]conversation.Image {
	if r.attachments == nil {
		return nil
	}
	owner, err := protocol.ParseSessionID(session)
	if err != nil {
		return nil
	}
	var references map[string]conversation.Image
	registerImage := func(value protocol.ConversationImage) {
		attachment := value.Attachment
		r.attachments.files[attachmentKey(session, attachment.ID.String())] = attachmentDescriptor{session: owner, worktree: attachment.Spec.Worktree, name: attachment.Spec.Name, mime: attachment.Spec.MediaType, attachment: &attachment, image: &value}
	}
	for _, entry := range page.Entries {
		for _, part := range entry.Parts {
			switch part.Kind {
			case "attachment":
				if value, ok := detail[protocol.Attachment](part.Data); ok {
					r.registerConversationAttachment(session, value)
				}
			case "image":
				if value, ok := detail[protocol.ConversationImage](part.Data); ok {
					registerImage(value)
				}
			case "tool_result":
				if value, ok := detail[protocol.ConversationPartToolResultPayload](part.Data); ok {
					for _, image := range value.Images {
						registerImage(image)
						if value.ID != nil {
							if references == nil {
								references = map[string]conversation.Image{}
							}
							key := *value.ID + "/" + strconv.FormatUint(image.Index, 10)
							if _, exists := references[key]; !exists {
								references[key] = conversation.Image{ID: image.Attachment.ID.String(), Name: image.Attachment.Spec.Name}
							}
						}
					}
				}
			}
		}
	}
	for _, run := range page.Runs {
		if run.Input != nil {
			for _, attachment := range run.Input.Attachments {
				r.registerConversationAttachment(session, attachment)
			}
		}
	}
	for _, queued := range page.Queue.Items {
		for _, attachment := range queued.Attachments {
			r.registerConversationAttachment(session, attachment)
		}
	}
	return references
}
func (r *Runtime) openAttachmentFile(x *conversationActions, file conversation.File) {
	descriptor, ok := r.attachmentDescriptor(x.cv.ID, file.ID)
	if !ok {
		r.report(x.c, fileActionFault(protocol.ErrorCodeNotFound, "file source is unavailable"), "workspace_changed")
		return
	}
	if descriptor.path != "" {
		file.Editable = file.Editable && descriptor.artifact == nil
	}
	r.openCapturedFilePreview(x.cv.ID, file, descriptor)
}
func (r *Runtime) prepareAttachmentDownload(ctx context.Context, descriptor attachmentDescriptor) (protocol.StreamID, uint64, string, error) {
	var command protocol.Command
	if descriptor.image != nil {
		command = protocol.Command{Kind: "download_image", Data: &protocol.CommandDownloadImagePayload{Session: descriptor.session, Image: *descriptor.image}}
	} else if descriptor.attachment != nil {
		command = protocol.Command{Kind: "download_attachment", Data: &protocol.CommandDownloadAttachmentPayload{Worktree: descriptor.attachment.Spec.Worktree, Attachment: descriptor.attachment.ID}}
	} else {
		command = protocol.Command{Kind: "download_file", Data: &protocol.CommandDownloadFilePayload{Worktree: descriptor.worktree, Path: descriptor.path}}
	}
	output, err := r.controller.Do(ctx, command)
	if err != nil {
		return protocol.StreamID{}, 0, "", err
	}
	invalid := func(stream protocol.StreamID) (protocol.StreamID, uint64, string, error) {
		r.cancelAttachmentStream(stream)
		return protocol.StreamID{}, 0, "", fileActionFault(protocol.ErrorCodeRevisionConflict, "download source changed")
	}
	if descriptor.attachment != nil {
		value, ok := detail[protocol.AttachmentDownload](output.Data)
		if output.Kind != "attachment_download" || !ok {
			return protocol.StreamID{}, 0, "", fileActionFault(protocol.ErrorCodeInvalidRequest, "download descriptor is invalid")
		}
		if value.Attachment != *descriptor.attachment || value.Attachment.Spec.Size > math.MaxInt64 {
			return invalid(value.Stream)
		}
		return value.Stream, value.Attachment.Spec.Size, value.Attachment.Spec.Revision, nil
	}
	value, ok := detail[protocol.FileDownload](output.Data)
	if output.Kind != "file_download" || !ok {
		return protocol.StreamID{}, 0, "", fileActionFault(protocol.ErrorCodeInvalidRequest, "download descriptor is invalid")
	}
	if value.Worktree != descriptor.worktree || value.Path != descriptor.path || value.Size > math.MaxInt64 || descriptor.revision != nil && value.Revision != *descriptor.revision {
		return invalid(value.Stream)
	}
	return value.Stream, value.Size, value.Revision, nil
}

type imageDownloadSink struct {
	data  []byte
	limit uint64
}

func (s *imageDownloadSink) Write(data []byte) (int, error) {
	if uint64(len(s.data))+uint64(len(data)) > s.limit {
		return 0, fileActionFault(protocol.ErrorCodeInvalidRequest, "image exceeds its transfer limit")
	}
	s.data = append(s.data, data...)
	return len(data), nil
}
func (*imageDownloadSink) Close() error { return nil }
func (r *Runtime) readAttachmentImage(ctx context.Context, descriptor attachmentDescriptor) ([]byte, error) {
	stream, size, digest, err := r.prepareAttachmentDownload(ctx, descriptor)
	if err != nil {
		return nil, err
	}
	if size > attachmentMaximum {
		r.cancelAttachmentStream(stream)
		return nil, fileActionFault(protocol.ErrorCodeInvalidRequest, "image exceeds its transfer limit")
	}
	sink := &imageDownloadSink{limit: size}
	if err = r.controller.Download(ctx, stream, size, digest, sink, nil); err != nil {
		return nil, err
	}
	return sink.data, nil
}
func (r *Runtime) attachmentImageAction(x *conversationActions, action kit.ImageAction, image conversation.Image) {
	descriptor, ok := r.attachmentDescriptor(x.cv.ID, image.ID)
	if !ok {
		r.report(x.c, fileActionFault(protocol.ErrorCodeNotFound, "image source is unavailable"), "workspace_changed")
		return
	}
	r.attachmentImageDescriptorAction(x.c, action, descriptor)
}
func (r *Runtime) attachmentImageDescriptorAction(c *ui.Context, action kit.ImageAction, descriptor attachmentDescriptor) {
	switch action {
	case kit.ImageSave:
		r.chooseAttachmentDestination(descriptor)
	case kit.ImageOpen:
		r.openExternalAttachment(descriptor, false)
	case kit.ImageCopy:
		native := r.attachments.native
		if native.CopyImage == nil {
			r.report(c, fileActionFault(protocol.ErrorCodeUnavailable, "image clipboard is unavailable"), "workspace_changed")
			return
		}
		r.async(func() {
			data, err := r.readAttachmentImage(r.attachments.ctx, descriptor)
			r.enqueue(func(c *ui.Context) {
				if err != nil {
					r.downloadOutcome(c, err)
					return
				}
				native.CopyImage(data, descriptor.mime)
			})
		})
	}
}
func (r *Runtime) draftImageAction(x *conversationActions, action kit.ImageAction, a *composer.Attachment) {
	if record := r.attachments.items[a]; record != nil && record.ready != nil {
		r.attachmentImageAction(x, action, conversation.Image{ID: record.ready.ID.String(), Name: a.Name})
		return
	}
	if action == kit.ImageCopy && len(a.Data) > 0 && r.attachments.native.CopyImage != nil {
		r.attachments.native.CopyImage(a.Data, a.Mime)
		return
	}
	if a.Path != "" && action == kit.ImageOpen && r.attachments.native.OpenLocal != nil {
		r.attachments.native.OpenLocal(a.Path)
		return
	}
	if len(a.Data) > 0 {
		r.localImageAction(a, action)
		return
	}
	r.report(x.c, fileActionFault(protocol.ErrorCodeBusy, "image upload is still preparing"), "workspace_changed")
}

// Decoded bitmaps belong to the owning Node, not to the projected history.
func (r *Runtime) resolveConversationImage(session string, image conversation.Image) conversation.Image {
	key := attachmentKey(session, image.ID)
	descriptor, ok := r.attachmentDescriptor(session, image.ID)
	if !ok {
		return image
	}
	if descriptor.artifact != nil {
		if thumb := r.projectDocumentThumbnail(session, image.ID, descriptor); thumb != nil {
			return *thumb
		}
		return image
	}
	cache := r.attachmentImages(session)
	if source := r.projectAttachmentImage(cache, key); source != nil {
		image.Source = source
		image.Width, image.Height = source.Size()
	}
	image.Missing = cache.missing[key]
	return image
}

func (r *Runtime) attachmentImages(session string) *attachmentImageState {
	state := r.attachments
	connected := r.controller.Connected()
	if connected && !state.imageConnected {
		for _, cache := range state.imageSessions {
			cache.cancel()
			cache.ctx, cache.cancel = context.WithCancel(state.ctx)
			clear(cache.loading)
			clear(cache.failed)
		}
		for _, publication := range state.publications {
			if !publication.ready {
				publication.loading, publication.failed = false, false
			}
		}
		for _, document := range state.documents {
			document.loading, document.failed = false, false
		}
	}
	state.imageConnected = connected
	cache := state.imageSessions[session]
	if cache == nil {
		ctx, cancel := context.WithCancel(state.ctx)
		cache = &attachmentImageState{ctx: ctx, cancel: cancel, images: map[string]*ui.Bitmap{}, loading: map[string]context.CancelFunc{}, visible: map[string]uint64{}, sources: map[string]attachmentDescriptor{}, failed: map[string]bool{}, missing: map[string]bool{}}
		state.imageSessions[session] = cache
	}
	return cache
}
func sameImageSource(a, b attachmentDescriptor) bool {
	equalRevision := a.revision == b.revision || a.revision != nil && b.revision != nil && *a.revision == *b.revision
	equalAttachment := a.attachment == b.attachment || a.attachment != nil && b.attachment != nil && *a.attachment == *b.attachment
	equalImage := a.image == b.image || a.image != nil && b.image != nil && *a.image == *b.image
	return a.session == b.session && a.worktree == b.worktree && a.path == b.path && equalRevision && equalAttachment && equalImage
}

func (r *Runtime) projectAttachmentImage(cache *attachmentImageState, key string) *ui.Bitmap {
	state := r.attachments
	descriptor, ok := state.files[key]
	if !ok {
		return nil
	}
	cache.visible[key] = state.imageFrame
	if old, exists := cache.sources[key]; exists && !sameImageSource(old, descriptor) {
		cache.releaseBitmap(key)
		delete(cache.failed, key)
		delete(cache.missing, key)
	}
	cache.sources[key] = descriptor
	if source := cache.images[key]; source != nil {
		return source
	}
	if cache.loading[key] != nil || cache.failed[key] || cache.missing[key] || !state.imageConnected {
		return nil
	}
	ctx, cancel := context.WithCancel(cache.ctx)
	cache.loading[key] = cancel
	r.async(func() {
		select {
		case state.imageSlots <- struct{}{}:
			defer func() { <-state.imageSlots }()
		case <-ctx.Done():
			return
		}
		if ctx.Err() != nil {
			return
		}
		data, err := r.readAttachmentImage(ctx, descriptor)
		var bitmap *ui.Bitmap
		undecodable := false
		if err == nil && ctx.Err() == nil {
			bitmap, err = decodeAttachmentBitmap(data)
			undecodable = err != nil
		}
		r.enqueue(func(*ui.Context) {
			if ctx.Err() != nil {
				return
			}
			delete(cache.loading, key)
			cancel()
			if err == nil {
				cache.images[key] = bitmap
				return
			}
			// Permanently unavailable pictures differ from reads that can
			// be retried on refresh or after reconnect.
			if code := faultCode(err); undecodable || code == protocol.ErrorCodeNotFound || code == protocol.ErrorCodeInvalidRequest {
				cache.missing[key] = true
			} else {
				cache.failed[key] = true
			}
		})
	})
	return nil
}

func (cache *attachmentImageState) releaseBitmap(key string) {
	if cancel := cache.loading[key]; cancel != nil {
		cancel()
		delete(cache.loading, key)
	}
	delete(cache.images, key)
}

func decodeAttachmentBitmap(data []byte, maximumEdge ...int) (*ui.Bitmap, error) {
	if len(data) > attachmentMaximum {
		return nil, fileActionFault(protocol.ErrorCodeInvalidRequest, "image exceeds its transfer limit")
	}
	config, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	if config.Width <= 0 || config.Height <= 0 || uint64(config.Width)*uint64(config.Height) > 256*1024*1024/4 {
		return nil, fileActionFault(protocol.ErrorCodeInvalidRequest, "image dimensions exceed the decoding limit")
	}
	if len(maximumEdge) > 0 && maximumEdge[0] > 0 && max(config.Width, config.Height) > maximumEdge[0] {
		decoded, _, err := image.Decode(bytes.NewReader(data))
		if err != nil {
			return nil, err
		}
		ratio := float64(maximumEdge[0]) / float64(max(config.Width, config.Height))
		width, height := max(1, int(float64(config.Width)*ratio)), max(1, int(float64(config.Height)*ratio))
		thumb := image.NewNRGBA(image.Rect(0, 0, width, height))
		draw.ApproxBiLinear.Scale(thumb, thumb.Bounds(), decoded, decoded.Bounds(), draw.Src, nil)
		return ui.NewBitmap(thumb), nil
	}
	return ui.DecodeBitmap(data)
}
func (r *Runtime) projectDraftAttachmentImages(x *conversationActions) {
	cache := r.attachmentImages(x.cv.ID)
	for _, attachment := range x.comp.Attachments {
		if attachment.Image != nil || attachment.Status != composer.Ready || !strings.HasPrefix(attachment.Mime, "image/") {
			continue
		}
		if source := r.projectAttachmentImage(cache, attachmentKey(x.cv.ID, attachment.ID)); source != nil {
			attachment.Image = source
		}
	}
}

// Shell's frame hook precedes MyGo's layout, which can build more rows.
// Allow a previous-frame mark until the current layout has had its chance.
func (r *Runtime) pruneAttachmentImages() {
	state := r.attachments
	if state == nil {
		return
	}
	for session, cache := range state.imageSessions {
		id, err := protocol.ParseSessionID(session)
		if (err != nil || !r.conversationSeen[id]) && !r.localRuntime().draftsShown[session] {
			r.releaseAttachmentImages(session)
			continue
		}
		for key, frame := range cache.visible {
			if state.imageFrame-frame > 1 {
				cache.releaseBitmap(key)
				delete(cache.visible, key)
				if !cache.failed[key] && !cache.missing[key] {
					delete(cache.sources, key)
				}
			}
		}
	}
	for key, thumbnail := range state.documents {
		if state.imageFrame-thumbnail.visible > 1 {
			if thumbnail.cancel != nil {
				thumbnail.cancel()
			}
			delete(state.documents, key)
		}
	}
}

func (r *Runtime) releaseAttachmentImages(session string) {
	state := r.attachments
	if state == nil {
		return
	}
	if cache := state.imageSessions[session]; cache != nil {
		cache.cancel()
		delete(state.imageSessions, session)
	}
	for key, publication := range state.publications {
		if publication.session.String() == session {
			delete(state.publications, key)
		}
	}
	for key, thumbnail := range state.documents {
		if strings.HasPrefix(key, session+"/") {
			if thumbnail.cancel != nil {
				thumbnail.cancel()
			}
			delete(state.documents, key)
		}
	}
}

func (r *Runtime) attachmentConversationVisible(session string) bool {
	id, err := protocol.ParseSessionID(session)
	return err == nil && r.conversationSeen[id] || r.App != nil && r.App.FocusedConversation() == session
}

var _ io.WriteCloser = (*imageDownloadSink)(nil)
