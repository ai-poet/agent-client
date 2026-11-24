package imagestudio

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"image"
	"image/png"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func sizedPNG(t *testing.T, w, h int) []byte {
	t.Helper()
	var buf bytes.Buffer
	if err := png.Encode(&buf, image.NewRGBA(image.Rect(0, 0, w, h))); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func TestRenderMaskPaintsTransparentAndErases(t *testing.T) {
	strokes := []MaskStroke{{Width: 10, Points: []MaskPoint{{X: 5, Y: 5}, {X: 45, Y: 5}}}}
	img, painted := RenderMask(50, 20, strokes)
	if !painted {
		t.Fatal("stroke painted nothing")
	}
	if a := img.NRGBAAt(25, 5).A; a != 0 {
		t.Fatalf("painted pixel alpha = %d, want 0 (redraw)", a)
	}
	if c := img.NRGBAAt(25, 18); c.A != 0xff || c.R != 0xff {
		t.Fatalf("unpainted pixel = %v, want opaque white (keep)", c)
	}

	erased := append(strokes, MaskStroke{Erase: true, Width: 40, Points: []MaskPoint{{X: 0, Y: 0}, {X: 50, Y: 0}, {X: 50, Y: 20}, {X: 0, Y: 20}}})
	if _, painted := RenderMask(50, 20, erased); painted {
		t.Fatal("erasing every stroke still left a painted area")
	}
	if _, err := EncodeMask(50, 20, erased); err == nil || err.Kind != ErrBadRequest {
		t.Fatalf("empty mask err = %v, want bad request", err)
	}
}

func TestPaintSegmentScalesAndClampsWidth(t *testing.T) {
	cov := image.NewAlpha(image.Rect(0, 0, 50, 50))
	// 1000 is clamped to MaxBrushWidth; at scale 0.25 the 160 px brush
	// covers a 20 px radius around the preview point (25, 25).
	PaintSegment(cov, MaskPoint{X: 100, Y: 100}, MaskPoint{X: 100, Y: 100}, 1000, false, 0.25)
	if cov.AlphaAt(25, 25).A != 255 || cov.AlphaAt(25, 6).A != 255 {
		t.Fatal("expected the scaled disc to cover its radius")
	}
	if cov.AlphaAt(25, 2).A != 0 || cov.AlphaAt(2, 2).A != 0 {
		t.Fatal("disc painted beyond its clamped radius")
	}
}

func TestEncodeMaskMatchesReferenceSize(t *testing.T) {
	data, err := EncodeMask(64, 32, []MaskStroke{{Width: 8, Points: []MaskPoint{{X: 10, Y: 10}}}})
	if err != nil {
		t.Fatal(err)
	}
	w, h, derr := ImageDimensions(data)
	if derr != nil || w != 64 || h != 32 {
		t.Fatalf("mask size = %dx%d (%v), want 64x32", w, h, derr)
	}
}

func TestReferenceStoreImportsOnceAndPrunes(t *testing.T) {
	profile := t.TempDir()
	store, err := NewReferenceStore(profile)
	if err != nil {
		t.Fatal(err)
	}
	data := sizedPNG(t, 3, 3)
	src := filepath.Join(t.TempDir(), "source.png")
	if err := os.WriteFile(src, data, 0o644); err != nil {
		t.Fatal(err)
	}
	first, ierr := store.ImportFile(src)
	if ierr != nil {
		t.Fatal(ierr)
	}
	second, ierr := store.Import(data)
	if ierr != nil || second != first || !store.Owns(first) {
		t.Fatalf("same picture stored twice: %q %q (%v)", first, second, ierr)
	}
	if store.Owns(src) {
		t.Fatal("store claims a file outside its directory")
	}

	if _, ierr := store.Import([]byte("GIF89a not accepted")); ierr == nil || ierr.Kind != ErrBadRequest {
		t.Fatalf("gif import err = %v, want bad request", ierr)
	}
	big := make([]byte, MaxReferenceBytes+1)
	copy(big, data)
	if _, ierr := store.Import(big); ierr == nil {
		t.Fatal("oversized reference accepted")
	}

	other, _ := store.Import(sizedPNG(t, 4, 4))
	// A copy written after the cutoff stays even when nothing names it.
	store.Prune(map[string]bool{filepath.Clean(first): true}, time.Now().Add(-time.Hour))
	if _, err := os.Stat(other); err != nil {
		t.Fatalf("recent copy pruned: %v", err)
	}
	store.Prune(map[string]bool{filepath.Clean(first): true}, time.Now().Add(time.Minute))
	if _, err := os.Stat(other); !os.IsNotExist(err) {
		t.Fatalf("unreferenced copy kept: %v", err)
	}
	if _, err := os.Stat(first); err != nil {
		t.Fatalf("referenced copy pruned: %v", err)
	}
}

func TestLoadInputsBuildsReferencesAndMask(t *testing.T) {
	store, _ := NewReferenceStore(t.TempDir())
	path, ierr := store.Import(sizedPNG(t, 40, 30))
	if ierr != nil {
		t.Fatal(ierr)
	}
	spec, ierr := LoadInputs(GenerateSpec{
		ReferencePaths: []string{path},
		MaskStrokes:    []MaskStroke{{Width: 6, Points: []MaskPoint{{X: 20, Y: 15}}}},
	})
	if ierr != nil {
		t.Fatal(ierr)
	}
	if !spec.IsEdit() || !spec.IsInpaint() || spec.References[0].MimeType != "image/png" {
		t.Fatalf("inputs not loaded: refs=%d mask=%v", len(spec.References), spec.Mask != nil)
	}
	if w, h, _ := ImageDimensions(spec.Mask.Data); w != 40 || h != 30 {
		t.Fatalf("mask size = %dx%d, want the reference's 40x30", w, h)
	}

	if _, ierr := LoadInputs(GenerateSpec{ReferencePaths: []string{filepath.Join(store.Dir(), "gone.png")}}); ierr == nil {
		t.Fatal("missing reference loaded")
	}
	plain, ierr := LoadInputs(GenerateSpec{Prompt: "x"})
	if ierr != nil || plain.IsEdit() {
		t.Fatalf("text-to-image spec became an edit: %v", ierr)
	}

	// Paths and strokes survive the job index; the loaded bytes do not.
	encoded, err := json.Marshal(spec)
	if err != nil {
		t.Fatal(err)
	}
	var restored GenerateSpec
	if err := json.Unmarshal(encoded, &restored); err != nil {
		t.Fatal(err)
	}
	if len(restored.ReferencePaths) != 1 || len(restored.MaskStrokes) != 1 || restored.References != nil || restored.Mask != nil {
		t.Fatalf("persisted spec = %+v", restored)
	}
}

func TestEditsSendMaskPart(t *testing.T) {
	ref := sizedPNG(t, 8, 8)
	mask, ierr := EncodeMask(8, 8, []MaskStroke{{Width: 4, Points: []MaskPoint{{X: 4, Y: 4}}}})
	if ierr != nil {
		t.Fatal(ierr)
	}
	var gotMask []byte
	var gotImages int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := r.ParseMultipartForm(8 << 20); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		gotImages = len(r.MultipartForm.File["image"])
		if files := r.MultipartForm.File["mask"]; len(files) == 1 {
			f, _ := files[0].Open()
			gotMask, _ = io.ReadAll(f)
			_ = f.Close()
		}
		_, _ = fmt.Fprintf(w, `{"data":[{"b64_json":%q}]}`, base64.StdEncoding.EncodeToString(ref))
	}))
	defer srv.Close()

	client := newTestClient(5*time.Millisecond, time.Second)
	_, err := client.GenerateCustom(t.Context(), CustomConfig{BaseURL: srv.URL, Model: "m"}, GenerateSpec{
		Prompt:     "redraw the sky",
		Count:      1,
		References: []ReferenceImage{{Filename: "a.png", MimeType: "image/png", Data: ref}},
		Mask:       &ReferenceImage{Filename: "mask.png", MimeType: "image/png", Data: mask},
	})
	if err != nil {
		t.Fatal(err)
	}
	if gotImages != 1 || !bytes.Equal(gotMask, mask) {
		t.Fatalf("images=%d mask=%d bytes, want 1 image and the mask", gotImages, len(gotMask))
	}

	_, err = client.GenerateCustom(t.Context(), CustomConfig{BaseURL: srv.URL, Model: "m"}, GenerateSpec{
		Prompt:     "x",
		References: []ReferenceImage{{Data: ref}},
		Mask:       &ReferenceImage{},
	})
	if err == nil || err.Kind != ErrBadRequest {
		t.Fatalf("empty mask err = %v, want bad request", err)
	}
}

func TestReferenceLimitByModel(t *testing.T) {
	if ReferenceLimit("gpt-image-2") != MaxReferences {
		t.Fatal("OpenAI image model limit")
	}
	if ReferenceLimit("grok-2-image") != MaxGrokReferences {
		t.Fatal("Grok image model limit")
	}
}
