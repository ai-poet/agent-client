package cheapauth

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
)

// Store owns one credential file below the sailry profile directory.
type Store struct {
	path string
}

func NewStore(profileDir string) (*Store, error) {
	if strings.TrimSpace(profileDir) == "" {
		return nil, &AuthError{Kind: ErrorStorage, Op: "profile"}
	}
	return &Store{path: filepath.Join(profileDir, credentialName)}, nil
}

func (s *Store) Path() string { return s.path }

func (s *Store) Load() (*Credentials, error) {
	raw, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, &AuthError{Kind: ErrorStorage, Op: "load"}
	}
	var credentials Credentials
	if err := json.Unmarshal(raw, &credentials); err != nil || !credentials.valid() {
		return nil, &AuthError{Kind: ErrorStorage, Op: "load"}
	}
	return &credentials, nil
}

func (s *Store) Save(credentials Credentials) error {
	if !credentials.valid() {
		return &AuthError{Kind: ErrorStorage, Op: "save"}
	}
	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "mkdir"}
	}
	encoded, err := json.MarshalIndent(credentials, "", "  ")
	if err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "encode"}
	}
	encoded = append(encoded, '\n')
	temporary, err := os.CreateTemp(filepath.Dir(s.path), ".cheaprouter-account-*")
	if err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "create"}
	}
	name := temporary.Name()
	defer os.Remove(name)
	defer temporary.Close()
	if err := temporary.Chmod(0o600); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "chmod"}
	}
	if _, err := temporary.Write(encoded); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "write"}
	}
	if err := temporary.Sync(); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "sync"}
	}
	if err := temporary.Close(); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "close"}
	}
	if err := os.Rename(name, s.path); err != nil {
		// Windows cannot replace an existing destination with Rename. Retry
		// after removing only the old credential file; the new file is already
		// synced and owner-only. Unix keeps the atomic replacement above.
		if removeErr := os.Remove(s.path); removeErr != nil && !errors.Is(removeErr, os.ErrNotExist) {
			return &AuthError{Kind: ErrorStorage, Op: "replace"}
		}
		if retryErr := os.Rename(name, s.path); retryErr != nil {
			return &AuthError{Kind: ErrorStorage, Op: "replace"}
		}
	}
	// Rename preserves the temporary file's mode. Chmod once more to repair a
	// pre-existing destination's permissions on platforms with replace quirks.
	if err := os.Chmod(s.path, 0o600); err != nil {
		return &AuthError{Kind: ErrorStorage, Op: "chmod"}
	}
	return nil
}

func (s *Store) Clear() error {
	if err := os.Remove(s.path); err != nil && !errors.Is(err, os.ErrNotExist) {
		return &AuthError{Kind: ErrorStorage, Op: "clear"}
	}
	return nil
}
