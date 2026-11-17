package daemon

import (
	"sort"
	"sync"
)

type replayKey struct {
	sessionID string
	runtimeID string
	epoch     string
}

// replayTracker is safe to consult from a reconnecting caller while a socket
// reader advances it. A sequence is unique only within its session, runtime,
// and epoch, so all three identifiers are part of the key.
type replayTracker struct {
	mu       sync.RWMutex
	sequence map[replayKey]uint64
}

func newReplayTracker(cursors []ReplayCursor) *replayTracker {
	t := &replayTracker{sequence: make(map[replayKey]uint64)}
	for _, cursor := range cursors {
		key := replayKey{cursor.SessionID, cursor.RuntimeID, cursor.Epoch}
		if previous, ok := t.sequence[key]; !ok || cursor.Sequence > previous {
			t.sequence[key] = cursor.Sequence
		}
	}
	return t
}

// advance records a sequence and reports whether it is newer than every event
// already seen for the same session/runtime/epoch. Replayed events return false.
func (t *replayTracker) advance(cursor ReplayCursor) bool {
	key := replayKey{cursor.SessionID, cursor.RuntimeID, cursor.Epoch}
	t.mu.Lock()
	defer t.mu.Unlock()
	previous, seen := t.sequence[key]
	if seen && cursor.Sequence <= previous {
		return false
	}
	t.sequence[key] = cursor.Sequence
	return true
}

func (t *replayTracker) cursors() []ReplayCursor {
	t.mu.RLock()
	cursors := make([]ReplayCursor, 0, len(t.sequence))
	for key, sequence := range t.sequence {
		cursors = append(cursors, ReplayCursor{
			SessionID: key.sessionID,
			RuntimeID: key.runtimeID,
			Epoch:     key.epoch,
			Sequence:  sequence,
		})
	}
	t.mu.RUnlock()
	sort.Slice(cursors, func(i, j int) bool {
		if cursors[i].SessionID != cursors[j].SessionID {
			return cursors[i].SessionID < cursors[j].SessionID
		}
		if cursors[i].RuntimeID != cursors[j].RuntimeID {
			return cursors[i].RuntimeID < cursors[j].RuntimeID
		}
		return cursors[i].Epoch < cursors[j].Epoch
	})
	return cursors
}
