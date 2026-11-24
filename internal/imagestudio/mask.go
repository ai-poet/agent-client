package imagestudio

import (
	"bytes"
	"image"
	"image/png"
	"math"
)

const (
	// MinBrushWidth and MaxBrushWidth bound an inpainting stroke's width in
	// the reference image's own pixels.
	MinBrushWidth = 4
	MaxBrushWidth = 160
	// DefaultBrushWidth is the brush a new mask starts with.
	DefaultBrushWidth = 40
)

// MaskPoint is one stroke sample in the first reference image's pixels.
type MaskPoint struct {
	X float32 `json:"x"`
	Y float32 `json:"y"`
}

// MaskStroke is one brush or eraser drag over the first reference image.
// Strokes are kept instead of a bitmap so the mask is rendered at the
// image's own resolution when the request is sent, and a job's mask can be
// restored and retried after a restart.
type MaskStroke struct {
	Erase  bool        `json:"erase,omitempty"`
	Width  float32     `json:"width"`
	Points []MaskPoint `json:"points"`
}

// ClampBrushWidth bounds w to [MinBrushWidth, MaxBrushWidth].
func ClampBrushWidth(w float32) float32 {
	return min(max(w, MinBrushWidth), MaxBrushWidth)
}

// PaintStrokes paints strokes into cov, whose pixels are scale times the
// image's (cov is a downscaled preview when scale < 1). Painted pixels get
// alpha 255; eraser strokes return them to 0.
func PaintStrokes(cov *image.Alpha, strokes []MaskStroke, scale float32) {
	for _, s := range strokes {
		PaintStroke(cov, s, scale)
	}
}

// PaintStroke paints one stroke into cov at scale, as PaintStrokes does.
func PaintStroke(cov *image.Alpha, s MaskStroke, scale float32) {
	if len(s.Points) == 0 {
		return
	}
	if len(s.Points) == 1 {
		PaintSegment(cov, s.Points[0], s.Points[0], s.Width, s.Erase, scale)
		return
	}
	for i := 1; i < len(s.Points); i++ {
		PaintSegment(cov, s.Points[i-1], s.Points[i], s.Width, s.Erase, scale)
	}
}

// PaintSegment paints the round-capped segment from a to b, width wide in
// image pixels, into cov at scale.
func PaintSegment(cov *image.Alpha, a, b MaskPoint, width float32, erase bool, scale float32) {
	if cov == nil || scale <= 0 {
		return
	}
	r := float64(ClampBrushWidth(width)) * float64(scale) / 2
	ax, ay := float64(a.X)*float64(scale), float64(a.Y)*float64(scale)
	bx, by := float64(b.X)*float64(scale), float64(b.Y)*float64(scale)
	bounds := cov.Bounds()
	x0 := max(int(math.Floor(min(ax, bx)-r)), bounds.Min.X)
	x1 := min(int(math.Ceil(max(ax, bx)+r)), bounds.Max.X-1)
	y0 := max(int(math.Floor(min(ay, by)-r)), bounds.Min.Y)
	y1 := min(int(math.Ceil(max(ay, by)+r)), bounds.Max.Y-1)
	if x0 > x1 || y0 > y1 {
		return
	}
	dx, dy := bx-ax, by-ay
	lenSq := dx*dx + dy*dy
	rSq := r * r
	value := uint8(255)
	if erase {
		value = 0
	}
	for y := y0; y <= y1; y++ {
		py := float64(y) + 0.5
		row := cov.Pix[(y-bounds.Min.Y)*cov.Stride:]
		for x := x0; x <= x1; x++ {
			px := float64(x) + 0.5
			t := 0.0
			if lenSq > 0 {
				t = ((px-ax)*dx + (py-ay)*dy) / lenSq
				t = min(max(t, 0), 1)
			}
			ex, ey := px-(ax+t*dx), py-(ay+t*dy)
			if ex*ex+ey*ey <= rSq {
				row[x-bounds.Min.X] = value
			}
		}
	}
}

// RenderMask renders strokes as the OpenAI Images edit mask of a w×h
// reference image: opaque white where the image is kept and fully
// transparent where it is redrawn. It reports false when no pixel is
// painted, as when every stroke was erased.
func RenderMask(w, h int, strokes []MaskStroke) (*image.NRGBA, bool) {
	if w <= 0 || h <= 0 {
		return nil, false
	}
	cov := image.NewAlpha(image.Rect(0, 0, w, h))
	PaintStrokes(cov, strokes, 1)
	out := image.NewNRGBA(image.Rect(0, 0, w, h))
	painted := false
	for i, a := range cov.Pix {
		if a != 0 {
			painted = true
			continue
		}
		o := i * 4
		out.Pix[o], out.Pix[o+1], out.Pix[o+2], out.Pix[o+3] = 0xff, 0xff, 0xff, 0xff
	}
	return out, painted
}

// EncodeMask renders strokes over a w×h reference image as a PNG edit mask.
func EncodeMask(w, h int, strokes []MaskStroke) ([]byte, *ImageError) {
	img, painted := RenderMask(w, h, strokes)
	if !painted {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "mask has no painted area"}
	}
	var buf bytes.Buffer
	if err := png.Encode(&buf, img); err != nil {
		return nil, &ImageError{Kind: ErrBadRequest, Message: "failed to encode mask"}
	}
	return buf.Bytes(), nil
}

// HasMask reports whether strokes paint anything at all, without rendering.
func HasMask(strokes []MaskStroke) bool {
	for _, s := range strokes {
		if !s.Erase && len(s.Points) > 0 {
			return true
		}
	}
	return false
}
