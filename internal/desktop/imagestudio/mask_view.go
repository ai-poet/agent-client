package imagestudio

import (
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	core "github.com/ai-poet/cheaprouter-harness/internal/imagestudio"
	"github.com/egoist/mygo/ui"
)

const (
	maskBoxW float32 = 720
	maskBoxH float32 = 480
)

// maskDialog is the inpainting editor over the draft's first reference
// image: the painted area tinted over the picture, a brush and an eraser
// of one width, Undo and Clear, and Cancel or Apply.
//
// The canvas is drawn here rather than with kit.Image because the brush
// needs the picture's exact on-screen rectangle to map the pointer into
// the image's own pixels, and its preview overlays a bitmap the editor
// rewrites while the pointer drags.
func maskDialog(c *ui.Context, state *State) {
	m := &state.Mask
	if !m.Open {
		return
	}
	p := theme.Of(c)
	t := func(k string, args ...any) string { return i18n.T(c, k, args...) }
	open := true
	kit.Dialog(c.Key("mask-dialog"), &open, func() {
		ui.Column(c).Gap(4).PaddingEnd(32).Children(func() {
			ui.Text(c, t("image_studio_mask_title")).FontSize(16).FontWeight(600).TextColor(p.PopoverForeground)
			ui.Text(c, t("image_studio_mask_hint")).FontSize(12).TextColor(p.MutedForeground)
		})

		switch {
		case m.Loading:
			ui.Column(c.Key("mask-loading")).Size(maskBoxW, maskBoxH/2).Center().Children(func() { kit.Loader(c, 22) })
		case m.Failed:
			ui.Column(c.Key("mask-failed")).Size(maskBoxW, maskBoxH/2).Center().Gap(6).Children(func() {
				ui.Icon(c, icons.Image).FontSize(22).TextColor(p.MutedForeground)
				ui.Text(c, t("image_studio_file_missing")).FontSize(12).TextColor(p.MutedForeground)
			})
		default:
			maskCanvas(c, m, p)
		}

		ui.Row(c.Key("mask-tools")).FillWidth().Gap(12).AlignItems(ui.Center).Wrap().Children(func() {
			tool := 0
			if m.Erase {
				tool = 1
			}
			if kit.Segmented(c, "mask-tool", &tool, []string{t("image_studio_mask_brush"), t("image_studio_mask_eraser")},
				kit.SegmentedProps{Size: kit.SizeSmall, Icons: []*ui.SVG{icons.Brush, icons.Eraser}}).Changed() {
				m.Erase = tool == 1
			}
			ui.Row(c.Key("mask-width")).Gap(8).AlignItems(ui.Center).Width(220).Children(func() {
				ui.Text(c, t("image_studio_mask_width")).FontSize(12).TextColor(p.MutedForeground).Shrink(0)
				width := float64(m.Width)
				ui.Slider(c.Key("slider"), &width, core.MinBrushWidth, core.MaxBrushWidth).Step(1).
					Label(t("image_studio_mask_width")).Grow(1)
				m.Width = float32(width)
			})
			ui.Spacer(c)
			kit.IconButton(c.Key("undo"), icons.Undo, t("image_studio_mask_undo"), 28, false).
				Disabled(len(m.Strokes) == 0).OnClick(m.Undo)
			kit.TextAction(c.Key("clear"), t("image_studio_mask_clear"), len(m.Strokes) == 0).OnClick(m.Clear)
		})

		ui.Row(c.Key("footer")).Gap(8).Justify(ui.End).Children(func() {
			kit.Secondary(c.Key("cancel"), t("settings_cancel")).OnClick(state.CloseMask)
			kit.Primary(c.Key("apply"), t("image_studio_mask_apply"), m.Loading || m.Failed).OnClick(state.ApplyMask)
		})
	})
	if !open {
		state.CloseMask()
	}
}

// maskCanvas draws the picture fitted in the editor's box with the painted
// area over it and the brush's outline under the pointer, and paints while
// the pointer presses it.
func maskCanvas(c *ui.Context, m *MaskEditor, p *theme.Palette) {
	scale := min(maskBoxW/float32(m.W), maskBoxH/float32(m.H))
	dw, dh := float32(m.W)*scale, float32(m.H)*scale
	ui.Row(c.Key("mask-stage")).Width(maskBoxW).Height(maskBoxH).Justify(ui.Center).AlignItems(ui.Center).
		Radius(p.Radius).Background(p.Muted).Children(func() {
		box := ui.Box(c.Key("mask-canvas")).Size(dw, dh).Cursor(ui.CursorCrosshair).Label(i18n.T(c, "image_studio_mask_title"))
		x, y, over := box.PointerPosition()
		at := core.MaskPoint{
			X: min(max(x/scale, 0), float32(m.W)),
			Y: min(max(y/scale, 0), float32(m.H)),
		}
		switch pressed := box.Pressed(); {
		case pressed && !m.Drawing():
			m.Press(at)
		case pressed:
			m.Move(at)
		case m.Drawing():
			m.Release()
		}
		tint := p.Danger.Alpha(0.45)
		overlay := m.Overlay(tint)
		radius := core.ClampBrushWidth(m.Width) * scale / 2
		box.Draw(func(pt *ui.Painter, r ui.Rect) {
			pt.Image(m.Image, r, ui.FillBox)
			if overlay != nil {
				pt.Image(overlay, r, ui.FillBox)
			}
			if over {
				ring := ui.Rect{X: r.X + x - radius, Y: r.Y + y - radius, W: 2 * radius, H: 2 * radius}
				pt.Stroke(ring, p.Background, radius, 2)
				pt.Stroke(ring, p.Foreground, radius, 1)
			}
		})
	})
}
