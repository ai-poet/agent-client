package sidebar

import (
	"regexp"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// Terminal is a running terminal in one of a project's worktrees.
type Terminal struct {
	ID, Title, Path string
}

// terminals are a project's terminal rows after its sessions, in the order
// chosen.
func (s *State) terminals(c *ui.Context, pr *preview.Project) {
	if s.Actions == nil || s.Actions.Terminals == nil {
		return
	}
	raw := s.Actions.Terminals(pr)
	byID := make(map[string]Terminal, len(raw))
	entries := make([]Entry, 0, len(raw))
	for _, t := range raw {
		byID[t.ID] = t
		entries = append(entries, Entry{ID: t.ID, Name: terminalLabel(c, t)})
	}
	for _, e := range Sorted(entries, s.sort()) {
		s.terminal(c, byID[e.ID])
	}
}

// terminal is a terminal's row, a session row's look: its title without the
// shell's user@host prefix, and the terminal's mark where a session shows its
// age. It is selected while the workspace shows the terminal. Clicking it
// opens the terminal's kept tab; its right-click menu is the system's.
func (s *State) terminal(c *ui.Context, t Terminal) {
	p := theme.Of(c)
	title := terminalLabel(c, t)
	b := ui.ButtonBase(c.Key("terminal/" + t.ID)).Label(title).Height(rowHeight).PaddingStart(indent).PaddingEnd(8).Gap(8).Radius(p.Radius)
	hovered, selected := b.Hovered(), s.Terminal == t.ID
	b.Background(p.SidebarItem(selected, hovered))
	text := p.MutedForeground
	switch {
	case selected:
		b.AccessSelected(true)
		text = p.SidebarAccentForeground
	case hovered:
		text = p.SidebarForeground
	}
	b.ContextMenu(func(m *ui.Menu) { s.terminalMenu(c, m, t, title) }).OnClick(func() { s.openTerminal(t.ID) })
	b.TextColor(text).Children(func() {
		kit.Level(c, ui.Text(c, title).FontSize(14).SingleLine().Grow(1).Shrink(1), 14, title)
		ui.Icon(c, icons.Terminal).FontSize(12).TextColor(p.MutedForeground.Alpha(0.6)).PassThrough()
	})
}

// shellPrompt is the user@host: a shell's default window title leads with.
var shellPrompt = regexp.MustCompile(`^[^\s@:]+@[^\s:]+:\s*`)

// TerminalTitle is a terminal's title without its shell's user@host prefix.
func TerminalTitle(title string) string {
	return shellPrompt.ReplaceAllString(title, "")
}

func terminalLabel(c *ui.Context, t Terminal) string {
	if title := TerminalTitle(t.Title); title != "" {
		return title
	}
	return i18n.T(c, "terminal")
}

func (s *State) openTerminal(id string) {
	if s.Actions != nil && s.Actions.OpenTerminal != nil {
		s.Actions.OpenTerminal(id)
	}
}

// terminalMenu opens the terminal or closes it, which ends its shell on the
// Node once confirmed, as closing its tab does.
func (s *State) terminalMenu(c *ui.Context, m *ui.Menu, t Terminal, title string) {
	var openTerminal, closeTerminal func(string)
	if s.Actions != nil {
		openTerminal = s.Actions.OpenTerminal
		closeTerminal = s.Actions.CloseTerminal
	}
	if m.Item(i18n.T(c, "directory_open")).Disabled(openTerminal == nil).Chosen() {
		s.openTerminal(t.ID)
	}
	m.Separator()
	if !m.Item(i18n.T(c, "workspace_close_terminal")).Disabled(closeTerminal == nil).Chosen() || s.confirm.open {
		return
	}
	id := t.ID
	s.confirm = confirm{
		open: true, title: i18n.T(c, "workspace_close_terminal"), message: title, action: i18n.T(c, "close"),
		run: func() { closeTerminal(id) },
	}
}
