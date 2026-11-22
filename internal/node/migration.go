package node

import (
	"context"
	"errors"

	"github.com/ai-poet/cheaprouter-harness/internal/migrate"
)

// RunLegacyMigration uses the running profile's single storage writer. The
// caller refreshes the Client projection after a successful or partial run.
func (node *Node) RunLegacyMigration(ctx context.Context, legacyRoot string, opts migrate.Options) (*migrate.Result, error) {
	node.mu.Lock()
	stopped := node.stopped
	node.mu.Unlock()
	if stopped || node.profile == nil {
		return nil, errors.New("local Node is stopped")
	}
	return migrate.RunWithHandle(ctx, legacyRoot, node.profile.Ownership.Path, node.profile.Store.Node, node.ID(), opts)
}
