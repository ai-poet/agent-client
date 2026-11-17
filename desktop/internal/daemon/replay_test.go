package daemon

import (
	"reflect"
	"testing"
)

func TestReplayTrackerDeduplicatesWithinExactStream(t *testing.T) {
	tracker := newReplayTracker([]ReplayCursor{{
		SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-a", Sequence: 4,
	}})
	base := ReplayCursor{SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-a"}
	for _, test := range []struct {
		name     string
		cursor   ReplayCursor
		accepted bool
	}{
		{name: "replayed", cursor: withSequence(base, 4)},
		{name: "out of order", cursor: withSequence(base, 3)},
		{name: "new", cursor: withSequence(base, 5), accepted: true},
		{name: "duplicate new", cursor: withSequence(base, 5)},
		{name: "other epoch", cursor: ReplayCursor{SessionID: "session-a", RuntimeID: "runtime-a", Epoch: "epoch-b", Sequence: 1}, accepted: true},
		{name: "other runtime", cursor: ReplayCursor{SessionID: "session-a", RuntimeID: "runtime-b", Epoch: "epoch-a", Sequence: 1}, accepted: true},
		{name: "other session", cursor: ReplayCursor{SessionID: "session-b", RuntimeID: "runtime-a", Epoch: "epoch-a", Sequence: 1}, accepted: true},
	} {
		t.Run(test.name, func(t *testing.T) {
			if got := tracker.advance(test.cursor); got != test.accepted {
				t.Fatalf("advance(%+v) = %v, want %v", test.cursor, got, test.accepted)
			}
		})
	}
}

func TestReplayTrackerReturnsStableMaximumCursors(t *testing.T) {
	tracker := newReplayTracker([]ReplayCursor{
		{SessionID: "b", RuntimeID: "r", Epoch: "e", Sequence: 7},
		{SessionID: "a", RuntimeID: "r", Epoch: "e", Sequence: 2},
		{SessionID: "a", RuntimeID: "r", Epoch: "e", Sequence: 6},
	})
	want := []ReplayCursor{
		{SessionID: "a", RuntimeID: "r", Epoch: "e", Sequence: 6},
		{SessionID: "b", RuntimeID: "r", Epoch: "e", Sequence: 7},
	}
	if got := tracker.cursors(); !reflect.DeepEqual(got, want) {
		t.Fatalf("cursors = %#v, want %#v", got, want)
	}
}

func withSequence(cursor ReplayCursor, sequence uint64) ReplayCursor {
	cursor.Sequence = sequence
	return cursor
}
