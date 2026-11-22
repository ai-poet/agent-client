package live

import (
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/client"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/composer"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/conversation"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/viewmodel"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
)

// A presentation belongs to one view model and one observed source copy.
// Turns keep their UI enrichment between frames; controls, images,
// publications and outgoing input are still refreshed by the ordinary pipeline.
type conversationPresentation struct {
	generation uint64
	language   string
	connection conversation.Connection
	clocks     transcriptClocks
	mounted    bool
}

func (r *Runtime) beginConversationFrame() {
	r.conversationSeen = map[protocol.SessionID]bool{}
	if r.attachments != nil {
		r.attachments.imageFrame++
	}
	if r.conversationCache == nil {
		r.conversationCache = map[protocol.SessionID]conversationCopy{}
	}
	r.conversationCopies = r.conversationCache
}

func (r *Runtime) finishConversationFrame() {
	for id := range r.conversationCache {
		if !r.conversationSeen[id] {
			delete(r.conversationCache, id)
		}
	}
	for cv, presentation := range r.conversationPresentations {
		if presentation.mounted {
			presentation.mounted = false
			r.conversationPresentations[cv] = presentation
			continue
		}
		cv.Turns, cv.Queue = nil, nil
		cv.Session.ResolveImage, cv.Session.Embed = nil, nil
		cv.Waiting, cv.Animating = composer.Interactions{}, false
		delete(r.conversationPresentations, cv)
		if r.childConversations[cv.ID] == cv {
			delete(r.childConversations, cv.ID)
		}
	}
	r.conversationCopies = nil
}

func (r *Runtime) saveConversationPresentation(cv *viewmodel.Conversation, presentation conversationPresentation) {
	if r.conversationPresentations == nil {
		r.conversationPresentations = map[*viewmodel.Conversation]conversationPresentation{}
	}
	r.conversationPresentations[cv] = presentation
}

// A bulk projection or an optimistic edit invalidates reuse, not ownership of
// the view. This still lets the frame release a detached presentation.
func (r *Runtime) invalidateConversationPresentation(cv *viewmodel.Conversation) {
	r.saveConversationPresentation(cv, conversationPresentation{mounted: r.conversationPresentations[cv].mounted})
}

func (r *Runtime) projectConversation(cv *viewmodel.Conversation, id protocol.SessionID, state client.ConversationState, now time.Time, language string) {
	copy := r.conversationCopies[id]
	// An outgoing row can temporarily hide initial loading without changing
	// the snapshot. Reproject that placeholder after the row is discarded.
	cacheable := copy.generation != 0 && !state.Loading && r.composerOutgoing[cv.ID] == nil
	saved := r.conversationPresentations[cv]
	if cacheable && saved.generation == copy.generation && saved.language == language {
		// The Node's connection override is applied later in conversation.
		// Restore the observed connection when that Node becomes available.
		cv.Session.Connection = saved.connection
		cv.Session.LoadingOlder, cv.Session.OlderFailed = state.LoadingOlder, state.OlderErr != nil
		saved.clocks.advance(cv.Turns, now)
		saved.mounted = true
		r.conversationPresentations[cv] = saved
		return
	}
	var clocks transcriptClocks
	cv.Turns, cv.Session, cv.Waiting, cv.Queue, clocks = projectTranscript(state, language)
	clocks.advance(cv.Turns, now)
	presentation := conversationPresentation{mounted: true}
	if cacheable {
		presentation.generation, presentation.language = copy.generation, language
		presentation.connection, presentation.clocks = cv.Session.Connection, clocks
	}
	r.saveConversationPresentation(cv, presentation)
}
