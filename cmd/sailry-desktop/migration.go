package main

import (
	"context"
	"errors"
	"path/filepath"
	"sync"

	"github.com/ai-poet/agent-client/internal/migrate"
	"github.com/ai-poet/agent-client/internal/node"
)

type desktopMigration struct {
	node         *node.Node
	profile      string
	root         string
	rootErr      error
	mu           sync.Mutex
	databasePath string
}

func newDesktopMigration(node *node.Node, profile string) *desktopMigration {
	root, err := migrate.DefaultLegacyRoot()
	return &desktopMigration{node: node, profile: profile, root: root, rootErr: err}
}

func (m *desktopMigration) Detect(ctx context.Context) (*migrate.LegacyState, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if m.rootErr != nil {
		migrate.RecordFailure(m.profile, m.root, "", "", m.rootErr)
		return nil, m.rootErr
	}
	state, err := migrate.Detect(m.root)
	if err != nil {
		migrate.RecordFailure(m.profile, m.root, filepath.Join(m.root, "app.db"), "", err)
		return state, err
	}
	if state.HasDatabase {
		return state, nil
	}
	alternate, pathErr := migrate.LegacyApplicationSupportDatabase()
	if pathErr != nil {
		return state, nil
	}
	database, databaseErr := migrate.Detect(filepath.Dir(alternate))
	if databaseErr != nil {
		migrate.RecordFailure(m.profile, m.root, alternate, "", databaseErr)
		return nil, databaseErr
	}
	m.mu.Lock()
	m.databasePath = ""
	m.mu.Unlock()
	if database.HasDatabase {
		state.HasDatabase = true
		state.Counts = database.Counts
		state.DatabasePath = database.DatabasePath
		m.mu.Lock()
		m.databasePath = database.DatabasePath
		m.mu.Unlock()
	}
	return state, nil
}

func (m *desktopMigration) Run(ctx context.Context, opts migrate.Options) (*migrate.Result, error) {
	if m.rootErr != nil {
		return nil, m.rootErr
	}
	m.mu.Lock()
	opts.LegacyDatabasePath = m.databasePath
	m.mu.Unlock()
	return m.node.RunLegacyMigration(ctx, m.root, opts)
}

func (m *desktopMigration) ReadState(ctx context.Context) (*migrate.Marker, *migrate.Report, error) {
	if err := ctx.Err(); err != nil {
		return nil, nil, err
	}
	marker, markerErr := migrate.ReadMarker(m.profile)
	report, reportErr := migrate.ReadReport(m.profile)
	return marker, report, errors.Join(markerErr, reportErr)
}
