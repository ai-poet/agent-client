package viewmodel

import "time"

// Shell is a terminal shell a host offers; an empty Path is its default.
type Shell struct {
	Path string
	Args []string
}

// KnowledgeState is what a knowledge collection is doing.
type KnowledgeState int

const (
	KnowledgePending     KnowledgeState = iota // waiting for its indexing pass
	KnowledgeIndexing                          // being read
	KnowledgeReady                             // indexed
	KnowledgePaused                            // paused by the user
	KnowledgeFailed                            // could not be indexed as a whole
	KnowledgeUnavailable                       // storage Node cannot currently answer
)

// KnowledgeCollection is a host's knowledge collection: its state, the
// sources indexed so far, those that could not be indexed (Failed) with
// the first one's Reason, or the collection's own when it failed as a
// whole, and whether an action on it is under way.
type KnowledgeCollection struct {
	ID, Name string
	State    KnowledgeState
	Sources  uint64
	Failed   uint64
	Reason   string
	Busy     bool
}

// Permission is a system permission the app or a plugin needs, by its
// i18n keys (permission_screen; permission_granted).
type Permission struct {
	Resource, Status string
	Settings         bool // the system has a settings page for it
}

// Archived is an archived session.
type Archived struct {
	ID, Title string
	At        time.Time
	// Private is a session without a project: deleting it deletes the
	// private workspace its files are in.
	Private bool
}
