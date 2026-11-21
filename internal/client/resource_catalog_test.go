package client

import (
	"context"
	"testing"

	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/protocol"
)

func TestResourceObserversIgnoreUnrelatedUpdates(t *testing.T) {
	controller := New(&clientTransport{})
	t.Cleanup(func() { _ = controller.Close() })
	snapshot := protocol.DefaultSnapshot()
	snapshot.Node = controller.Target()
	apply := func(kind string, data any, want Apply) {
		t.Helper()
		got, err := controller.project.Apply(0, protocol.Update{Kind: kind, Data: data})
		if err != nil || got != want {
			t.Fatalf("%s: apply = %v, %v", kind, got, err)
		}
		controller.changed()
	}
	apply("snapshot", snapshot, Applied)
	var sources, directory int
	defer controller.OnResourceChange(false, func() { sources++ })()
	defer controller.OnResourceChange(true, func() { directory++ })()
	emit := func(cursor uint64, kind string, data any, want Apply) {
		t.Helper()
		apply("event", protocol.EventEnvelope{Node: snapshot.Node, Cursor: cursor, Event: protocol.Event{Kind: kind, Data: data}}, want)
	}
	assertChanges := func(wantSources, wantDirectory int) {
		t.Helper()
		if sources != wantSources || directory != wantDirectory {
			t.Fatalf("source/directory changes = %d/%d, want %d/%d", sources, directory, wantSources, wantDirectory)
		}
	}
	emit(1, "cursor", nil, Applied)
	assertChanges(0, 0)
	snapshot.Cursor = 1
	apply("snapshot", snapshot, Applied)
	assertChanges(0, 0)
	emit(2, "resource_catalog_changed", nil, Applied)
	assertChanges(0, 1)
	emit(3, "knowledge_collections_changed", nil, Applied)
	assertChanges(1, 2)
	project := protocol.DefaultProject()
	project.ID = protocol.NewProjectID()
	emit(4, "project_registered", project, Recover)
	assertChanges(1, 2)
	snapshot.Cursor, snapshot.Projects = 4, []protocol.Project{project}
	apply("snapshot", snapshot, Applied)
	assertChanges(2, 3)
	// A cursor gap can hide knowledge or directory changes not in a snapshot.
	emit(6, "cursor", nil, Recover)
	assertChanges(3, 4)
}

func TestResourceCatalogRejectsMixedEntries(t *testing.T) {
	entry := protocol.NodeID{1}
	catalog := protocol.ResourceCatalog{Entry: entry, Sources: []protocol.ResourceSource{{Node: entry, Route: protocol.RouteKey{Entry: entry, Target: entry}}}}
	transport := &clientTransport{dispatch: func(request protocol.Request) (link.Admission, error) {
		completion := make(chan link.Response, 1)
		completion <- link.Response{Output: protocol.Output{Kind: "resource_catalog", Data: catalog}}
		return link.Admission{Receipt: protocol.Receipt{ID: request.ID}, Completion: completion}, nil
	}}
	controller := New(transport)
	t.Cleanup(func() { _ = controller.Close() })
	catalog.Entry = controller.Target()
	catalog.Sources[0].Node = controller.Target()
	catalog.Sources[0].Route = protocol.RouteKey{Entry: controller.Target(), Target: controller.Target()}
	if _, err := controller.ReadResourceCatalog(t.Context()); err != nil {
		t.Fatal(err)
	}
	catalog.Sources[0].Route.Entry = protocol.NodeID{99}
	if _, err := controller.ReadResourceCatalog(t.Context()); faultCode(err) != protocol.ErrorCodeInvalidRequest {
		t.Fatalf("mixed entry catalog was accepted: %v", err)
	}
}

type knowledgeRouteTransport struct {
	*clientTransport
	target protocol.NodeID
	entry  protocol.NodeID
	sent   *protocol.Request
}

func (transport knowledgeRouteTransport) Target() protocol.NodeID { return transport.target }
func (transport knowledgeRouteTransport) Route() protocol.RouteKey {
	return protocol.RouteKey{Entry: transport.entry, Target: transport.target}
}
func (transport knowledgeRouteTransport) ForTarget(target protocol.NodeID) link.Transport {
	transport.target = target
	return transport
}
func (transport knowledgeRouteTransport) Dispatch(_ context.Context, request protocol.Request) (link.Admission, error) {
	*transport.sent = request
	completion := make(chan link.Response, 1)
	completion <- link.Response{Output: protocol.Output{Kind: "knowledge_collections", Data: []protocol.KnowledgeCollection{}}}
	return link.Admission{Receipt: protocol.Receipt{ID: request.ID}, Completion: completion}, nil
}

func TestKnowledgeRouteKeepsPreparedTargetAndIdentity(t *testing.T) {
	entry, storage := protocol.NodeID{11}, protocol.NodeID{12}
	var sent protocol.Request
	controller := New(knowledgeRouteTransport{clientTransport: &clientTransport{}, target: entry, entry: entry, sent: &sent})
	t.Cleanup(func() { _ = controller.Close() })
	request := controller.PrepareKnowledge(protocol.KnowledgeRef{Node: storage}, protocol.Command{Kind: "list_knowledge_collections"})
	if _, err := controller.ExecuteKnowledge(t.Context(), request); err != nil {
		t.Fatal(err)
	}
	if request.Target != storage || sent.Target != storage || sent.ID != request.ID {
		t.Fatalf("knowledge route rewrote target or ID: %+v -> %+v", request, sent)
	}
}

func TestKnowledgeCreateDoesNotDefaultToCurrentExecutionNode(t *testing.T) {
	var sent protocol.Request
	controller := New(knowledgeRouteTransport{clientTransport: &clientTransport{}, target: protocol.NodeID{11}, sent: &sent})
	t.Cleanup(func() { _ = controller.Close() })
	definition := protocol.DefaultKnowledgeDefinition()
	definition.Name = "Notes"
	definition.Source = protocol.KnowledgeSource{Kind: "file", Data: &protocol.KnowledgeSourceFilePayload{Path: "/notes.txt"}}
	request := controller.PrepareKnowledge(protocol.KnowledgeRef{}, protocol.Command{Kind: "create_knowledge_collection", Data: &protocol.CommandCreateKnowledgeCollectionPayload{Definition: definition}})
	if request.Target != (protocol.NodeID{}) {
		t.Fatalf("ambiguous creation silently selected %v", request.Target)
	}
	if _, err := controller.ExecuteKnowledge(t.Context(), request); faultCode(err) != protocol.ErrorCodeInvalidRequest {
		t.Fatalf("ambiguous creation was sent: %v", err)
	}
	if sent.ID != (protocol.RequestID{}) {
		t.Fatal("ambiguous creation reached a transport")
	}
}
