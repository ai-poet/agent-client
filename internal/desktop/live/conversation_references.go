package live

import (
	"github.com/ai-poet/agent-client/internal/client"
	"github.com/ai-poet/agent-client/internal/desktop/conversation"
	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/ai-poet/agent-client/internal/desktop/settings"
	"github.com/ai-poet/agent-client/internal/desktop/shell"
	"github.com/ai-poet/agent-client/internal/protocol"
	"github.com/egoist/mygo/ui"
)

func (r *Runtime) bindReferences(session string, page protocol.ConversationPage) {
	bound := map[string][]conversation.Reference{}
	canonical := map[protocol.TurnID]bool{}
	appendReference := func(turn protocol.TurnID, ref protocol.ConversationReference) {
		value, ok := composerReference(ref)
		if !ok {
			return
		}
		bound[turn.String()] = append(bound[turn.String()], conversation.Reference{ID: value.ID, Label: ref.Label, Open: func() { r.openReference(session, turn, ref) }})
	}
	for _, entry := range page.Entries {
		if entry.Author != "user" {
			continue
		}
		canonical[entry.Turn] = true
		for _, part := range entry.Parts {
			if part.Kind == "reference" {
				if ref, ok := detail[protocol.ConversationReference](part.Data); ok {
					appendReference(entry.Turn, ref)
				}
			}
		}
	}
	for _, run := range page.Runs {
		if !canonical[run.Turn] && run.Input != nil {
			for _, ref := range run.Input.Message.References {
				appendReference(run.Turn, ref)
			}
		}
	}
	r.App.Messages.SetReferences(session, bound)
}

func (r *Runtime) openReference(session string, turn protocol.TurnID, ref protocol.ConversationReference) {
	r.enqueue(func(c *ui.Context) {
		snapshot := r.snapshot()
		switch ref.Target.Kind {
		case "plugin":
			if name, ok := detail[string](ref.Target.Data); ok {
				r.App.Host = r.controller.Target().String()
				r.App.Settings.Open(settings.Plugins)
				r.App.Navigate(shell.SettingsPage)
				r.App.Settings.ShowDetails(settings.DetailPlugin, name)
				return
			}
		case "session":
			if id, ok := detail[protocol.SessionID](ref.Target.Data); ok && r.App.Data.Find(id.String()) != nil {
				r.OpenSession(id.String())
				return
			}
		case "host":
			r.App.Host = snapshot.Node.String()
			r.App.Navigate(shell.HostPage)
			return

		case "project", "worktree":
			if id, err := protocol.ParseSessionID(session); err == nil {
				if state, ok := r.controller.Conversation(id); ok {
					if path, project, ok := referenceLocation(snapshot, state, turn); ok {
						key := "reference_worktree"
						if ref.Target.Kind == "project" {
							path, key = project.Path, "reference_project"
						}
						r.App.Messages.ShowReference(session, ref.Label, i18n.T(c, key), "```text\n"+path+"\n```")
						return
					}
				}
			}
		}
		kit.Toast(c, kit.Failed, i18n.T(c, "unavailable"))
	})
}

func referenceLocation(snapshot protocol.Snapshot, state client.ConversationState, turn protocol.TurnID) (string, protocol.Project, bool) {
	for _, run := range state.Snapshot.Page.Runs {
		if run.Turn != turn {
			continue
		}
		for _, worktree := range snapshot.Worktrees {
			if worktree.ID != run.Worktree || worktree.Project == nil {
				continue
			}
			for _, project := range snapshot.Projects {
				if project.ID == *worktree.Project {
					return worktree.Path, project, true
				}
			}
		}
	}
	return "", protocol.Project{}, false
}
