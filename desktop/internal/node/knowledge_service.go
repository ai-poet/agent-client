package node

import (
	"context"
	"database/sql"
	"slices"

	"github.com/jonboulle/clockwork"

	"github.com/ai-poet/agent-client/desktop/internal/link"

	"github.com/ai-poet/agent-client/desktop/internal/node/admission"
	knowledgeservice "github.com/ai-poet/agent-client/desktop/internal/node/knowledge/service"

	"github.com/ai-poet/agent-client/desktop/internal/node/store"

	"github.com/ai-poet/agent-client/desktop/internal/protocol"
)

func knowledgeCommand(kind string) bool {
	return protocol.KnowledgeCommand(kind)
}

// initKnowledge registers the semantic storage collections of the enabled
// packages and starts the retrieval service.
func (node *Node) initKnowledge(ctx context.Context) error {
	changed := false
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		if changed, err = store.RegisterKnowledgeSemantic(ctx, db); err != nil || !changed {
			return err
		}
		_, err = store.RecordEvents(ctx, db, node.ID(), []protocol.Event{{Kind: "knowledge_collections_changed"}})
		return err
	})
	if err != nil {
		return err
	}
	options := node.knowledgeOptions()
	if node.knowledgeAdjust != nil {
		node.knowledgeAdjust(&options)
	}
	node.knowledge = knowledgeservice.New(options)
	node.knowledge.Start()
	return nil
}

// followKnowledgeStorage keeps the semantic storage collections current in
// the transaction of a plugin command: a changed package registers its
// declared collections, and a storage write restarts the indexing of the
// writer's collections.
func (node *Node) followKnowledgeStorage(ctx context.Context, db *sql.Conn, request protocol.Request, changes []protocol.Event) ([]protocol.Event, error) {
	changed := false
	if slices.ContainsFunc(changes, func(change protocol.Event) bool { return change.Kind == "plugin_changed" }) {
		registered, err := store.RegisterKnowledgeSemantic(ctx, db)
		if err != nil {
			return changes, err
		}
		changed = registered
	}
	if store.KnowledgeStorageCommand(request.Command) {
		advanced, err := store.KnowledgeStorageChanged(ctx, db, request)
		if err != nil {
			return changes, err
		}
		changed = changed || advanced
	}
	if changed {
		changes = append(changes, protocol.Event{Kind: "knowledge_collections_changed"})
	}
	return changes, nil
}

// refreshKnowledgeStorage asks for an indexing pass after a committed
// command that may change what storage collections index, that registered
// one, or that removed an owner of collections. The pass runs on the knowledge worker and never delays the
// command.
func (node *Node) refreshKnowledgeStorage(command protocol.Command, changes []protocol.Event) {
	switch command.Kind {
	case "identity":
		if action, valid := dispatchPayload[protocol.IdentityCommand](command.Data); valid && (action.Kind == "confirm_sponsorship" || action.Kind == "assign_orphan_user") {
			// A user merge changes the personal catalog and its captured sources.
			if node.knowledge != nil {
				node.knowledge.Invalidate()
			}
		}
		return
	case "install_plugin", "install_plugin_source", "install_catalog_plugin", "install_plugin_upload", "set_plugin_enabled", "save_plugin_settings":
	case "remove_plugin", "remove_project", "remove_session", "remove_sessions":
		// The pass removes the collections these owned.
	default:
		if !store.KnowledgeStorageCommand(command) && !slices.ContainsFunc(changes, func(change protocol.Event) bool { return change.Kind == "knowledge_collections_changed" }) {
			return
		}
	}
	// A Node that has not started holds no knowledge index; there is nothing to
	// refresh, and shutdown/close paths must not dereference it.
	if node.knowledge != nil {
		node.knowledge.Refresh()
	}
}

// knowledgeOptions connects keyword retrieval to the profile.
func (node *Node) knowledgeOptions() knowledgeservice.Options {
	return knowledgeservice.Options{
		Node:    node.ID(),
		Working: node.power.Hold,
		Index:   node.profile.Store.Knowledge,
		Catalog: node.knowledgeCatalog,
		Context: func(ctx context.Context, stored store.KnowledgeStored) (context.Context, error) {
			var operation context.Context
			err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				var err error
				operation, err = store.KnowledgeExecutionContext(ctx, db, stored)
				return err
			})
			return operation, nodeFault(err)
		},
		Allowed: func(ctx context.Context, request protocol.Request) (allowed []store.KnowledgeStored, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				allowed, err = store.AllowedKnowledge(ctx, db, request)
				return err
			})
			return allowed, nodeFault(err)
		},
		Progress: node.recordKnowledgeProgress,
		Root: func(ctx context.Context, worktree protocol.WorktreeID) (string, error) {
			root, err := node.workspaceRoot(ctx, worktree, false)
			return root, nodeFault(err)
		},
		Sources: func(ctx context.Context, stored store.KnowledgeStored, after *string) (page store.KnowledgeStoragePage, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				ctx, err = store.KnowledgeExecutionContext(ctx, db, stored)
				if err != nil {
					return err
				}
				page, err = store.ReadKnowledgeStoragePage(ctx, db, stored, after, store.KnowledgeStoragePageLimit)
				return err
			})
			return page, nodeFault(err)
		},
		Source: func(ctx context.Context, stored store.KnowledgeStored, key string) (record *store.KnowledgeStorageRecord, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				ctx, err = store.KnowledgeExecutionContext(ctx, db, stored)
				if err != nil {
					return err
				}
				record, err = store.ReadKnowledgeStorageRecord(ctx, db, stored, key)
				return err
			})
			return record, nodeFault(err)
		},
		SourceCurrent: func(ctx context.Context, stored store.KnowledgeStored, record store.KnowledgeStorageRecord) (current bool, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				ctx, err = store.KnowledgeExecutionContext(ctx, db, stored)
				if err != nil {
					return err
				}
				current, err = store.KnowledgeStorageUnchanged(ctx, db, stored, record)
				return err
			})
			return current, nodeFault(err)
		},
		Plan: func(ctx context.Context, request protocol.Request) (plan store.SemanticPlan, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				plan, err = store.PrepareSemanticSearch(ctx, db, request)
				return err
			})
			return plan, nodeFault(err)
		},
		Materialize: func(ctx context.Context, request protocol.Request, plan store.SemanticPlan, selection store.SemanticSelection) (page protocol.PluginStorageSearchPage, stale bool, err error) {
			err = node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
				page, stale, err = store.MaterializeSemanticSearch(ctx, db, request, plan, selection)
				return err
			})
			return page, stale, nodeFault(err)
		},
		Clock: clockwork.NewRealClock(),
	}
}

// knowledgeCatalog reads the collections for an indexing pass after removing
// those whose worktree, project, conversation or package was removed. Every
// pass reconciles ownership, so a removal needs no hook of its own beyond
// requesting a pass.
func (node *Node) knowledgeCatalog(ctx context.Context) ([]store.KnowledgeStored, error) {
	var catalog []store.KnowledgeStored
	orphaned := false
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		if catalog, err = store.ReadKnowledgeCatalog(ctx, db); err == nil {
			orphaned, err = store.OrphanedKnowledge(ctx, db, catalog)
		}
		return err
	})
	if err != nil || !orphaned {
		return catalog, nodeFault(err)
	}
	removed := false
	err = node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		if catalog, removed, err = store.RemoveOrphanedKnowledge(ctx, db); err != nil || !removed {
			return err
		}
		_, err = store.RecordEvents(ctx, db, node.ID(), []protocol.Event{{Kind: "knowledge_collections_changed"}})
		return err
	})
	if err == nil && removed {
		node.publish()
	}
	return catalog, nodeFault(err)
}

// recordKnowledgeProgress stores the worker's progress and announces a
// changed collection.
func (node *Node) recordKnowledgeProgress(ctx context.Context, id protocol.KnowledgeID, generation uint64, progress protocol.KnowledgeProgress) error {
	changed := false
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		if changed, err = store.RecordKnowledgeProgress(ctx, db, id, generation, progress); err != nil || !changed {
			return err
		}
		_, err = store.RecordEvents(ctx, db, node.ID(), []protocol.Event{{Kind: "knowledge_collections_changed"}})
		return err
	})
	if err == nil && changed {
		node.publish()
	}
	return nodeFault(err)
}

func (node *Node) dispatchKnowledge(ctx context.Context, caller protocol.NodeID, request protocol.Request, body []byte) (link.Admission, error) {
	if err := node.begin(); err != nil {
		return link.Admission{}, err
	}
	completion := make(chan link.Response, 1)
	accepted := link.Admission{Receipt: protocol.Receipt{ID: request.ID, Durable: request.Command.Durable()}, Completion: completion}
	if !accepted.Receipt.Durable {
		go func() {
			defer node.work.Done()
			defer close(completion)
			// Retrieval stops with its caller and with the Node.
			operation, cancel := context.WithCancel(ctx)
			defer cancel()
			defer context.AfterFunc(node.ctx, cancel)()
			completion <- response(node.readKnowledge(operation, request))
		}()
		return accepted, nil
	}
	defer node.work.Done()
	var previous *link.Response
	err := node.profile.Store.Node.Write(ctx, func(db *sql.Conn) error {
		var err error
		previous, err = admission.Admit(ctx, db, caller, request, body)
		return err
	})
	if err != nil {
		return link.Admission{}, nodeFault(err)
	}
	defer close(completion)
	if previous != nil {
		completion <- *previous
		return accepted, nil
	}
	var output protocol.Output
	var operationErr error
	operation := node.commandContext(commandJob{user: userFromContext(ctx), scope: ctx})
	if verified, _ := ctx.Value(knowledgeLocalKey{}).(bool); verified {
		operation = context.WithValue(operation, knowledgeLocalKey{}, true)
		if info, present := store.KnowledgePackageFromContext(ctx); present {
			operation = store.WithKnowledgePackage(operation, info)
		}
	}
	err = node.profile.Store.Node.Write(operation, func(db *sql.Conn) error {
		if _, err := db.ExecContext(operation, "SAVEPOINT sailry_knowledge"); err != nil {
			return err
		}
		var events []protocol.Event
		if verified, _ := operation.Value(knowledgeLocalKey{}).(bool); !verified {
			operationErr = node.authorizeCommandDB(operation, db, caller, request)
			if operationErr == nil {
				operationErr = node.validatePluginRequestDB(operation, db, caller, request)
			}
		}
		if operationErr == nil {
			output, events, operationErr = store.ExecuteKnowledge(operation, db, request)
		}
		if operationErr != nil {
			events = nil
			if _, err := db.ExecContext(operation, "ROLLBACK TO sailry_knowledge"); err != nil {
				return err
			}
		}
		if _, err := db.ExecContext(operation, "RELEASE sailry_knowledge"); err != nil {
			return err
		}
		if _, err := store.RecordEvents(operation, db, node.ID(), events); err != nil {
			return err
		}
		return admission.Complete(operation, db, caller, request.ID, response(output, operationErr))
	})
	if err != nil {
		completion <- node.unapplied(caller, request.ID, err)
		return accepted, nil
	}
	if operationErr == nil {
		if node.knowledge != nil {
			node.knowledge.Invalidate()
		}
		node.publish()
	}
	completion <- response(output, operationErr)
	return accepted, nil
}

func (node *Node) readKnowledge(ctx context.Context, request protocol.Request) (protocol.Output, error) {
	if request.Command.Kind == "read_knowledge_candidates" {
		if node.knowledge == nil {
			return protocol.Output{Kind: "knowledge_candidates", Data: protocol.KnowledgeCandidates{Candidates: []protocol.KnowledgeCandidate{}}}, nil
		}
		candidates, err := node.knowledge.StorageCandidates(ctx, request)
		return protocol.Output{Kind: "knowledge_candidates", Data: candidates}, err
	}
	switch input := request.Command.Data.(type) {
	case *protocol.KnowledgeQuery:
		if node.knowledge == nil {
			return protocol.Output{Kind: "knowledge_search", Data: protocol.PluginStorageSearchPage{}}, nil
		}
		page, err := node.knowledge.Query(ctx, request, *input)
		return protocol.Output{Kind: "knowledge_search", Data: page}, err
	case *protocol.KnowledgeCitationRequest:
		if node.knowledge == nil {
			return protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeNotConfigured, Message: "knowledge index is not running"}
		}
		page, err := node.knowledge.Citation(ctx, request, *input)
		return protocol.Output{Kind: "knowledge_citation", Data: page}, err
	}
	if request.Command.Kind != "list_knowledge_collections" {
		return protocol.Output{}, &protocol.Fault{Code: protocol.ErrorCodeInvalidRequest, Message: "knowledge read expected"}
	}
	collections := []protocol.KnowledgeCollection{}
	err := node.profile.Store.Node.Read(ctx, func(db *sql.Conn) error {
		var err error
		collections, err = store.ListKnowledgeCollections(ctx, db, request)
		return err
	})
	return protocol.Output{Kind: "knowledge_collections", Data: collections}, err
}
