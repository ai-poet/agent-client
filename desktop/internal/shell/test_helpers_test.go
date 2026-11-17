package shell

import (
	"encoding/json"
	"testing"
)

func rawJSON(t *testing.T, value any) json.RawMessage {
	t.Helper()
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

func stringPointer(value string) *string { return &value }

// setModelSnapshot sets only render values for a view test. Projection
// invariants are exercised through workbench commands in their own package.
func setModelSnapshot(model *Model, snapshot Snapshot) {
	model.state.Status = snapshot.Status
	model.state.DaemonVersion = snapshot.DaemonVersion
	model.state.Detail = snapshot.Detail
	model.state.Sessions = snapshot.Sessions
	model.state.SelectedSessionID = snapshot.SelectedSessionID
	model.state.SessionsLoading = snapshot.SessionsLoading
	model.state.SessionsError = snapshot.SessionsError
	model.state.SessionLoading = snapshot.SessionLoading
	model.state.SessionError = snapshot.SessionError
	model.state.MessageCount = snapshot.MessageCount
	model.state.Transcript = snapshot.Transcript
	model.state.Draft = snapshot.Draft
	model.state.Sending = snapshot.Sending
	model.state.SendError = snapshot.SendError
}
