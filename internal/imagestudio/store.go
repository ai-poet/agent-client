package imagestudio

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/ai-poet/agent-client/internal/cheapauth"
)

const (
	customConfigFile = "image-api.json"
	studioIndexFile  = "index.json"
	maxStoredJobs    = 200
)

// CustomStore persists the custom image API configuration and API key in a
// 0600 private file under the profile directory.
type CustomStore struct {
	mu   sync.Mutex
	path string
}

// NewCustomStore creates a store rooted at <profileDir>/image-api.json.
func NewCustomStore(profileDir string) (*CustomStore, error) {
	if strings.TrimSpace(profileDir) == "" {
		return nil, errors.New("profile directory is empty")
	}
	return &CustomStore{path: filepath.Join(profileDir, customConfigFile)}, nil
}

// Path returns the 0600 credential file path.
func (s *CustomStore) Path() string { return s.path }

// Load reads the persisted custom config. A missing file returns zero config.
func (s *CustomStore) Load() (CustomConfig, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.loadLocked()
}

func (s *CustomStore) loadLocked() (CustomConfig, error) {
	raw, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) || (err == nil && len(bytes.TrimSpace(raw)) == 0) {
		return CustomConfig{}, nil
	}
	if err != nil {
		return CustomConfig{}, err
	}
	var cfg CustomConfig
	if err := json.Unmarshal(raw, &cfg); err != nil {
		return CustomConfig{}, err
	}
	return cfg, nil
}

// Summary loads only the UI-safe projection (no API key).
func (s *CustomStore) Summary() (CustomSummary, error) {
	cfg, err := s.Load()
	if err != nil {
		return CustomSummary{}, err
	}
	return cfg.Summary(), nil
}

// Save validates and persists the custom image API config with 0600 permissions.
// When keepExistingKey is true and apiKeyInput is blank, the previously stored
// API key is preserved without ever exposing it to the UI thread.
func (s *CustomStore) Save(baseURL, apiKeyInput string, keepExistingKey bool, model, extraParams string) (CustomSummary, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	normalizedBase, err := NormalizeBaseURL(baseURL)
	if err != nil {
		return CustomSummary{}, err
	}
	model = strings.TrimSpace(model)
	if model == "" {
		return CustomSummary{}, &ImageError{Kind: ErrBadRequest, Message: "model is required"}
	}
	extra := strings.TrimSpace(extraParams)
	if _, err := ParseExtraParams(extra); err != nil {
		return CustomSummary{}, err
	}

	key := strings.TrimSpace(apiKeyInput)
	if key == "" && keepExistingKey {
		if existing, loadErr := s.loadLocked(); loadErr == nil {
			key = strings.TrimSpace(existing.APIKey)
		}
	}

	cfg := CustomConfig{
		BaseURL:     normalizedBase,
		APIKey:      key,
		Model:       model,
		ExtraParams: extra,
	}
	encoded, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return CustomSummary{}, err
	}
	encoded = append(encoded, '\n')
	if err := cheapauth.WritePrivateFile(s.path, encoded); err != nil {
		return CustomSummary{}, err
	}
	return cfg.Summary(), nil
}

// ClearKey removes only the stored API key while retaining BaseURL/Model/ExtraParams.
func (s *CustomStore) ClearKey() (CustomSummary, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	cfg, err := s.loadLocked()
	if err != nil {
		return CustomSummary{}, err
	}
	cfg.APIKey = ""
	encoded, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return CustomSummary{}, err
	}
	encoded = append(encoded, '\n')
	if err := cheapauth.WritePrivateFile(s.path, encoded); err != nil {
		return CustomSummary{}, err
	}
	return cfg.Summary(), nil
}

// Clear removes the custom API credential file completely.
func (s *CustomStore) Clear() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	err := os.Remove(s.path)
	if err == nil || errors.Is(err, os.ErrNotExist) || os.WriteFile(s.path, nil, 0o600) == nil {
		return nil
	}
	return err
}

// IndexStore persists studio preferences, group denials, and job history at
// <profileDir>/image-studio/index.json.
type IndexStore struct {
	mu   sync.Mutex
	path string
}

// NewIndexStore creates a store at <profileDir>/image-studio/index.json.
func NewIndexStore(profileDir string) (*IndexStore, error) {
	if strings.TrimSpace(profileDir) == "" {
		return nil, errors.New("profile directory is empty")
	}
	return &IndexStore{path: filepath.Join(profileDir, "image-studio", studioIndexFile)}, nil
}

// Load reads and recovers the studio index. Interrupted in-flight jobs from a
// previous run are settled as failed/interrupted, and expired denials are pruned.
func (s *IndexStore) Load(now time.Time) (Index, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	raw, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) || (err == nil && len(bytes.TrimSpace(raw)) == 0) {
		return Index{}, nil
	}
	if err != nil {
		return Index{}, err
	}
	var idx Index
	if err := json.Unmarshal(raw, &idx); err != nil {
		return Index{}, nil
	}
	RecoverIndex(&idx, now.Unix())
	return idx, nil
}

// Save writes the studio index atomically with 0600 permissions.
func (s *IndexStore) Save(idx Index) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(idx.Jobs) > maxStoredJobs {
		idx.Jobs = idx.Jobs[:maxStoredJobs]
	}
	encoded, err := json.MarshalIndent(idx, "", "  ")
	if err != nil {
		return err
	}
	encoded = append(encoded, '\n')
	return cheapauth.WritePrivateFile(s.path, encoded)
}

// RecoverIndex marks any running job left from an earlier process as interrupted
// and drops group denials older than DenialTTL.
func RecoverIndex(idx *Index, nowUnix int64) {
	if idx == nil {
		return
	}
	for i := range idx.Jobs {
		if idx.Jobs[i].Status == JobRunning {
			idx.Jobs[i].Status = JobFailed
			idx.Jobs[i].FinishedAt = nowUnix
			idx.Jobs[i].Error = &ImageError{Kind: ErrInterrupted, Message: "interrupted"}
		}
	}
	ttl := int64(DenialTTL / time.Second)
	kept := idx.Denials[:0]
	for _, d := range idx.Denials {
		if nowUnix-d.At < ttl {
			kept = append(kept, d)
		}
	}
	idx.Denials = kept
}

// IsDenied reports whether groupID refused model within DenialTTL.
func IsDenied(idx *Index, model string, groupID int64, nowUnix int64) bool {
	if idx == nil || groupID <= 0 {
		return false
	}
	ttl := int64(DenialTTL / time.Second)
	for _, d := range idx.Denials {
		if d.GroupID == groupID && strings.EqualFold(strings.TrimSpace(d.Model), strings.TrimSpace(model)) && nowUnix-d.At < ttl {
			return true
		}
	}
	return false
}

// RecordDenial records a refusal for (model, groupID).
func RecordDenial(idx *Index, model string, groupID int64, nowUnix int64) {
	if idx == nil || groupID <= 0 || strings.TrimSpace(model) == "" {
		return
	}
	kept := idx.Denials[:0]
	for _, d := range idx.Denials {
		if !(d.GroupID == groupID && strings.EqualFold(strings.TrimSpace(d.Model), strings.TrimSpace(model))) {
			kept = append(kept, d)
		}
	}
	idx.Denials = append(kept, Denial{
		Model:   strings.TrimSpace(model),
		GroupID: groupID,
		At:      nowUnix,
	})
}
