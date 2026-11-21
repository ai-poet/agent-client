package client

import (
	"context"
	"encoding/json"
	"slices"
	"sync"

	"github.com/ai-poet/agent-client/internal/link"
	"github.com/ai-poet/agent-client/internal/protocol"
)

// Resource sends a captured knowledge operation through this direct Node. The
// ordinary transport cannot manufacture a trusted runtime scope for a shell.
func (client *Client) Resource(ctx context.Context, call protocol.ResourceCall) (link.Admission, error) {
	transport, present := client.transport.(interface {
		Resource(context.Context, protocol.ResourceCall) (link.Admission, error)
	})
	if !present {
		return link.Admission{}, fault(protocol.ErrorCodeUnavailable, "resource routing is unavailable")
	}
	return transport.Resource(ctx, call)
}

func (client *Client) ReadResourceCatalog(ctx context.Context) (protocol.ResourceCatalog, error) {
	output, err := client.Do(ctx, protocol.Command{Kind: "read_resource_catalog"})
	if err != nil {
		return protocol.ResourceCatalog{}, err
	}
	catalog, err := payload[protocol.ResourceCatalog](output.Data)
	if err != nil || output.Kind != "resource_catalog" || catalog.Entry != client.Target() {
		return protocol.ResourceCatalog{}, fault(protocol.ErrorCodeInvalidRequest, "resource catalog belongs to another entry")
	}
	seen := map[protocol.NodeID]bool{}
	for _, source := range catalog.Sources {
		if seen[source.Node] || source.Route.Entry != catalog.Entry || source.Route.Target != source.Node || slices.ContainsFunc(source.Knowledge, func(collection protocol.KnowledgeCollection) bool { return collection.Node != source.Node }) {
			return protocol.ResourceCatalog{}, fault(protocol.ErrorCodeInvalidRequest, "resource source has an inconsistent route")
		}
		seen[source.Node] = true
	}
	return copyValue(catalog), nil
}

// ResourceSnapshot copies only the project and worktree projection from one
// accepted snapshot generation.
func (client *Client) ResourceSnapshot() (projects []protocol.Project, worktrees []protocol.Worktree, cursor uint64) {
	client.mu.RLock()
	defer client.mu.RUnlock()
	if snapshot := client.project.snapshot; snapshot != nil {
		return copyValue(snapshot.Projects), copyValue(snapshot.Worktrees), snapshot.Cursor
	}
	return nil, nil, 0
}

// OnResourceChange observes source resources and connectivity. Directory
// consumers also need changes to the source's direct-host view.
func (client *Client) OnResourceChange(directory bool, callback func()) func() {
	type revision struct {
		generation, sources, catalog uint64
		connected                    bool
		fault                        protocol.ErrorCode
	}
	current := func() revision {
		value := revision{
			generation: client.project.generation, sources: client.project.resources,
			connected: client.connected,
		}
		if client.err != nil {
			value.fault = faultCode(client.err)
		}
		if directory {
			value.catalog = client.project.directory
		}
		return value
	}
	var mu sync.Mutex
	// Install the callback and capture its starting revision atomically.
	client.mu.Lock()
	defer client.mu.Unlock()
	last := current()
	return client.onChangeLocked(func() {
		mu.Lock()
		client.mu.RLock()
		next := current()
		client.mu.RUnlock()
		changed := next != last
		last = next
		mu.Unlock()
		if changed {
			callback()
		}
	})
}

type resourceCatalogView struct {
	Catalog    *protocol.ResourceCatalog `json:"catalog"`
	Refreshing bool                      `json:"refreshing"`
	Error      *protocol.Fault           `json:"error"`
}

func (client *Client) watchResourceCatalog(ctx context.Context) *Observer {
	observer := newObserver(ctx)
	state := resourceCatalogView{}
	wake := make(chan struct{}, 1)
	refresh := func() {
		select {
		case wake <- struct{}{}:
		default:
		}
	}
	observer.release = client.OnResourceChange(true, refresh)
	observer.read = func() (any, error) {
		observer.mu.Lock()
		defer observer.mu.Unlock()
		return copyValue(state), nil
	}
	observer.control = func(_ context.Context, method string, _ json.RawMessage) error {
		if method != "refresh" {
			return fault(protocol.ErrorCodeInvalidRequest, "unknown resource control")
		}
		refresh()
		return nil
	}
	observer.run(func() {
		_ = client.startObserver(observer.ctx, false)
		refresh()
		for {
			select {
			case <-observer.ctx.Done():
				return
			case <-wake:
			}
			observer.mu.Lock()
			state.Refreshing = true
			observer.mu.Unlock()
			observer.notify()
			catalog, err := client.ReadResourceCatalog(observer.ctx)
			observer.mu.Lock()
			state.Refreshing, state.Error = false, observerFault(err)
			if err == nil {
				state.Catalog = &catalog
			}
			observer.mu.Unlock()
			observer.notify()
		}
	})
	return observer
}
