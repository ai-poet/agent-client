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
