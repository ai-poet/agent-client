package trace

import "errors"

// Scope is process-local. A serialized Fantasy option must never transmit
// the profile path or grant a caller a filesystem destination.
func (*Scope) Options()                     {}
func (*Scope) MarshalJSON() ([]byte, error) { return []byte("null"), nil }
func (*Scope) UnmarshalJSON([]byte) error   { return errors.New("trace scope is local to the node") }
