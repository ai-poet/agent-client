package composer

import (
	"fmt"
	"maps"
	"slices"
	"strconv"
	"strings"
	"unicode"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// The model chip and its controls: the chip names the provider, the model,
// its effort and its features, or the team; its popover holds the team row,
// the features, the effort and its track, and pages for the model picker and
// the team list.

// Chip widths.
const (
	chipWidth        = 260
	compactChip      = 192
	narrowChip       = 140
	tightChipMax     = 128
	tightChipMin     = 88
	tightChipReserve = 148
	panelWidth       = 296
	panelHeight      = 320
	member           = 24
	teamRow          = 40
)

// The model picker has a gap after the rail and a hairline with two gaps
// between the models and the efforts. The list is what the page has beside
// the others: listWidth where the window has room for it. Below
// compactPickerMin the vendor rail shrinks to icons alone so the model list
// keeps room to read and select models even at narrow widths.
const (
	railTile         = 32
	railIcon         = 18
	railPad          = 6
	railWidth        = 136
	compactRailWidth = railTile + 2*railPad
	compactPickerMin = 420
	listWidth        = 236
	effortWidth      = 132
	columnGap        = 8
	pickerWidth      = railWidth + listWidth + effortWidth + 3*columnGap + 1
	// The model controls use the same inset and gap on every page.
	modelPopoverPadding = 12
	modelPopoverGap     = 8
	// windowInset is how far a popover stays from the window's sides.
	windowInset = 8
	// maxSearchResults caps rendered search hits in the model picker scroll list.
	maxSearchResults = 50
)

// effortState is the model picker's effort column: the model whose efforts
// it lists and how a click and the keys reach it.
type effortState struct {
	// model is the model whose efforts show: the one whose row was clicked
	// or that the keys were on last, the chosen one until then.
	model string
	// focused is the model row that had the keyboard focus last frame, so
	// the focus takes the column once, as it comes to a row.
	focused string
	// enter gives the chosen model's row the focus as the picker opens,
	// back gives the shown model's row the focus the column had, and open
	// gives the column the focus once a model's row opened it.
	enter, back, open bool
}

// layer is the page the model popover shows.
type layer int

const (
	layerMain layer = iota
	layerModels
	layerTeams
)

// team returns the team the draft runs as, nil for the model alone.
func (s *State) team(ctx *Context) *Team {
	for i := range ctx.Teams {
		if ctx.Teams[i].ID == s.Team {
			return &ctx.Teams[i]
		}
	}
	return nil
}

// Badge is a provider's initials: the first letter of each word, of camel-
// case humps too, up to three; a single word gives its first two letters; CJK
// names keep two characters.
func Badge(name string) string {
	var words [][]rune
	var cur []rune
	flush := func() {
		if len(cur) > 0 {
			words = append(words, cur)
			cur = nil
		}
	}
	rs := []rune(name)
	for i, r := range rs {
		switch {
		case !unicode.IsLetter(r) && !unicode.IsDigit(r):
			flush()
		case unicode.IsUpper(r) && i > 0 && unicode.IsLower(rs[i-1]):
			flush()
			cur = append(cur, r)
		default:
			cur = append(cur, r)
		}
	}
	flush()
	switch len(words) {
	case 0:
		return ""
	case 1:
		w := words[0]
		return strings.ToUpper(string(w[:min(2, len(w))]))
	}
	var b []rune
	for _, w := range words[:min(3, len(words))] {
		b = append(b, unicode.ToUpper(w[0]))
	}
	return string(b)
}

// modelDisplayName returns the user-facing model label, falling back to its ID.
func modelDisplayName(m Model) string {
	if name := strings.TrimSpace(m.Name); name != "" {
		return name
	}
	return strings.TrimSpace(m.ID)
}

// shortModelName removes any leading vendor or provider namespace prefix so
// narrow chips keep the model identifier legible.
func shortModelName(m Model) string {
	name := modelDisplayName(m)
	if i := strings.LastIndex(name, "/"); i >= 0 && i+1 < len(name) {
		name = name[i+1:]
	}
	if _, bare, ok := strings.Cut(name, "::"); ok && bare != "" {
		name = bare
	}
	return name
}

// modelChipWidth returns the maximum chip width for the measured toolbar width.
func modelChipWidth(toolbarWidth float32) float32 {
	switch {
	case toolbarWidth <= 0 || toolbarWidth >= compactWidth:
		return chipWidth
	case toolbarWidth >= iconWidth:
		return compactChip
	case toolbarWidth >= tightWidth:
		return narrowChip
	default:
		return min(tightChipMax, max(tightChipMin, toolbarWidth-tightChipReserve))
	}
}

// modelChip is model_menu: the round chip that opens the model controls
// above it, right-aligned (Anchor::BottomRight, bottom_2, p_3). It stays
// visible at every toolbar width, tightening its presentation as the composer
// narrows.
func (s *State) modelChip(c *ui.Context, ctx *Context, actions Actions) {
	p := theme.Of(c)
	m := s.Chosen(ctx.Models)
	compact := s.width > 0 && s.width < compactWidth
	narrow := s.width > 0 && s.width < iconWidth
	tight := s.width > 0 && s.width < tightWidth
	width := modelChipWidth(s.width)
	padX, gap := float32(10), float32(8)
	if tight {
		padX, gap = 8, 4
	} else if compact {
		padX, gap = 8, 6
	}
	textSize := float32(14)
	if tight {
		textSize = 13
	}
	if len(ctx.Models) == 0 {
		// model_label without a configuration: the chip reads Select model
		// and opens the Node's model settings.
		label := i18n.T(c, "composer_select_model")
		chip := kit.GhostLook(c, ui.ButtonBase(c.Key("model")), false).Label(label).Height(32).PaddingX(padX).Gap(gap).Radius(16).
			MaxWidth(width).MinWidth(0).Shrink(1).Disabled(ctx.Readonly || ctx.SelectModel == nil)
		if compact {
			kit.Tooltip(c, chip, label)
		}
		chip.Children(func() {
			ui.Text(c, label).FontSize(textSize).LineHeight(20.0 / textSize).SingleLine().Shrink(1).MinWidth(0)
			if compact {
				ui.Icon(c, icons.ChevronDown).FontSize(12).Shrink(0).TextColor(p.MutedForeground)
			}
		}).OnClick(func() {
			s.settings = false
			if ctx.SelectModel != nil {
				ctx.SelectModel()
			}
		})
		return
	}
	team := s.team(ctx)
	effort := ""
	if len(efforts(m)) > 0 {
		effort = s.EffortName(c, m)
	}
	displayName := modelDisplayName(m)
	shownName := displayName
	if narrow {
		shownName = shortModelName(m)
	}
	lead := displayName
	if effort != "" {
		lead += " · " + effort
	}
	label := i18n.T(c, "composer_model_settings") + ": " + displayName
	if team != nil {
		label = i18n.T(c, "composer_model_settings") + ": " + team.Name + " · " + lead
	}
	fast, other := s.chosenFeatures(c, m)
	for _, f := range append(fast, other...) {
		label += " · " + f.label
	}
	chip := kit.GhostLook(c, ui.ButtonBase(c.Key("model")), s.picker).Label(label).Height(32).PaddingX(padX).Gap(gap).Radius(16).
		MaxWidth(width).MinWidth(0).Shrink(1).Disabled(ctx.Readonly)
	if team != nil {
		kit.Tooltip(c, chip, lead)
	} else if compact || (m.Group != nil && m.Group.Summary != "") {
		tip := lead
		if m.Provider != "" {
			tip = m.Provider + " · " + lead
		}
		if m.Group != nil && m.Group.Summary != "" {
			tip += " · " + m.Group.Summary
		}
		kit.Tooltip(c, chip, tip)
	}
	badge := func(f featureBadge) {
		if f.icon != nil {
			kit.Tooltip(c, ui.Icon(c, f.icon).FontSize(16).Shrink(0).TextColor(p.Foreground), f.label)
		} else {
			kit.Tooltip(c, ui.Text(c, f.label).FontSize(14).SingleLine().Shrink(0).TextColor(p.Primary), f.label)
		}
	}
	chip.HasPopup(ui.PopupDialog).Expanded(s.picker).Children(func() {
		if team == nil && !compact {
			kit.Tooltip(c, kit.Tag(c, Badge(m.Provider)).Shrink(0), m.Provider)
		}
		if !tight {
			for _, f := range fast {
				badge(f)
			}
		}
		if team != nil {
			if !tight {
				avatarSize := float32(20)
				if compact {
					avatarSize = 18
				}
				teamAvatar(c, *team, avatarSize)
			}
			ui.Text(c, team.Name).FontSize(textSize).SingleLine().Shrink(1).MinWidth(0)
		} else {
			ui.Text(c, shownName).FontSize(textSize).SingleLine().Shrink(1).MinWidth(0)
			if effort != "" && !tight {
				if narrow {
					kit.Tooltip(c, ui.Icon(c, icons.Brain).FontSize(16).Shrink(0).TextColor(p.MutedForeground), effort)
				} else {
					ui.Text(c, effort).FontSize(14).SingleLine().Shrink(0).TextColor(p.MutedForeground)
				}
			}
		}
		if !tight {
			for _, f := range other {
				if !compact || f.icon != nil {
					badge(f)
				}
			}
		}
		if compact {
			ui.Icon(c, icons.ChevronDown).FontSize(12).Shrink(0).TextColor(p.MutedForeground)
		}
	}).OnClick(func() {
		s.settings = false
		s.picker, s.layer = !s.picker, layerMain
	})
	panel := kit.Popover(c, chip, &s.picker, func() { s.controls(c, ctx, actions, chip.Bounds()) })
	if panel != (ui.Element{}) {
		panel.AttachTo(chip, ui.AnchorTopEnd, ui.AnchorBottomEnd).Margin(0, 0, modelPopoverGap, 0).
			Padding(modelPopoverPadding).Width(s.popoverWidth(c, modelPopoverPadding))
		s.sizePopover(panel)
	}
}

// sizePopover gives a popover of the controls the model picker's size in
// the frame the picker shows: a popover easing to it, as it eases between
// its other pages and back from this one, would have the picker's columns
// and rows past its edge on the way, where a press closes the popover.
func (s *State) sizePopover(panel ui.Element) {
	if s.layer == layerModels {
		kit.Snapped(panel)
	}
}

// popoverWidth is how wide a popover of the controls is, pad inside its
// edge: the controls' page and the padding, and for the model picker its
// three columns, the padding and the edge, as far as the window has room.
// The picker's page fills the popover (controls). In a window too narrow
// for the three columns the rail and the efforts keep their widths and the
// list between them gives up its own.
func (s *State) popoverWidth(c *ui.Context, pad float32) float32 {
	w, _ := c.Size()
	limit := w - 2*windowInset
	width := panelWidth + 2*pad
	if w > 0 && limit > 0 {
		width = min(width, limit)
	}
	if s.layer != layerModels {
		return width
	}
	target := pickerWidth + 2*(pad+1)
	if w > 0 && limit > 0 {
		target = min(target, limit)
	}
	return max(width, target)
}

// featureBadge is a chosen feature on the chip: Fast's bolt, an option's
// icon, or its name in the emphasis color.
type featureBadge struct {
	label string
	icon  *ui.SVG
}

// chosenFeatures are the features set for the model, Fast first.
func (s *State) chosenFeatures(c *ui.Context, m Model) (fast, other []featureBadge) {
	for _, f := range m.Features {
		id, ok := s.Features[f.ID]
		if !ok {
			continue
		}
		for _, o := range f.Options {
			if o.ID != id {
				continue
			}
			if f.ID == "speed" && o.ID == "fast" {
				fast = append(fast, featureBadge{i18n.T(c, "composer_feature_fast"), icons.Zap})
				continue
			}
			other = append(other, featureBadge{o.Name, o.Icon})
		}
	}
	return fast, other
}

// controls is the popover's body: its height animates between the main
// page's and the team list's own (up to the picker's); the picker's page
// has its own at once.
func (s *State) controls(c *ui.Context, ctx *Context, actions Actions, anchor ui.Rect) {
	// The main page has the team row's divider (1, and a gap of 8); the
	// team list is as tall as its rows, the model picker the full height.
	h := float32(108 + teamRow + 9)
	if s.layer == layerMain {
		m := s.controlModel(s.controlModels(ctx))
		if len(efforts(m)) == 0 {
			h = float32(68 + teamRow + 9)
		}
		if m.Group != nil {
			h += 24
		}
	}
	switch s.layer {
	case layerModels:
		// The picker floats outside the composer. Size it against the
		// space on either side of its actual anchor, not a fixed allowance
		// for the composer that can cut off ordinary effort rows.
		_, wh := c.Size()
		room := max(anchor.Y, wh-anchor.Y-anchor.H)
		chrome := windowInset + modelPopoverGap + 2*(modelPopoverPadding+1)
		h = min(panelHeight, max(0, room-float32(chrome)))
	case layerTeams:
		// Back 28, the rows 32 each, the divider 1 + 8, Manage 32, 4 apart.
		rows := 1 + len(ctx.Teams)
		h = min(float32(28+32*rows+9+32+4*(rows+2)), panelHeight)
	}
	body := ui.Column(c.Key("model-controls")).Height(h).Clip().AlignItems(ui.Center).Gap(8).
		Disabled(ctx.Readonly || s.TeamLocked)
	if s.layer == layerModels {
		// The picker's page fills its popover and has its height at once,
		// as the popover does (sizePopover).
		body.FillWidth().MinWidth(0)
	} else {
		bodyW := float32(panelWidth)
		if w, _ := c.Size(); w > 0 {
			bodyW = min(bodyW, max(160, w-2*windowInset-24))
		}
		body.Width(bodyW).Transition(ui.ElementTransition{Height: true, Duration: 200e6})
	}
	body.Children(func() {
		switch s.layer {
		case layerModels:
			s.modelPicker(c, ctx)
		case layerTeams:
			s.teamList(c, ctx, actions)
		default:
			s.mainPanel(c, ctx)
		}
	})
}

// mainPanel is the controls' first page: the team row, then the
// features, the effort with its score and reset, the model to choose
// another, and the effort track.
func (s *State) mainPanel(c *ui.Context, ctx *Context) {
	p := theme.Of(c)
	m := s.controlModel(s.controlModels(ctx))
	effortIndex := s.controlEffort()
	list := efforts(m)
	s.teamRow(c, ctx)
	hasReasoning := len(list) > 0
	effort := ""
	if hasReasoning {
		effort = s.controlEffortName(c, m)
	}
	highest := len(list) > 1 && int(*effortIndex) == len(list)-1
	ui.Column(c.Key("controls")).FillWidth().Shrink(0).Gap(4).AlignItems(ui.Center).Children(func() {
		ui.Box(c.Key("effort-row")).FillWidth().Height(28).Children(func() {
			ui.Row(c.Key("features")).Absolute().Left(0).Top(0).Height(28).Gap(4).AlignItems(ui.Center).Children(func() {
				s.features(c, m)
			})
			if hasReasoning {
				// The effort stays centered; its score's help mark follows it.
				ui.Row(c.Key("effort-label")).FillWidth().Height(28).Center().Gap(4).Children(func() {
					e := ui.Text(c.Key("effort"), effort).FontSize(16).SingleLine().TextColor(p.Primary)
					if highest {
						e.TextColor(p.Warning)
					}
					if sc := measuredAt(m, effort); sc != nil {
						scoreHelp(c, sc)
					}
				})
			}
			ui.Row(c.Key("reset-row")).Absolute().Right(0).Top(0).Height(28).AlignItems(ui.Center).Children(func() {
				resettable := (hasReasoning && int(*effortIndex) != defaultEffort(m)) || len(s.controlFeatures()) > 0
				kit.IconButton(c.Key("reset"), icons.Restore, i18n.T(c, "composer_effort_reset"), 28, false).Radius(14).
					Disabled(!resettable).OnClick(func() {
					if hasReasoning {
						*effortIndex = float64(defaultEffort(m))
					}
					s.setControlFeatures(map[string]string{})
					s.markMemberOverride()
				})
			})
		})
		modelName := modelDisplayName(m)
		kit.GhostLook(c, ui.ButtonBase(c.Key("choose")).Label(modelName), false).Height(28).PaddingX(8).Gap(4).Radius(14).MaxWidth(panelWidth).Children(func() {
			ui.Text(c, modelName).FontSize(14).SingleLine().Shrink(1).MinWidth(0)
			ui.Icon(c, icons.ChevronRight).FontSize(12).TextColor(p.MutedForeground)
		}).OnClick(func() {
			s.layer, s.family, s.modelQuery = layerModels, vendorID(m), ""
			s.efforts = effortState{enter: true}
		})
		if m.Group != nil {
			modelGroupHintRow(c, p, "model-group-hint", m.Group)
		}
	})
	if hasReasoning {
		before := *effortIndex
		track(c, effortIndex, len(list), highest, len(list) < 2)
		if before != *effortIndex {
			s.markMemberOverride()
		}
	}
}

// defaultEffort is the level a model starts at: medium, or the middle.
func defaultEffort(m Model) int {
	list := efforts(m)
	if m.DefaultEffort != "" {
		for i, e := range list {
			if e == m.DefaultEffort {
				return i
			}
		}
	}
	for i, e := range list {
		if e == "medium" {
			return i
		}
	}
	return len(list) / 2
}

// features are Fast toggles, a one-option feature toggles, others choose from
// a menu with Off. A model without a speed feature shows Fast off, its
// tooltip saying it does not take it.
func (s *State) features(c *ui.Context, m Model) {
	speed := false
	for _, f := range m.Features {
		speed = speed || f.ID == "speed"
	}
	if !speed {
		kit.IconButton(c.Key("fast-unsupported"), icons.Zap, i18n.T(c, "composer_feature_fast_unsupported"), 28, false).
			Label(i18n.T(c, "composer_feature_fast")).Radius(14).Disabled(true)
	}
	for _, f := range m.Features {
		features := s.controlFeatures()
		current := features[f.ID]
		var opt *FeatureOption
		for i := range f.Options {
			if f.Options[i].ID == current {
				opt = &f.Options[i]
			}
		}
		set := opt != nil
		cycles := f.ID == "speed" || len(f.Options) == 1
		name := func(o FeatureOption) string {
			if f.ID == "speed" && o.ID == "fast" {
				return i18n.T(c, "composer_feature_fast")
			}
			if o.Name != "" {
				return o.Name
			}
			return o.ID
		}
		label := f.Name
		icon := f.Icon
		if cycles {
			switch {
			case opt != nil:
				label = name(*opt)
			case len(f.Options) > 0:
				label = name(f.Options[0])
			}
			if f.ID == "speed" {
				icon = icons.Zap
			} else if opt != nil && opt.Icon != nil {
				icon = opt.Icon
			}
		}
		if icon == nil {
			icon = icons.Settings2
		}
		tip := label + " · " + i18n.T(c, "composer_feature_description")
		if cycles && len(f.Options) > 1 && !set {
			tip = i18n.T(c, "composer_feature_off") + " · " + label
		}
		if cycles {
			b := kit.IconButton(c.Key(f.ID), icon, tip, 28, set).Label(label).Radius(14)
			b.AccessSelected(set).OnClick(func() {
				s.markMemberFeatureOverride()
				// off, then each option in turn.
				i := 0
				if set {
					i = len(f.Options)
					for j, o := range f.Options {
						if o.ID == current {
							i = j + 1
						}
					}
				}
				if i < len(f.Options) {
					features[f.ID] = f.Options[i].ID
				} else {
					delete(features, f.ID)
				}
			})
			continue
		}
		kit.MenuButton(c.Key(f.ID), s.menuID("feature-"+f.ID), func(b ui.Element) {
			kit.IconLook(c, b, icon, tip, "", 28, set).Label(label).Radius(14)
		}, func(menu *kit.Menu) {
			if menu.Item(i18n.T(c, "composer_feature_off")).Checked(!set).Chosen() {
				delete(features, f.ID)
				s.markMemberFeatureOverride()
			}
			for _, o := range f.Options {
				if menu.Item(name(o)).Checked(set && o.ID == current).Chosen() {
					features[f.ID] = o.ID
					s.markMemberFeatureOverride()
				}
			}
		})
	}
}

// teamRow is the execution's avatar opening the team list, and, right-
// aligned, the lead and the team's members, the one whose model the controls
// set full, the others faded.
func (s *State) teamRow(c *ui.Context, ctx *Context) {
	p := theme.Of(c)
	team := s.team(ctx)
	name := i18n.T(c, "composer_execution_model")
	if team != nil {
		name = team.Name
	}
	ui.Row(c.Key("team-row")).FillWidth().MinWidth(0).Height(32).Shrink(0).Gap(8).AlignItems(ui.Center).Children(func() {
		b := kit.GhostLook(c, ui.ButtonBase(c.Key("team")), false).Label(name)
		kit.Tooltip(c, b, name)
		b.Height(member).PaddingStart(0).PaddingEnd(4).Gap(2).Radius(member / 4).Children(func() {
			if team != nil {
				teamAvatar(c, *team, member)
			} else {
				avatar(c, icons.Bot, p.Primary, member, true, false)
			}
			ui.Icon(c, icons.ChevronRight).FontSize(12)
		}).OnClick(func() { s.layer = layerTeams })
		ui.Spacer(c)
		if team == nil {
			return
		}
		lead := i18n.T(c, "composer_team_lead")
		memberButton(c, "lead", lead, func() ui.Element { return kit.LeadAvatar(c, member) }, false, s.member == "", func() { s.member = "" })
		for _, m := range team.Members {
			memberButton(c, m.ID, m.Name, func() ui.Element { return kit.RoleAvatar(c, m.ID, m.Avatar, member) }, m.Overridden, s.member == m.ID,
				func() { s.member = m.ID })
		}
	})
	// A divider under the row, inside the panel's padding, so it stops
	// short of the panel's edges.
	ui.Box(c.Key("team-divider")).FillWidth().Height(1).Shrink(0).Background(p.Border)
}

// memberButton is a member of the row: its avatar (face), faded unless
// selected, with a dot where its model was set apart.
func memberButton(c *ui.Context, key, name string, face func() ui.Element, overridden, selected bool, pick func()) {
	p := theme.Of(c)
	b := ui.ButtonBase(c.Key("member-" + key)).Label(name)
	kit.Tooltip(c, b, name)
	b.Size(member, member).Radius(member / 4).Cursor(ui.CursorPointer).AccessSelected(selected)
	b.Children(func() {
		ui.Box(c).Size(member, member).Shrink(0).Children(func() {
			if drawn := face(); !selected {
				drawn.Opacity(0.35)
			}
			if overridden {
				ui.Box(c).Absolute().Top(-1).Right(-1).Size(8, 8).Radius(4).Background(p.Primary).Border(1, p.Floating())
			}
		})
	}).OnClick(pick)
}

// teamAvatar is a team's picture (kit.TeamAvatar).
func teamAvatar(c *ui.Context, team Team, size float32) ui.Element {
	return kit.TeamAvatar(c, team.ID, team.Avatar, size)
}

// avatar is the identity's icon on its fill, faded unless selected, with a
// dot where the model was set apart. Its tile is the rounded square of Kit's
// avatars, so it lines up with the roles' and the teams'.
func avatar(c *ui.Context, icon *ui.SVG, fill ui.Color, size float32, selected, overridden bool) {
	p := theme.Of(c)
	a := float32(1)
	if !selected {
		a = 0.35
	}
	ui.Box(c).Size(size, size).Shrink(0).Radius(size / 4).Background(fill.Alpha(a)).Center().Children(func() {
		glyph := ui.Icon(c, icon).FontSize(size * 0.58).TextColor(p.PrimaryForeground)
		if !selected {
			glyph.Opacity(0.6)
		}
		if overridden {
			ui.Box(c).Absolute().Top(-1).Right(-1).Size(8, 8).Radius(4).Background(p.Primary).Border(1, p.Floating())
		}
	})
}

// teamList is Model only, the default team and the teams with their member
// counts, then Manage teams. The chosen one is told as the picker's rows
// are, by the neutral Palette.Selected alone, with no check; assistive
// technology has it as selected.
func (s *State) teamList(c *ui.Context, ctx *Context, actions Actions) {
	p := theme.Of(c)
	row := func(key, name string, face func(), count int, selected bool, pick func()) {
		b := kit.GhostLook(c, ui.ButtonBase(c.Key(key)).Label(name), false).FillWidth().Height(32).PaddingX(8).Gap(8).Justify(ui.Start).
			AccessSelected(selected)
		if selected {
			b.Background(p.Selected())
		}
		b.Children(func() {
			face()
			ui.Text(c, name).FontSize(14).SingleLine().Grow(1).Shrink(1).MinWidth(0)
			if count >= 0 {
				ui.Textf(c, "%d", count).FontSize(12).Shrink(0).TextColor(p.MutedForeground)
			}
		}).OnClick(pick)
	}
	ui.Scroll(c.Key("teams")).FillWidth().Grow(1).Gap(4).Children(func() {
		kit.GhostLook(c, ui.ButtonBase(c.Key("back")).Label(i18n.T(c, "composer_back")), false).Height(28).PaddingX(8).Gap(6).Justify(ui.Start).
			Children(func() {
				ui.Icon(c, icons.ArrowLeft).FontSize(14)
				ui.Text(c, i18n.T(c, "composer_back")).FontSize(14)
			}).OnClick(func() { s.layer = layerMain })
		row("model-only", i18n.T(c, "composer_execution_model"), func() { avatar(c, icons.Bot, p.Primary, member, true, false) }, -1, s.Team == "", func() {
			if s.SelectTeam != nil && !s.SelectTeam("") {
				return
			}
			s.Team, s.member, s.layer = "", "", layerMain
			s.MemberModels = nil
		})
		for _, t := range ctx.Teams {
			row("team-"+t.ID, t.Name, func() { teamAvatar(c, t, member) }, len(t.Members), s.Team == t.ID, func() {
				if s.SelectTeam != nil {
					if !s.SelectTeam(t.ID) {
						return
					}
				} else if s.Team != t.ID {
					s.MemberModels = nil
				}
				s.Team, s.member, s.layer = t.ID, "", layerMain
			})
		}
		ui.Box(c).FillWidth().Height(1).Margin(4, 0).Background(p.Border).Shrink(0)
		kit.GhostLook(c, ui.ButtonBase(c.Key("manage")).Label(i18n.T(c, "composer_team_manage")), false).FillWidth().Height(32).PaddingX(8).Gap(8).
			Justify(ui.Start).TextColor(p.MutedForeground).Children(func() {
			ui.Box(c).Size(member, member).Shrink(0).Center().Children(func() { ui.Icon(c, icons.Settings).FontSize(16) })
			ui.Text(c, i18n.T(c, "composer_team_manage")).FontSize(14).SingleLine()
		}).OnClick(actions.ManageTeams)
	})
}

func providerExpandKey(family, providerKey string) string {
	return family + "\x00" + providerKey
}

func (s *State) setProviderExpanded(family, providerKey string, open bool) {
	if s.expandedProviders == nil {
		s.expandedProviders = map[string]bool{}
	}
	s.expandedProviders[providerExpandKey(family, providerKey)] = open
}

func (s *State) providerGroupOpen(family string, g providerGroup, index int, hasChosenGroup, queryActive, enter bool) bool {
	if queryActive {
		return true
	}
	key := providerExpandKey(family, g.Key)
	if enter && g.ContainsChosen {
		s.setProviderExpanded(family, g.Key, true)
		return true
	}
	if open, ok := s.expandedProviders[key]; ok {
		return open
	}
	if hasChosenGroup {
		return g.ContainsChosen
	}
	return index == 0
}

func providerMark(c *ui.Context, icon *ui.SVG, name string, size float32, color ui.Color) {
	if icon != nil {
		ui.Icon(c, icon).FontSize(size).Shrink(0).TextColor(color)
		return
	}
	initial := []rune(strings.TrimSpace(name))
	if len(initial) == 0 {
		return
	}
	ui.Row(c).Size(size, size).Shrink(0).Radius(4).Center().Background(theme.Of(c).Secondary).Children(func() {
		ui.Text(c, string(unicode.ToUpper(initial[0]))).FontSize(size * 0.62).FontWeight(600).SingleLine().TextColor(color)
	})
}

type providerHeader struct {
	el   ui.Element
	key  string
	open bool
}

// modelPicker shows vendors and models, with the existing contextual effort
// controls next to the model list. Search ignores the current vendor.
func (s *State) modelPicker(c *ui.Context, ctx *Context) {
	p := theme.Of(c)
	e := &s.efforts
	models := s.controlModels(ctx)
	chosen := s.controlModel(models)
	vendors, names := pickerVendors(c, models, ctx.Families)
	byID := make(map[string]Model, len(models))
	for _, m := range models {
		byID[m.ID] = m
	}
	var recent []Model
	recentSet := make(map[string]bool, len(ctx.RecentModels))
	for _, id := range ctx.RecentModels {
		if m, ok := byID[id]; ok {
			recent = append(recent, m)
			recentSet[id] = true
		}
	}
	if s.family == recentVendor && len(recent) == 0 || s.family != recentVendor && names[s.family] == "" {
		s.family = vendorID(chosen)
	}
	search := i18n.T(c, "search")
	field := ui.SearchField(c.Key("model-query"), &s.modelQuery).Label(search).Placeholder(search).
		FillWidth().Height(32).Shrink(0)
	query := strings.TrimSpace(s.modelQuery)
	shown, visible := matchingModels(models, names, query, s.family)
	isRecent := query == "" && s.family == recentVendor
	if isRecent {
		shown = recent
	}
	var groups []providerGroup
	truncated := false
	if !isRecent {
		groups = groupModelsByProvider(shown, chosen.ID, recentSet)
		if query != "" {
			groups, truncated = truncateProviderGroups(groups, maxSearchResults)
		}
	}
	grouped := !isRecent && len(groups) >= 2
	var firstModel Model
	switch {
	case isRecent && len(shown) > 0:
		firstModel = shown[0]
	case !isRecent && len(groups) > 0 && len(groups[0].Models) > 0:
		firstModel = groups[0].Models[0]
	}
	if field.Submitted() && firstModel.ID != "" {
		effort := defaultEffort(firstModel)
		if firstModel.ID == chosen.ID {
			effort = s.carried(firstModel, chosen)
		}
		s.pick(firstModel, effort)
		c.Invalidate()
	}
	line := func(key string) {
		ui.Box(c.Key(key)).Width(1).Shrink(0).Background(p.Border)
	}
	windowW, _ := c.Size()
	compactRail := windowW > 0 && windowW < compactPickerMin
	vendorsW := float32(railWidth)
	if compactRail {
		vendorsW = compactRailWidth
	}
	multiProviderCatalog := hasMultipleProviders(models)
	picker := ui.Row(c.Key("model-picker")).FillWidth().Grow(1).MinHeight(0).Gap(columnGap).AlignItems(ui.Stretch)
	picker.Children(func() {
		ui.Scroll(c.Key("vendors")).Width(vendorsW).Shrink(0).Padding(railPad).Gap(4).Radius(p.Radius2xl()).Background(p.Secondary).
			FocusGroup(ui.Vertical).
			Children(func() {
				kit.IconButton(c.Key("back"), icons.ArrowLeft, i18n.T(c, "composer_model_settings"), railTile, false).Radius(p.RadiusLg).
					OnClick(func() { s.layer = layerMain })
				vendorRow := func(id, label string, mark *ui.SVG) {
					selected := query == "" && s.family == id
					b := ui.ButtonBase(c.Key("vendor-" + id)).Label(label).Height(railTile).Shrink(0).
						Radius(p.RadiusLg).AlignItems(ui.Center).
						Cursor(ui.CursorPointer).AccessSelected(selected).TextColor(p.MutedForeground)
					if compactRail {
						b.Size(railTile, railTile).Center()
						kit.Tooltip(c, b, label)
					} else {
						b.FillWidth().PaddingX(8).Gap(6)
					}
					switch {
					case selected || b.Pressed():
						b.Background(p.Selected()).TextColor(p.Foreground)
					case b.Hovered():
						b.Background(p.Hover)
					}
					b.Children(func() {
						ui.Icon(c, mark).FontSize(railIcon).Shrink(0)
						if !compactRail {
							ui.Text(c, label).FontSize(13).SingleLine().MinWidth(0).Shrink(1)
						}
					}).OnClick(func() {
						s.family, s.modelQuery = id, ""
						*e = effortState{}
					})
				}
				if query == "" && len(recent) > 0 {
					vendorRow(recentVendor, i18n.T(c, "composer_models_recent"), icons.Clock)
				}
				for _, f := range vendors {
					if visible[f.ID] {
						vendorRow(f.ID, names[f.ID], f.Icon)
					}
				}
			})
		rows := map[string]ui.Element{}
		var headers []providerHeader
		listKey := s.family
		if query != "" {
			listKey = "#search"
		}
		var first, firstHeader ui.Element
		list := ui.Scroll(c.Key("models-" + listKey)).Grow(1).MinWidth(0).MaxHeight(panelHeight).FocusGroup(ui.Vertical)
		list.Children(func() {
			if len(shown) == 0 {
				ui.Text(c, i18n.T(c, "composer_model_empty")).FontSize(14).Padding(8).TextColor(p.MutedForeground)
				return
			}
			if grouped {
				hasChosenGroup := slices.ContainsFunc(groups, func(g providerGroup) bool { return g.ContainsChosen })
				for gi, g := range groups {
					open := s.providerGroupOpen(s.family, g, gi, hasChosenGroup, query != "", e.enter)
					if gi > 0 {
						ui.Box(c.Key("gap-" + g.Key)).Height(4).Shrink(0)
					}
					label := g.Name
					if label == "" {
						label = g.Key
					}
					group := ui.ButtonBase(c.Key("provider-" + g.Key)).Role(ui.RoleDisclosure).Label(label).Expanded(open).
						FillWidth().MinWidth(0).Height(32).Shrink(0).PaddingX(rowPad).Gap(6).Radius(p.RadiusLg).
						AlignItems(ui.Center).Cursor(ui.CursorPointer).TextColor(p.MutedForeground)
					switch {
					case group.Pressed():
						group.Background(p.Selected())
					case group.Hovered():
						group.Background(p.Hover)
					}
					group.Children(func() {
						providerMark(c, g.Icon, label, 14, p.MutedForeground)
						ui.Text(c, label).FontSize(13).LineHeight(18.0 / 13).SingleLine().Grow(1).Shrink(1).MinWidth(0)
						if g.Tag != "" {
							kit.StatusPill(c, g.Tag, nil, kit.Neutral, true)
						}
						ui.Text(c, strconv.Itoa(len(g.Models))).FontSize(11).SingleLine().Shrink(0).TextColor(p.MutedForeground)
						chevron := icons.ChevronRight
						if open {
							chevron = icons.ChevronDown
						}
						ui.Icon(c, chevron).FontSize(12).Shrink(0)
					})
					if query == "" {
						family, key, wasOpen := s.family, g.Key, open
						group.OnClick(func() {
							s.setProviderExpanded(family, key, !wasOpen)
						})
					}
					headers = append(headers, providerHeader{el: group, key: g.Key, open: open})
					if firstHeader == (ui.Element{}) {
						firstHeader = group
					}
					if !open {
						continue
					}
					for _, m := range g.Models {
						vendor := ""
						if query != "" {
							vendor = names[vendorID(m)]
						}
						row := s.modelRow(c, m, chosen, vendor, "")
						rows[m.ID] = row
						if first == (ui.Element{}) {
							first = row
						}
					}
				}
			} else {
				flat := shown
				if !isRecent && len(groups) == 1 {
					flat = groups[0].Models
				}
				for i, m := range flat {
					vendor, source := "", ""
					if query != "" || isRecent {
						vendor = names[vendorID(m)]
					}
					if isRecent && multiProviderCatalog {
						source = m.Provider
					}
					row := s.modelRow(c, m, chosen, vendor, source)
					rows[m.ID] = row
					if i == 0 {
						first = row
					}
				}
			}
			if truncated {
				ui.Text(c.Key("search-more"), i18n.T(c, "composer_model_search_more")).FontSize(11).Padding(6, 8).TextColor(p.MutedForeground)
			}
		})
		// The picker opens with the focus on the chosen model's row, so the
		// keys move from it.
		if row, ok := rows[chosen.ID]; ok && e.enter {
			row.ScrollIntoView()
			row.Focus()
		}
		e.enter = false
		if field.Shortcut(0, ui.KeyDown) {
			if first != (ui.Element{}) {
				first.Focus()
			} else if firstHeader != (ui.Element{}) {
				firstHeader.Focus()
			}
		}
		if _, listed := rows[e.model]; !listed {
			e.model = chosen.ID
			if query != "" && firstModel.ID != "" {
				e.model = firstModel.ID
			}
		}
		headerFocused := false
		for _, h := range headers {
			if h.el.Focused() {
				headerFocused = true
				if query == "" {
					if !h.open && h.el.Shortcut(0, ui.KeyRight) {
						s.setProviderExpanded(s.family, h.key, true)
						c.Invalidate()
					} else if h.open && h.el.Shortcut(0, ui.KeyLeft) {
						s.setProviderExpanded(s.family, h.key, false)
						c.Invalidate()
					}
				}
				break
			}
		}
		// The focus takes the column as it comes to a row; the pointer only
		// by a click on one.
		focused := ""
		for id, row := range rows {
			if row.Focused() {
				if focused = id; e.focused != id {
					e.model = id
				}
			}
		}
		e.focused = focused
		column := byID[e.model]
		if query != "" && len(shown) == 0 {
			column = Model{}
		}
		if _, colListed := rows[column.ID]; !colListed && column.ID != "" {
			if len(rows) == 0 || slices.ContainsFunc(shown, func(m Model) bool { return m.ID == column.ID }) {
				column = Model{}
			}
		}
		levels := efforts(column)
		// Right and Tab go to the efforts, where there are any; Left comes
		// back to the model they are of.
		right, tab := false, false
		if len(levels) > 0 && focused != "" && !headerFocused {
			right, tab = list.Shortcut(0, ui.KeyRight), list.Shortcut(0, ui.KeyTab)
		}
		row, listed := rows[column.ID]
		// The column's model stays marked while the pointer or the focus is
		// in the column.
		if listed && column.ID != chosen.ID && !row.Pressed() {
			row.Background(p.Hover)
		}
		switch {
		case !e.back:
		case listed:
			row.Focus()
		case first != (ui.Element{}):
			first.Focus()
		case firstHeader != (ui.Element{}):
			firstHeader.Focus()
		default:
			field.Focus()
		}
		e.back = false
		if len(levels) > 0 {
			line("effort-line")
			s.effortColumn(c, column, chosen, right || tab || e.open)
		}
		e.open = false
	})
}

// rowPad is a picker row's padding at each end: its score ends there.
const rowPad = 8

// rowLook is a row of the picker, a model's or an effort's: 32 high, its
// content from rowPad. The chosen one is told by its fill alone, the
// neutral Palette.Selected, with no check; assistive technology has it as
// selected.
func rowLook(c *ui.Context, b ui.Element, selected bool) ui.Element {
	p := theme.Of(c)
	b.FillWidth().MinWidth(0).Height(32).Shrink(0).PaddingX(rowPad).Gap(8).Radius(p.RadiusLg).Justify(ui.Start).
		Cursor(ui.CursorPointer).AccessSelected(selected).TextColor(p.PopoverForeground)
	if b.Pressed() || selected {
		b.Background(p.Selected())
	}
	return b
}

// score is a row's last child: the score in the monospace font, muted, its
// digits ending at the row's padding. It is text alone: nothing opens
// over it (AGENTS.md, Removed UI).
func score(c *ui.Context, sc *Score) ui.Element {
	return ui.Text(c, fmt.Sprintf("%.1f", sc.Overall)).Font("monospace").FontSize(14).SingleLine().Shrink(0).
		TextColor(theme.Of(c).MutedForeground)
}

// modelGroupHintRow draws the active routing group's name, canonical tags, and
// online rate / degraded status below the model selector trigger.
func modelGroupHintRow(c *ui.Context, p *theme.Palette, key string, hint *ModelGroupHint) {
	if hint == nil {
		return
	}
	row := ui.Row(c.Key(key)).MaxWidth(panelWidth).Height(20).Gap(4).AlignItems(ui.Center).Center()
	if hint.Summary != "" {
		kit.Tooltip(c, row, hint.Summary)
	}
	row.Children(func() {
		if hint.GroupName != "" {
			ui.Text(c, hint.GroupName).FontSize(11).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
		}
		for _, tag := range hint.Tags {
			kit.StatusPill(c, tag.Label, tag.Icon, tag.Tone, true)
		}
		if hint.Rate != "" {
			ui.Text(c, hint.Rate).FontSize(10).SingleLine().Shrink(0).TextColor(p.MutedForeground)
		}
		if hint.StatusText != "" {
			statusEl := kit.StatusIndicator(c, hint.StatusText, hint.StatusIcon, hint.StatusTone, true)
			if hint.StatusError != "" {
				kit.Tooltip(c, statusEl, hint.StatusError)
			}
		}
	})
}

// modelRow chooses the model at the effort its column checks.
func (s *State) modelRow(c *ui.Context, m, chosen Model, vendor, source string) ui.Element {
	p := theme.Of(c)
	b := rowLook(c, ui.ButtonBase(c.Key(m.ID)).Label(m.Name), m.ID == chosen.ID)
	if m.Group != nil || m.Unavailable {
		b.Height(40)
	}
	if m.Unavailable {
		b.Border(1, p.Warning.Alpha(0.55))
	}
	var detail []string
	for _, label := range []string{vendor, source} {
		if label != "" && !slices.Contains(detail, label) {
			detail = append(detail, label)
		}
	}
	if m.ContextWindow >= 8192 {
		count := strconv.FormatUint(uint64(m.ContextWindow), 10)
		if m.ContextWindow >= 1000 {
			key, value := "provider_tokens_k", float64(m.ContextWindow)/1000
			if m.ContextWindow >= 1_000_000 {
				key, value = "provider_tokens_m", float64(m.ContextWindow)/1_000_000
			}
			count = i18n.T(c, key, "count", strconv.FormatFloat(value, 'f', -1, 64))
		}
		detail = append(detail, count)
	} else if m.ContextUnknown {
		detail = append(detail, i18n.T(c, "meter_limit_unknown"))
	}
	subtitle := strings.Join(detail, " · ")
	if m.Unavailable {
		detail = append(detail, i18n.T(c, "unavailable"))
	}
	if m.Group != nil && m.Group.Summary != "" {
		detail = append(detail, m.Group.Summary)
	}
	if m.Reasoning {
		detail = append(detail, i18n.T(c, "composer_effort"))
	}
	b.Description(strings.Join(detail, " · "))
	b.Children(func() {
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, m.Name).FontSize(14).SingleLine()
			if m.Group != nil {
				ui.Row(c).FillWidth().MinWidth(0).Gap(4).AlignItems(ui.Center).Children(func() {
					if subtitle != "" {
						ui.Text(c, subtitle).FontSize(10).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
						ui.Text(c, "·").FontSize(10).Shrink(0).TextColor(p.MutedForeground)
					}
					if m.Unavailable {
						kit.StatusPill(c, i18n.T(c, "unavailable"), icons.TriangleAlert, kit.Caution, true)
					}
					if m.Group.GroupName != "" {
						ui.Text(c, m.Group.GroupName).FontSize(10).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
					}
					for _, tag := range m.Group.Tags {
						kit.StatusPill(c, tag.Label, tag.Icon, tag.Tone, true)
					}
					if m.Group.StatusText != "" {
						kit.StatusIndicator(c, m.Group.StatusText, m.Group.StatusIcon, m.Group.StatusTone, true)
					}
				})
			} else if m.Unavailable {
				ui.Row(c).FillWidth().MinWidth(0).Gap(4).AlignItems(ui.Center).Children(func() {
					if subtitle != "" {
						ui.Text(c, subtitle).FontSize(10).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
					}
					kit.StatusPill(c, i18n.T(c, "unavailable"), icons.TriangleAlert, kit.Caution, true)
				})
			} else if subtitle != "" {
				ui.Text(c, subtitle).FontSize(10).SingleLine().TextColor(p.MutedForeground)
			}
		})
		if m.Reasoning {
			ui.Icon(c, icons.Brain).FontSize(12).Shrink(0).TextColor(p.MutedForeground)
		}
		if m.Score != nil {
			score(c, m.Score)
		}
	}).OnClick(func() {
		if len(efforts(m)) == 0 {
			s.pick(m, 0)
			return
		}
		s.efforts.model, s.efforts.open = m.ID, true
	})
	return b
}

// carried is the effort its column checks for a model: the chosen model's
// current effort, else another model's first.
func (s *State) carried(m, chosen Model) int {
	list := efforts(m)
	if len(list) == 0 || m.ID != chosen.ID {
		return 0
	}
	return min(max(int(*s.controlEffort()), 0), len(list)-1)
}

// effortColumn is the efforts of a model under their heading, as high and as
// muted as a group's name (kit.ModelColumnHeading), rows as the models' are:
// the level and, at the row's end, the score measured at it.
// The one the model's row chooses is filled and selected. A model without
// efforts has a muted line for them, so the column keeps its place. focus
// gives the selected row the keyboard focus.
func (s *State) effortColumn(c *ui.Context, m, chosen Model, focus bool) {
	p := theme.Of(c)
	name := i18n.T(c, "composer_effort")
	list := efforts(m)
	current := s.carried(m, chosen)
	// It scrolls, as the models do: a model declares as many levels as it
	// has.
	var column ui.Element
	ui.Column(c.Key("effort-column")).Width(effortWidth).Shrink(0).Children(func() {
		kit.ModelColumnHeading(c, i18n.T(c, "model_effort"))
		column = ui.Scroll(c.Key("efforts")).FillWidth().Grow(1).MinHeight(0).Role(ui.RoleListBox).Label(name).FocusGroup(ui.Vertical)
	})
	column.Children(func() {
		if len(list) == 0 {
			ui.Text(c.Key("none"), i18n.T(c, "composer_effort_unavailable")).FontSize(14).LineHeight(20.0/14).
				Padding(6, 8).TextColor(p.MutedForeground)
			return
		}
		for i, level := range list {
			selected := i == current
			b := rowLook(c, ui.ButtonBase(c.Key("effort-"+level)).Label(level).Role(ui.RoleOption), selected)
			if b.Hovered() && !selected && !b.Pressed() {
				b.Background(p.Hover)
			}
			if focus && selected {
				b.Focus()
			}
			b.Children(func() {
				ui.Text(c, level).FontSize(14).LineHeight(20.0 / 14).SingleLine().Grow(1).Shrink(1).MinWidth(0)
				measured := m.Score
				if at, ok := m.Scores[level]; ok {
					measured = at
				}
				if measured != nil && measured.Tested == level {
					score(c, measured)
				}
			}).OnClick(func() { s.pick(m, i) })
		}
	})
	if column.Shortcut(0, ui.KeyLeft) {
		s.efforts.back = true
	}
}

// pick chooses a model at an effort and goes back to the controls.
func (s *State) pick(m Model, effort int) {
	if member := s.memberSelection(); member != nil {
		features := maps.Clone(member.Features)
		if member.InheritFeatures {
			features = maps.Clone(s.Features)
		}
		for id, selected := range features {
			if !slices.ContainsFunc(m.Features, func(feature Feature) bool {
				return feature.ID == id && slices.ContainsFunc(feature.Options, func(option FeatureOption) bool { return option.ID == selected })
			}) {
				delete(features, id)
			}
		}
		member.Model, member.Effort, member.Features, member.Overridden = m.ID, float64(effort), features, true
	} else {
		if s.Model != m.ID {
			s.Features = map[string]string{}
		}
		s.Model, s.Effort = m.ID, float64(effort)
	}
	s.layer, s.efforts = layerMain, effortState{}
}
