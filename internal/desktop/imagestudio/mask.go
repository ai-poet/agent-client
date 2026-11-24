package imagestudio

import (
	"image"
	"slices"

	core "github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/egoist/mygo/ui"
)

// maskPreviewEdge bounds the preview coverage's longest edge in pixels: the
// editor shows the image well under it, and a smaller bitmap keeps each
// stroke's upload cheap while the pointer drags.
const maskPreviewEdge = 1024

// BeginMask opens the mask editor over the draft's first reference image,
// with the draft's strokes. The caller decodes the image (OpenMask) and
// hands it over with MaskLoaded.
func (s *State) BeginMask() bool {
	if s == nil || len(s.References) == 0 {
		return false
	}
	width := s.Mask.Width
	if width == 0 {
		width = core.DefaultBrushWidth
	}
	s.Mask = MaskEditor{
		Open:    true,
		Path:    s.References[0],
		Loading: true,
		Strokes: cloneStrokes(s.MaskStrokes),
		Width:   width,
	}
	return true
}

// MaskLoaded hands the decoded reference to the editor opened for path. A
// nil image marks it failed.
func (s *State) MaskLoaded(path string, img ui.ImageSource, w, h int) {
	m := &s.Mask
	if !m.Open || m.Path != path {
		return
	}
	m.Loading = false
	if img == nil || w <= 0 || h <= 0 {
		m.Failed = true
		return
	}
	m.Image, m.W, m.H = img, w, h
	m.scale = min(1, float32(maskPreviewEdge)/float32(max(w, h)))
	m.cov = image.NewAlpha(image.Rect(0, 0, max(1, int(float32(w)*m.scale)), max(1, int(float32(h)*m.scale))))
	core.PaintStrokes(m.cov, m.Strokes, m.scale)
	m.dirty = true
}

// ApplyMask keeps the editor's strokes as the draft's mask and closes it.
func (s *State) ApplyMask() {
	if s == nil || !s.Mask.Open {
		return
	}
	if len(s.References) > 0 && s.References[0] == s.Mask.Path {
		s.MaskStrokes = nil
		if core.HasMask(s.Mask.Strokes) {
			s.MaskStrokes = cloneStrokes(s.Mask.Strokes)
		}
	}
	s.CloseMask()
}

// CloseMask closes the editor without changing the draft's mask, keeping
// the brush width for the next time.
func (s *State) CloseMask() {
	if s == nil {
		return
	}
	s.Mask = MaskEditor{Width: s.Mask.Width}
}

// Press starts a stroke at p, in the image's pixels.
func (m *MaskEditor) Press(p core.MaskPoint) {
	if m.cov == nil {
		return
	}
	m.drawing = true
	stroke := core.MaskStroke{Erase: m.Erase, Width: core.ClampBrushWidth(m.Width), Points: []core.MaskPoint{p}}
	m.Strokes = append(m.Strokes, stroke)
	core.PaintSegment(m.cov, p, p, stroke.Width, stroke.Erase, m.scale)
	m.dirty = true
}

// Move extends the stroke being drawn to p. Samples closer than a pixel to
// the last are dropped, so a resting pointer adds nothing.
func (m *MaskEditor) Move(p core.MaskPoint) {
	if !m.drawing || len(m.Strokes) == 0 {
		return
	}
	s := &m.Strokes[len(m.Strokes)-1]
	last := s.Points[len(s.Points)-1]
	if dx, dy := p.X-last.X, p.Y-last.Y; dx*dx+dy*dy < 1 {
		return
	}
	s.Points = append(s.Points, p)
	core.PaintSegment(m.cov, last, p, s.Width, s.Erase, m.scale)
	m.dirty = true
}

// Release ends the stroke being drawn.
func (m *MaskEditor) Release() { m.drawing = false }

// Drawing reports whether a stroke is being drawn.
func (m *MaskEditor) Drawing() bool { return m.drawing }

// Undo removes the last stroke.
func (m *MaskEditor) Undo() {
	if len(m.Strokes) == 0 {
		return
	}
	m.drawing = false
	m.Strokes = m.Strokes[:len(m.Strokes)-1]
	m.repaint()
}

// Clear removes every stroke.
func (m *MaskEditor) Clear() {
	m.drawing = false
	m.Strokes = nil
	m.repaint()
}

// Painted reports whether the strokes paint any area.
func (m *MaskEditor) Painted() bool { return core.HasMask(m.Strokes) }

func (m *MaskEditor) repaint() {
	if m.cov == nil {
		return
	}
	clear(m.cov.Pix)
	core.PaintStrokes(m.cov, m.Strokes, m.scale)
	m.dirty = true
}

// Overlay returns the preview of the painted area in tint, refreshed when
// the strokes or the tint changed. It is called on the UI thread while
// drawing: the bitmap shows one of two buffers while the other is written,
// as Bitmap.Update requires.
func (m *MaskEditor) Overlay(tint ui.Color) *ui.Bitmap {
	if m.cov == nil {
		return nil
	}
	if tint != m.tint {
		m.tint, m.dirty = tint, true
	}
	if !m.dirty && m.bitmap != nil {
		return m.bitmap
	}
	back := 1 - m.front
	b := m.cov.Bounds()
	if m.overlay[back] == nil || m.overlay[back].Bounds() != b {
		m.overlay[back] = image.NewRGBA(b)
	}
	dst := m.overlay[back]
	a := uint32(tint.A)
	r, g, bl := uint8(uint32(tint.R)*a/255), uint8(uint32(tint.G)*a/255), uint8(uint32(tint.B)*a/255)
	for i, c := range m.cov.Pix {
		o := i * 4
		if c == 0 {
			dst.Pix[o], dst.Pix[o+1], dst.Pix[o+2], dst.Pix[o+3] = 0, 0, 0, 0
			continue
		}
		dst.Pix[o], dst.Pix[o+1], dst.Pix[o+2], dst.Pix[o+3] = r, g, bl, tint.A
	}
	if m.bitmap == nil {
		m.bitmap = ui.NewBitmap(dst)
	} else {
		m.bitmap.Update(dst)
	}
	m.front, m.dirty = back, false
	return m.bitmap
}

func cloneStrokes(strokes []core.MaskStroke) []core.MaskStroke {
	out := slices.Clone(strokes)
	for i := range out {
		out[i].Points = slices.Clone(out[i].Points)
	}
	return out
}
