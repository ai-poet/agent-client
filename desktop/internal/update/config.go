// Package update connects the Desktop's update settings to MyGo's signed
// updater. An empty configuration is valid and keeps all network activity off.
package update

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

const ConfigName = "update.json"

// Config is the client-owned update configuration. FeedURL is the full URL of
// the running platform's MyGo manifest. PublicKey is the base64 Ed25519 public
// key whose corresponding build-time key MyGo uses to authenticate packages.
// Automatic controls the quiet launch check; Prereleases selects MyGo's
// prerelease channel.
type Config struct {
	FeedURL     string `json:"feed_url"`
	PublicKey   string `json:"public_key"`
	Enabled     bool   `json:"enabled"`
	Automatic   bool   `json:"automatic"`
	Prereleases bool   `json:"prereleases"`
}

// Validate rejects a partial or malformed source configuration. The empty
// configuration remains valid and disabled.
func (c Config) Validate() error {
	if !c.Enabled && strings.TrimSpace(c.FeedURL) == "" && strings.TrimSpace(c.PublicKey) == "" {
		return nil
	}
	if strings.TrimSpace(c.FeedURL) == "" || strings.TrimSpace(c.PublicKey) == "" {
		return fmt.Errorf("%w: feed URL and public key are both required", ErrConfiguration)
	}
	key, err := base64.StdEncoding.DecodeString(strings.TrimSpace(c.PublicKey))
	if err != nil || len(key) != ed25519.PublicKeySize {
		return fmt.Errorf("%w: public key must be a base64 Ed25519 key", ErrConfiguration)
	}
	return nil
}

// Configured reports whether checks are allowed. Native installation support
// is reported separately by the running MyGo build.
func (c Config) Configured() bool {
	return c.Enabled && c.Validate() == nil
}

// Store owns one profile's update configuration.
type Store struct {
	mu   sync.Mutex
	path string
	data Config
	err  error
}

// OpenStore reads path. A missing file is the default, disabled configuration;
// a read or decode failure leaves updates disabled and is returned to the
// caller without preventing application startup.
func OpenStore(path string) (*Store, error) {
	s := &Store{path: path}
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return s, nil
	}
	if err != nil {
		s.err = fmt.Errorf("read update config: %w", err)
		return s, s.err
	}
	if err := json.Unmarshal(data, &s.data); err != nil {
		s.data = Config{}
		s.err = fmt.Errorf("%w: decode update config: %v", ErrConfiguration, err)
		return s, s.err
	}
	if err := s.data.Validate(); err != nil {
		s.data = Config{}
		s.err = err
		return s, s.err
	}
	return s, nil
}

// Config returns a copy of the last valid configuration.
func (s *Store) Config() Config {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.data
}

// Save validates and atomically replaces the configuration with permissions
// limited to the current user. It never logs or includes PublicKey in errors.
func (s *Store) Save(config Config) error {
	if err := config.Validate(); err != nil {
		return err
	}
	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')

	s.mu.Lock()
	defer s.mu.Unlock()
	if err := os.MkdirAll(filepath.Dir(s.path), 0o700); err != nil {
		return fmt.Errorf("create update config directory: %w", err)
	}
	file, err := os.CreateTemp(filepath.Dir(s.path), ".update-")
	if err != nil {
		return fmt.Errorf("write update config: %w", err)
	}
	name := file.Name()
	defer os.Remove(name)
	if err := file.Chmod(0o600); err != nil {
		file.Close()
		return fmt.Errorf("protect update config: %w", err)
	}
	if _, err := file.Write(data); err != nil {
		file.Close()
		return fmt.Errorf("write update config: %w", err)
	}
	if err := file.Sync(); err != nil {
		file.Close()
		return fmt.Errorf("sync update config: %w", err)
	}
	if err := file.Close(); err != nil {
		return fmt.Errorf("close update config: %w", err)
	}
	if err := os.Rename(name, s.path); err != nil {
		return fmt.Errorf("replace update config: %w", err)
	}
	s.data, s.err = config, nil
	return nil
}
