package main

import (
	"context"
	"errors"

	"github.com/ai-poet/agent-client/internal/acceptance"
	"github.com/ai-poet/agent-client/internal/client"
)

// cleanupAcceptanceSessions runs after the child has exited. The ledger is the
// only authority: a fixture's displayed session or an incomplete child stream
// cannot grant ownership. Archive also recovers pending creation identities and
// preserves unknown requests instead of sending another stop with a new ID.
func cleanupAcceptanceSessions(ctx context.Context, controller *client.Client, directory string) error {
	if controller == nil {
		return errors.New("acceptance cleanup requires an existing Client")
	}
	sessions, err := acceptance.OwnedSessions(directory, controller.Target())
	if err != nil {
		return err
	}
	return acceptance.Archive(ctx, controller, directory, sessions)
}
