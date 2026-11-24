package imagestudio

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"image"
	_ "image/jpeg" // reference and mask sizes are read from JPEG inputs
	_ "image/png"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	_ "golang.org/x/image/webp"
)

const (
	// MaxReferences caps the reference images of one edit request.
	MaxReferences = 16
	// MaxGrokReferences is Grok's lower reference limit.
	MaxGrokReferences = 3
	referencesDir     = "references"
)

// ReferenceLimit returns how many reference images model accepts.
func ReferenceLimit(model string) int {
	if cheapauth.IsGrokImageModel(model) {
		return MaxGrokReferences
	}
	return MaxReferences
}

// ReferenceMimeType returns the MIME type of a PNG, JPEG or WebP reference
// image, the formats the Images edits endpoint accepts.
func ReferenceMimeType(data []byte) (string, bool) {
	switch {
	case len(data) >= 4 && bytes.Equal(data[:4], []byte{0x89, 'P', 'N', 'G'}):
		return "image/png", true
	case len(data) >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF:
		return "image/jpeg", true
	case len(data) >= 12 && string(data[:4]) == "RIFF" && string(data[8:12]) == "WEBP":
		return "image/webp", true
	default:
		return "", false
	}
}

// ImageDimensions reads the stored pixel size of a PNG, JPEG or WebP image,
// ignoring any EXIF orientation: an edit mask matches the pixels the
// endpoint receives.
func ImageDimensions(data []byte) (int, int, error) {
	cfg, _, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		return 0, 0, err
	}
	return cfg.Width, cfg.Height, nil
}

// ReferenceStore keeps content-addressed copies of the studio's reference
// images under <profile>/image-studio/references. A job names its inputs by
// these copies, so moving or deleting an original does not break a retry,
// and the same picture attached twice is stored once.
type ReferenceStore struct {
	dir string
}

// NewReferenceStore returns the store rooted in profileDir.
func NewReferenceStore(profileDir string) (*ReferenceStore, error) {
	if strings.TrimSpace(profileDir) == "" {
		return nil, errors.New("profile directory is empty")
	}
	return &ReferenceStore{dir: filepath.Join(profileDir, "image-studio", referencesDir)}, nil
}

// Dir returns the store's directory.
func (s *ReferenceStore) Dir() string { return s.dir }

// Owns reports whether path is one of the store's copies.
func (s *ReferenceStore) Owns(path string) bool {
	return s != nil && filepath.Dir(filepath.Clean(path)) == filepath.Clean(s.dir)
}

// ImportFile copies the image at path into the store and returns the copy's path.
func (s *ReferenceStore) ImportFile(path string) (string, *ImageError) {
	f, err := os.Open(path)
	if err != nil {
		return "", &ImageError{Kind: ErrBadRequest, Message: "reference image cannot be read"}
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, MaxReferenceBytes+1))
	if err != nil {
		return "", &ImageError{Kind: ErrBadRequest, Message: "reference image cannot be read"}
	}
	return s.Import(data)
}

// Import validates data as a reference image, stores it once by content and
// returns the stored path.
func (s *ReferenceStore) Import(data []byte) (string, *ImageError) {
	if err := ValidateReference(data); err != nil {
		return "", err
	}
	sum := sha256.Sum256(data)
	path := filepath.Join(s.dir, hex.EncodeToString(sum[:16])+"."+SniffExtension(data))
	if info, err := os.Stat(path); err == nil && info.Size() == int64(len(data)) {
		return path, nil
	}
	if err := os.MkdirAll(s.dir, 0o755); err != nil {
		return "", &ImageError{Kind: ErrOther, Message: "failed to store reference image"}
	}
	tmp, err := os.CreateTemp(s.dir, ".reference-*")
	if err != nil {
		return "", &ImageError{Kind: ErrOther, Message: "failed to store reference image"}
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName)
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return "", &ImageError{Kind: ErrOther, Message: "failed to store reference image"}
	}
	if err := tmp.Close(); err != nil {
		return "", &ImageError{Kind: ErrOther, Message: "failed to store reference image"}
	}
	if err := os.Rename(tmpName, path); err != nil {
		return "", &ImageError{Kind: ErrOther, Message: "failed to store reference image"}
	}
	return path, nil
}

// Prune removes stored copies that no job or draft names (keep) and that
// were written before cutoff, so a copy imported while pruning runs stays.
func (s *ReferenceStore) Prune(keep map[string]bool, cutoff time.Time) {
	entries, err := os.ReadDir(s.dir)
	if err != nil {
		return
	}
	for _, entry := range entries {
		if entry.IsDir() || strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		path := filepath.Join(s.dir, entry.Name())
		if keep[filepath.Clean(path)] {
			continue
		}
		info, err := entry.Info()
		if err != nil || !info.ModTime().Before(cutoff) {
			continue
		}
		_ = os.Remove(path)
	}
}

// ValidateReference checks one reference image's format and size.
func ValidateReference(data []byte) *ImageError {
	if len(data) == 0 {
		return &ImageError{Kind: ErrBadRequest, Message: "reference image is empty"}
	}
	if len(data) > MaxReferenceBytes {
		return &ImageError{Kind: ErrBadRequest, Message: "reference image exceeds the 20 MiB upload limit"}
	}
	if _, ok := ReferenceMimeType(data); !ok {
		return &ImageError{Kind: ErrBadRequest, Message: "reference image is not a PNG, JPEG or WebP image"}
	}
	return nil
}

// LoadInputs reads spec's stored reference images and renders its mask over
// the first of them, filling References and Mask for the request.
func LoadInputs(spec GenerateSpec) (GenerateSpec, *ImageError) {
	if len(spec.ReferencePaths) == 0 {
		spec.References, spec.Mask = nil, nil
		return spec, nil
	}
	refs := make([]ReferenceImage, 0, len(spec.ReferencePaths))
	for i, path := range spec.ReferencePaths {
		data, err := os.ReadFile(path)
		if err != nil {
			return spec, &ImageError{Kind: ErrBadRequest, Message: fmt.Sprintf("reference image %d is missing", i+1)}
		}
		if verr := ValidateReference(data); verr != nil {
			return spec, verr
		}
		mime, _ := ReferenceMimeType(data)
		refs = append(refs, ReferenceImage{Filename: filepath.Base(path), MimeType: mime, Data: data})
	}
	spec.References = refs
	spec.Mask = nil
	if HasMask(spec.MaskStrokes) {
		w, h, err := ImageDimensions(refs[0].Data)
		if err != nil {
			return spec, &ImageError{Kind: ErrBadRequest, Message: "reference image size cannot be read"}
		}
		data, merr := EncodeMask(w, h, spec.MaskStrokes)
		if merr != nil {
			return spec, merr
		}
		spec.Mask = &ReferenceImage{Filename: "mask.png", MimeType: "image/png", Data: data}
	}
	return spec, nil
}
