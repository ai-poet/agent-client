package live

import (
	"slices"
	"time"

	"github.com/ai-poet/agent-client/desktop/internal/client"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/composer"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/conversation"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

// A presentation belongs to one view model and one observed source copy.
// Completed turns keep their UI enrichment between frames; controls, images,
// publications and outgoing input are still refreshed by the ordinary pipeline.
type conversationPresentation struct {
	generation uint64
	language   string
	connection conversation.Connection
	mounted    bool
}

func (r *Runtime) beginConversationFrame() {
	r.conversationSeen = map[protocol.SessionID]bool{}
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
	cacheable := copy.generation != 0 && r.composerOutgoing[cv.ID] == nil
	saved := r.conversationPresentations[cv]
	if cacheable && saved.generation == copy.generation && saved.language == language {
		// The Node's connection override is applied later in conversation.
		// Restore the observed connection when that Node becomes available.
		cv.Session.Connection = saved.connection
		saved.mounted = true
		r.conversationPresentations[cv] = saved
		return
	}
	cv.Turns, cv.Session, cv.Waiting, cv.Queue = conversationAt(state, now, language)
	cacheable = cacheable && len(state.Snapshot.Drafts) == 0 && !slices.ContainsFunc(state.Snapshot.Page.Runs, func(run protocol.ConversationRun) bool {
		return run.Status == protocol.ConversationStatusQueued || run.Status == protocol.ConversationStatusRunning || run.Status == protocol.ConversationStatusStopping
	})
	presentation := conversationPresentation{mounted: true}
	if cacheable {
		presentation.generation, presentation.language = copy.generation, language
		presentation.connection = cv.Session.Connection
	}
	r.saveConversationPresentation(cv, presentation)
}
