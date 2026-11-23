package imagestudio

import (
	"bytes"
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
)

const (
	// DefaultCheapRouterModel is the fallback image model when catalog has no match.
	DefaultCheapRouterModel = "gpt-image-2"
	// MaxCount is the maximum number of images requested in one job.
	MaxCount = 4
	// MaxRunning caps concurrent generation jobs.
	MaxRunning = 3
	// MaxReferenceBytes caps each reference image uploaded to /v1/images/edits.
	MaxReferenceBytes = 20 << 20
	// DenialTTL is how long a refused CheapRouter group is skipped before retrying.
	DenialTTL = 24 * time.Hour
	// PollInterval matches Rust sub2api::images::POLL_INTERVAL_SECONDS.
	PollInterval = 3 * time.Second
	// TaskTimeout matches Rust sub2api::images::TASK_TIMEOUT_SECONDS.
	TaskTimeout = 30 * time.Minute
	// MaxDownloadBytes protects memory/disk when fetching generated image URLs.
	MaxDownloadBytes = 50 << 20
	// MaxErrorBodyBytes caps error responses read from image servers.
	MaxErrorBodyBytes = 64 << 10
)

// PickDefaultImageModel selects the preferred default CheapRouter image model.
func PickDefaultImageModel(models []cheapauth.Model) string {
	if len(models) == 0 {
		return DefaultCheapRouterModel
	}
	for _, preferred := range []string{"gpt-image-2", "gpt-image-1.5", "gpt-image-1"} {
		for _, m := range models {
			if strings.EqualFold(m.ID, preferred) {
				return m.ID
			}
		}
	}
	return models[0].ID
}

// SizeOptions returns supported sizes for model, first being the default.
// Matches Rust sub2api::images::size_options plus common OpenAI-compatible sizes.
func SizeOptions(model string) []string {
	id := strings.ToLower(strings.TrimSpace(model))
	if idx := strings.LastIndex(id, "/"); idx >= 0 {
		id = id[idx+1:]
	}
	switch {
	case cheapauth.IsGrokImageModel(id):
		return []string{"1024x1024", "1536x1024", "1024x1536", "2048x2048", "2048x1152"}
	case strings.HasPrefix(id, "gpt-image-2"):
		return []string{"1024x1024", "1536x1024", "1024x1536", "2048x2048", "2048x1152", "3840x2160", "2160x3840", "auto"}
	case strings.HasPrefix(id, "gpt-image-"):
		return []string{"1024x1024", "1536x1024", "1024x1536", "auto"}
	case strings.HasPrefix(id, "dall-e-3"):
		return []string{"1024x1024", "1792x1024", "1024x1792"}
	default:
		return []string{"1024x1024", "1536x1024", "1024x1536", "1792x1024", "1024x1792", "2048x2048", "auto"}
	}
}

// SupportsQuality reports whether model accepts a quality field.
func SupportsQuality(model string) bool {
	return !cheapauth.IsGrokImageModel(model)
}

// QualityOptions returns the quality values offered for model.
func QualityOptions(model string) []string {
	id := strings.ToLower(strings.TrimSpace(model))
	if idx := strings.LastIndex(id, "/"); idx >= 0 {
		id = id[idx+1:]
	}
	if strings.HasPrefix(id, "dall-e-3") {
		return []string{"auto", "standard", "hd"}
	}
	return []string{"auto", "low", "medium", "high"}
}

// SniffExtension inspects magic bytes for PNG, JPEG, WEBP, and GIF outputs.
func SniffExtension(data []byte) string {
	switch {
	case len(data) >= 4 && bytes.Equal(data[:4], []byte{0x89, 'P', 'N', 'G'}):
		return "png"
	case len(data) >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF:
		return "jpg"
	case len(data) >= 12 && string(data[:4]) == "RIFF" && string(data[8:12]) == "WEBP":
		return "webp"
	case len(data) >= 4 && string(data[:4]) == "GIF8":
		return "gif"
	default:
		return "png"
	}
}

// MimeTypeForExt returns the canonical MIME type for a sniffed image extension.
func MimeTypeForExt(ext string) string {
	switch strings.ToLower(strings.TrimPrefix(strings.TrimSpace(ext), ".")) {
	case "jpg", "jpeg":
		return "image/jpeg"
	case "webp":
		return "image/webp"
	case "gif":
		return "image/gif"
	default:
		return "image/png"
	}
}

// DefaultOutputDir returns the local folder where generated images are saved:
// ~/Pictures/CheapRouter when a Pictures folder is usable, else <profile>/image-studio/outputs.
func DefaultOutputDir(profileDir string) string {
	if home, err := os.UserHomeDir(); err == nil && strings.TrimSpace(home) != "" {
		pictures := filepath.Join(home, "Pictures")
		if info, statErr := os.Stat(pictures); statErr == nil && info.IsDir() {
			return filepath.Join(pictures, "CheapRouter")
		}
	}
	if strings.TrimSpace(profileDir) != "" {
		return filepath.Join(profileDir, "image-studio", "outputs")
	}
	return filepath.Join(os.TempDir(), "cheaprouter-images")
}

// NormalizeBaseURL validates a custom image API base URL and trims trailing slashes.
func NormalizeBaseURL(raw string) (string, error) {
	trimmed := strings.TrimRight(strings.TrimSpace(raw), "/")
	parsed, err := url.Parse(trimmed)
	if err != nil || trimmed == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") ||
		parsed.Hostname() == "" || parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" {
		return "", &ImageError{Kind: ErrBadRequest, Message: "invalid base URL"}
	}
	return trimmed, nil
}

// GenerationsURL resolves the OpenAI-compatible POST /v1/images/generations URL
// from a user-supplied base URL (accepting origin, .../v1, or full endpoint).
func GenerationsURL(baseURL string) (string, error) {
	normalized, err := NormalizeBaseURL(baseURL)
	if err != nil {
		return "", err
	}
	lower := strings.ToLower(normalized)
	if strings.HasSuffix(lower, "/images/generations") {
		return normalized, nil
	}
	parsed, _ := url.Parse(normalized)
	path := strings.TrimRight(parsed.Path, "/")
	if path == "" {
		return normalized + "/v1/images/generations", nil
	}
	return normalized + "/images/generations", nil
}

// EditsURL resolves the OpenAI-compatible POST /v1/images/edits URL from a
// user-supplied base URL (accepting origin, .../v1, .../images/generations, or
// .../images/edits).
func EditsURL(baseURL string) (string, error) {
	normalized, err := NormalizeBaseURL(baseURL)
	if err != nil {
		return "", err
	}
	lower := strings.ToLower(normalized)
	if strings.HasSuffix(lower, "/images/edits") {
		return normalized, nil
	}
	if strings.HasSuffix(lower, "/images/generations") {
		return normalized[:len(normalized)-len("/images/generations")] + "/images/edits", nil
	}
	parsed, _ := url.Parse(normalized)
	path := strings.TrimRight(parsed.Path, "/")
	if path == "" {
		return normalized + "/v1/images/edits", nil
	}
	return normalized + "/images/edits", nil
}

// CheapRouterImageURL builds a CheapRouter gateway Images API URL (e.g. "images/generations").
func CheapRouterImageURL(origin, relPath string) (string, error) {
	normalized, err := NormalizeBaseURL(origin)
	if err != nil {
		return "", err
	}
	rel := strings.TrimLeft(relPath, "/")
	if strings.HasSuffix(strings.ToLower(normalized), "/v1") {
		return normalized + "/" + rel, nil
	}
	return normalized + "/v1/" + rel, nil
}

// ParseExtraParams validates optional extra JSON object parameters.
func ParseExtraParams(raw string) (map[string]any, error) {
	trimmed := strings.TrimSpace(raw)
	if trimmed == "" {
		return nil, nil
	}
	var parsed any
	if err := json.Unmarshal([]byte(trimmed), &parsed); err != nil {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "extra_params must be valid JSON object"}
	}
	obj, ok := parsed.(map[string]any)
	if !ok {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "extra_params must be a JSON object"}
	}
	return obj, nil
}

// DisplayHost extracts scheme://host from a URL for safe display in UI cards.
func DisplayHost(raw string) string {
	parsed, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || parsed.Host == "" {
		return ""
	}
	return parsed.Host
}
