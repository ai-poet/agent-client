package settings

import (
	"github.com/egoist/mygo/ui"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/icons"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/viewmodel"
)

type CollectionActions struct {
	Pause  func(id string, paused bool)
	Update func(id string)
}

var CollectionMessages = Messages{Failed: "runtime_failed", Conflict: "runtime_conflict", Uncertain: "runtime_unknown"}

// collections are the selected host's knowledge collections on the
// Knowledge page: each one's state, and pausing, resuming and updating it.
// A host without collections shows no group.
func collections(c *ui.Context, m *Model) {
	if len(m.Collections) == 0 {
		return
	}
	kit.Group(c, "collections", i18n.T(c, "knowledge_collections"), "", nil, func() {
		kit.Card(c, func() {
			for _, x := range m.Collections {
				kit.Entry(c, x.ID, func() { kit.Record(c, x.Name, collectionState(c, x)) }, nil, func() {
					paused := x.State == viewmodel.KnowledgePaused
					icon, label := icons.Pause, "knowledge_pause"
					if paused {
						icon, label = icons.Play, "knowledge_resume"
					}
					kit.Action(c.Key("pause"), icon, i18n.T(c, label), x.Busy).OnClick(func() {
						if !m.Live {
							notice(c)
						} else if m.CollectionActions != nil {
							m.CollectionActions.Pause(x.ID, !paused)
						}
					})
					kit.Action(c.Key("update"), icons.Reload, i18n.T(c, "knowledge_update"), x.Busy || paused).OnClick(func() {
						if !m.Live {
							notice(c)
						} else if m.CollectionActions != nil {
							m.CollectionActions.Update(x.ID)
						}
					})
				})
			}
		})
	})
}

// collectionState is a collection's state as its row says it: what it is
// doing with the sources indexed, or what could not be indexed and why.
func collectionState(c *ui.Context, x viewmodel.KnowledgeCollection) string {
	switch x.State {
	case viewmodel.KnowledgeUnavailable:
		return i18n.T(c, "knowledge_collection_unavailable")
	case viewmodel.KnowledgePaused:
		return i18n.T(c, "knowledge_collection_paused")
	case viewmodel.KnowledgeFailed:
		return i18n.T(c, "knowledge_collection_failed", "reason", x.Reason)
	case viewmodel.KnowledgeIndexing:
		return i18n.T(c, "knowledge_collection_indexing", "count", x.Sources)
	case viewmodel.KnowledgeReady:
		if x.Failed > 0 {
			return i18n.T(c, "knowledge_collection_failures", "count", x.Failed, "reason", x.Reason)
		}
		return i18n.T(c, "knowledge_collection_ready", "count", x.Sources)
	}
	return i18n.T(c, "knowledge_collection_pending")
}
