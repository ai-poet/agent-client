package store

import "testing"

func TestTraceSettingPersists(t *testing.T) {
	profile, node, storage := schemaFixture(t)
	if storage.Node.TraceEnabled() {
		t.Fatal("fresh profiles must not capture model traffic")
	}
	if err := storage.Node.SetTraceEnabled(t.Context(), true); err != nil || !storage.Node.TraceEnabled() {
		t.Fatalf("enable: %v", err)
	}
	if err := storage.Close(); err != nil {
		t.Fatal(err)
	}
	reopened := reopenSchemaFixture(t, profile, node)
	if !reopened.Node.TraceEnabled() {
		t.Fatal("lost committed setting on reopen")
	}
	if err := reopened.Node.SetTraceEnabled(t.Context(), false); err != nil || reopened.Node.TraceEnabled() {
		t.Fatalf("disable: %v", err)
	}
}

func TestTraceSchemaUpgrade(t *testing.T) {
	profile, node, old := schemaFixture(t)
	schemaExecute(t, old.Node, "DROP INDEX agent_execution_timing; DROP INDEX agent_observation_outcome; ALTER TABLE node DROP COLUMN trace_enabled; ALTER TABLE node DROP COLUMN trace_gateway_enabled; ALTER TABLE node DROP COLUMN trace_revision")
	if err := old.Close(); err != nil {
		t.Fatal(err)
	}
	current := reopenSchemaFixture(t, profile, node)
	if current.Node.TraceEnabled() {
		t.Fatal("upgrade opted in to collection")
	}
	if got := schemaInteger(t, current.Node, "SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('agent_execution_timing','agent_observation_outcome')"); got != 2 {
		t.Fatalf("missing migrated indexes: %d", got)
	}
}
