package kit

import (
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// Ghost is Kit's ghost button with Sailry's subtle_button states:
// transparent, accent on hover, secondary_active when pressed or selected. It
// is a MyGo ButtonBase, which keeps focus, keys and accessibility.
func Ghost(c *ui.Context, label string, selected bool) ui.Element {
	// ButtonBase draws no text of its own: Label only names the control for
	// assistive technology, so the visible label is a child element.
	return GhostLook(c, ui.ButtonBase(c).Label(label), selected).
		Children(func() { ui.Text(c, label).FontSize(14).SingleLine() })
}

// GhostLook gives a button without a look Kit's ghost states, as Ghost.
func GhostLook(c *ui.Context, b ui.Element, selected bool) ui.Element {
	p := theme.Of(c)
	b.Radius(p.Radius).Center().TextColor(p.Foreground).Cursor(ui.CursorPointer)
	switch {
	case b.Pressed() || selected:
		b.Background(p.Active)
	case b.Hovered():
		b.Background(p.Hover)
	}
	return b
}

// IconButton is a square ghost button holding one icon, named by label for
// its tooltip and assistive technology. Kit sizes: xsmall 20, small 28,
// default 32, rail 36; icons are 16 (14 below 24).
func IconButton(c *ui.Context, icon *ui.SVG, label string, size float32, selected bool) ui.Element {
	return IconButtonHint(c, icon, label, "", size, selected)
}

// IconButtonHint is an IconButton whose tooltip also shows the shortcut
// that does the same, hint.
func IconButtonHint(c *ui.Context, icon *ui.SVG, label, hint string, size float32, selected bool) ui.Element {
	return IconLook(c, ui.ButtonBase(c), icon, label, hint, size, selected)
}

// IconLook gives a button without a look, as a MenuButton's trigger, the
// look of an IconButtonHint.
func IconLook(c *ui.Context, b ui.Element, icon *ui.SVG, label, hint string, size float32, selected bool) ui.Element {
	tip := label
	if hint != "" {
		tip += " " + hint
	}
	Tooltip(c, GhostLook(c, b.Label(label), selected).Size(size, size), tip)
	glyph := float32(15)
	if size < 24 {
		glyph = 14
	}
	return b.Children(func() { ui.Icon(c, icon).FontSize(glyph) })
}

// Primary is Kit's primary button in the button.primary colors: the
// neutral foreground fill of the built-in theme.
func Primary(c *ui.Context, label string, disabled bool) ui.Element {
	// ButtonBase draws no text: Label only names the control for assistive
	// technology, so the visible caption is a child element.
	return PrimaryLook(c, ui.ButtonBase(c), label, disabled).
		Children(func() { ui.Text(c, label).FontSize(14).SingleLine() })
}

// PrimaryLook gives b, a button of any kind, Primary's look and no caption, for
// a caller that draws its own content (PrimaryBusy's busy mark and label).
func PrimaryLook(c *ui.Context, b ui.Element, label string, disabled bool) ui.Element {
	p := theme.Of(c)
	b = b.Label(label).Disabled(disabled).DimDisabled(false).Radius(p.Radius).Center().Gap(6).
		PaddingX(12).Height(32).TextColor(p.OnPrimary).Background(p.ButtonPrimary)
	switch {
	case disabled && !p.Dark:
		b.Background(p.Active).TextColor(p.MutedForeground.Alpha(0.5))
	case disabled:
		b.Background(p.ButtonPrimary.Alpha(0.15)).TextColor(p.MutedForeground.Alpha(0.5))
	case b.Pressed():
		b.Background(p.ButtonPrimaryActive)
	case b.Hovered():
		b.Background(p.ButtonPrimaryHi)
	}
	return b
}

// Tag is Kit's small Tag: a rounded label in the emphasis color at 12%.
func Tag(c *ui.Context, text string) ui.Element {
	p := theme.Of(c)
	return ui.Text(c, text).FontSize(12).SingleLine().Padding(1, 6).Radius(999).
		Background(p.Primary.Alpha(0.12)).TextColor(p.Primary)
}

// Tone selects a semantic color role from theme.Palette.
type Tone uint8

const (
	ToneNeutral Tone = iota
	TonePrimary
	ToneSuccess
	ToneWarning
	ToneDanger
)

// ToneColors maps a semantic Tone onto foreground and soft-fill background
// colors from the active palette.
func ToneColors(p *theme.Palette, tone Tone) (fg, bg ui.Color) {
	switch tone {
	case TonePrimary:
		return p.Primary, p.Primary.Alpha(0.12)
	case ToneSuccess:
		return p.Success, p.Success.Alpha(0.12)
	case ToneWarning:
		return p.Warning, p.Warning.Alpha(0.14)
	case ToneDanger:
		return p.Danger, p.Danger.Alpha(0.12)
	default:
		return p.MutedForeground, p.Muted
	}
}

// StatusPill draws a rounded semantic badge with an optional leading icon.
func StatusPill(c *ui.Context, text string, icon *ui.SVG, tone Tone, compact bool) ui.Element {
	p := theme.Of(c)
	fg, bg := ToneColors(p, tone)
	fontSize, iconSize := float32(11), float32(11)
	padV, padH := float32(1), float32(6)
	if compact {
		fontSize, iconSize = 10, 10
		padV, padH = 0, 5
	}
	return ui.Row(c).Shrink(0).AlignItems(ui.Center).Gap(3).Padding(padV, padH).Radius(999).Background(bg).Children(func() {
		if icon != nil {
			ui.Icon(c, icon).FontSize(iconSize).Shrink(0).TextColor(fg)
		}
		ui.Text(c, text).FontSize(fontSize).FontWeight(500).SingleLine().Shrink(0).TextColor(fg)
	})
}

// StatusIndicator draws an inline semantic status readout (icon + text) with
// no pill fill, keeping dense rows legible without encoding meaning in color alone.
func StatusIndicator(c *ui.Context, text string, icon *ui.SVG, tone Tone, compact bool) ui.Element {
	p := theme.Of(c)
	fg, _ := ToneColors(p, tone)
	fontSize, iconSize := float32(11), float32(11)
	if compact {
		fontSize, iconSize = 10, 10
	}
	return ui.Row(c).Shrink(0).AlignItems(ui.Center).Gap(3).Children(func() {
		if icon != nil {
			ui.Icon(c, icon).FontSize(iconSize).Shrink(0).TextColor(fg)
		}
		ui.Text(c, text).FontSize(fontSize).FontWeight(500).SingleLine().Shrink(0).TextColor(fg)
	})
}

// Secondary is Kit's outline button: a 1 DIP border over the
// button.secondary fill, 32 high, its label 14.
func Secondary(c *ui.Context, label string) ui.Element {
	return SecondaryLook(c, ui.ButtonBase(c).Label(label)).Children(func() { ui.Text(c, label).FontSize(14).SingleLine() })
}

// PrimaryBusy is Primary with its label, and while the operation it started
// runs the busy mark (Busy) before the label and no press.
func PrimaryBusy(c *ui.Context, label string, disabled, busy bool) ui.Element {
	return PrimaryLook(c, ui.ButtonBase(c), label, disabled || busy).Busy(busy).Children(func() {
		if busy {
			Busy(c)
		}
		ui.Text(c, label).FontSize(14).SingleLine()
	})
}

// BusyAction holds an Action's place while its operation runs: the busy
// mark for the icon, named by label, and no press.
func BusyAction(c *ui.Context, label string) ui.Element {
	return ui.ButtonBase(c).Label(label).Disabled(true).Busy(true).Size(28, 28).Shrink(0).Center().
		Children(func() { Busy(c) })
}

// SecondaryBusy is Secondary while the operation it started runs: the busy
// mark before the label, as a plugin control shows its loading, and no
// press until the operation ends.
func SecondaryBusy(c *ui.Context, label string, busy bool) ui.Element {
	return SecondaryLook(c, ui.ButtonBase(c).Label(label).Disabled(busy).Busy(busy)).Children(func() {
		if busy {
			Busy(c)
		}
		ui.Text(c, label).FontSize(14).SingleLine()
	})
}

// SecondaryLook gives a button base Secondary's look, for a trigger with
// content of its own (a MenuButton's label and caret).
func SecondaryLook(c *ui.Context, b ui.Element) ui.Element {
	p := theme.Of(c)
	b.Radius(p.Radius).Center().Gap(6).PaddingX(12).Height(32).
		Border(1, p.Border).Background(p.ButtonSecondary).TextColor(p.ButtonSecondaryForeground).Cursor(ui.CursorPointer)
	switch {
	case b.Pressed():
		b.Background(p.ButtonSecondaryActive)
	case b.Hovered():
		b.Background(p.ButtonSecondaryHover)
	}
	return b
}

// Danger is Kit's danger button, for what removes something.
func Danger(c *ui.Context, label string) ui.Element {
	return dangerLook(c, label).Children(func() { ui.Text(c, label).FontSize(14).SingleLine() })
}

// DangerBusy is Danger, and while the removal it started runs the busy
// mark before its label and no press.
func DangerBusy(c *ui.Context, label string, busy bool) ui.Element {
	return dangerLook(c, label).Disabled(busy).Busy(busy).Children(func() {
		if busy {
			BusyIn(c, theme.Of(c).DangerForeground)
		}
		ui.Text(c, label).FontSize(14).SingleLine()
	})
}

// dangerLook is the destructive button without its content.
func dangerLook(c *ui.Context, label string) ui.Element {
	p := theme.Of(c)
	b := ui.ButtonBase(c).Label(label).Radius(p.Radius).Center().Gap(6).PaddingX(12).Height(32).
		Background(p.Danger).TextColor(p.DangerForeground).Cursor(ui.CursorPointer)
	if b.Hovered() || b.Pressed() {
		b.Background(p.Danger.Mix(p.Foreground, 0.12))
	}
	return b
}

// Tone colors a badge or a status dot.
type Tone int

const (
	Neutral Tone = iota
	Info
	Positive
	Caution
	Negative
)

func (t Tone) color(p *theme.Palette) ui.Color {
	switch t {
	case Info:
		return p.Primary
	case Positive:
		return p.Success
	case Caution:
		return p.Warning
	case Negative:
		return p.Danger
	}
	return p.MutedForeground
}

// Badge is a Tag in a tone: the tone's color on 12% of it.
func Badge(c *ui.Context, text string, tone Tone) ui.Element {
	col := tone.color(theme.Of(c))
	return ui.Text(c, text).FontSize(12).SingleLine().Padding(1, 6).Radius(999).
		Background(col.Alpha(0.12)).TextColor(col)
}

// StatusDot is an 8 DIP dot in a tone, named by label for assistive
// technology.
func StatusDot(c *ui.Context, tone Tone, label string) ui.Element {
	return ui.Box(c).Size(8, 8).Shrink(0).Radius(4).Background(tone.color(theme.Of(c))).Label(label)
}

// StatusRing is the status dot on a mark: the same 8 DIP dot in a 1 DIP
// ring of the color under it, so it stands apart from the mark.
func StatusRing(c *ui.Context, tone Tone, under ui.Color) ui.Element {
	return ui.Box(c).Size(10, 10).Shrink(0).Radius(5).Background(tone.color(theme.Of(c))).Border(1, under)
}
