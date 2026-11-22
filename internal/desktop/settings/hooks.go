package settings

import (
	"slices"
	"strconv"
	"strings"

	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/preview"
	"github.com/ai-poet/cheaprouter-harness/internal/protocol"
	"github.com/egoist/mygo/ui"
)

var hookMessages = Messages{"hooks_saved", "hooks_failed", "hooks_conflict", "hooks_unknown"}

// phases name the hook phases.
var phases = []string{"hooks_before_tool", "hooks_after_tool", "hooks_turn_end"}

// hookEditor is the hook dialog's draft. A blank script ID means a new hook.
type hookEditor struct {
	open     bool
	script   preview.Hook
	original preview.Hook
	timeout  string
	// removing is the ID of the script whose removal awaits confirmation.
	removing string
}

// consent is the enabling dialog: the settings that run once the user agrees.
type consent struct {
	open     bool
	settings HookSettings
	revision uint64
}

func (m *Model) HooksEditing() bool { return m.hook.open || m.consent.open }

// Check validates a hook's draft: the i18n key of the problem, or "".
func (e *hookEditor) Check() string {
	ms, err := strconv.Atoi(strings.TrimSpace(e.timeout))
	if strings.TrimSpace(e.script.Command) == "" || err != nil || ms <= 0 || ms > 60000 {
		return "hooks_invalid"
	}
	e.script.TimeoutMS = ms
	return ""
}

// hooks is hooks on or off, and the scripts, each with its switch, its editor
// and its removal. Turning scripts on asks for consent.
func hooks(c *ui.Context, m *Model) {
	if m.Live && m.HooksContext != nil {
		if _, ok := m.HooksContext(); !ok {
			kit.Result(c, kit.ResultProps{Title: i18n.T(c, "hooks_project_required"), Card: true})
			return
		}
	}
	h := &m.Hooks
	writable := func() bool { return !m.Live || m.SaveHooks != nil && !h.Pending }
	save := func(s HookSettings) {
		if !writable() {
			return
		}
		h.Draft = s
		if m.Live {
			m.SaveHooks()
		} else {
			report(c, h.Commit(), hookMessages)
		}
	}
	enable := func(s HookSettings) {
		if s.Enabled && slices.ContainsFunc(s.Scripts, func(x preview.Hook) bool { return x.Enabled }) {
			m.consent = consent{open: true, settings: s, revision: h.Revision}
			return
		}
		save(s)
	}
	kit.Group(c, "settings", i18n.T(c, "hooks_settings"), "", nil, func() {
		kit.Card(c, func() {
			on := h.Draft.Enabled
			kit.Row(c, "enabled", i18n.T(c, "hooks_enabled"), "", func() {
				if ui.Switch(c.Key("switch"), &on).Label(i18n.T(c, "hooks_enabled")).Disabled(!writable() || len(h.Draft.Scripts) == 0).Changed() {
					s := clone(h.Draft)
					s.Enabled = on
					enable(s)
				}
			})
		})
	})
	kit.Group(c, "scripts", i18n.T(c, "hooks_scripts"), "", func() {
		if m.Live && h.Pending {
			kit.BusyAction(c.Key("add"), i18n.T(c, "hooks_add")) // the hooks are being read or saved
			return
		}
		kit.Action(c.Key("add"), icons.Plus, i18n.T(c, "hooks_add"), !writable()).OnClick(func() {
			m.hook = hookEditor{open: true, script: preview.Hook{Phase: preview.BeforeTool}, timeout: "30000"}
		})
	}, func() {
		if len(h.Draft.Scripts) == 0 {
			kit.Result(c, kit.ResultProps{Icon: icons.Settings2, Title: i18n.T(c, "hooks_empty"), Card: true})
			return
		}
		kit.Card(c, func() {
			for i, x := range h.Draft.Scripts {
				title, detail := x.Name, i18n.T(c, phases[x.Phase])
				if title == "" {
					title, detail = detail, x.Command
				}
				kit.Entry(c, x.ID, func() { kit.Record(c, title, detail) }, func() {
					on := x.Enabled
					if ui.Switch(c.Key("switch"), &on).Label(i18n.T(c, "hooks_enabled") + " " + title).Disabled(!writable()).Changed() {
						s := clone(h.Draft)
						s.Scripts[i].Enabled = on
						if on {
							enable(s)
						} else {
							save(s)
						}
					}
				}, func() {
					kit.Action(c.Key("edit"), icons.Settings2, i18n.T(c, "hooks_configure"), !writable()).OnClick(func() {
						m.hook = hookEditor{open: true, script: x, original: x, timeout: strconv.Itoa(x.TimeoutMS)}
					})
					kit.Action(c.Key("remove"), icons.CircleX, i18n.T(c, "hooks_remove"), !writable()).OnClick(func() {
						m.hook.removing = x.ID
					})
				})
			}
		})
	})
	if i := slices.IndexFunc(h.Draft.Scripts, func(x preview.Hook) bool { return x.ID == m.hook.removing }); writable() && m.hook.removing != "" && i >= 0 {
		x := h.Draft.Scripts[i]
		name := x.Name
		if name == "" {
			name = x.Command
		}
		open := true
		if confirm(c, "remove-hook", &open, i18n.T(c, "hooks_remove"), name, i18n.T(c, "hooks_remove")) {
			s := clone(h.Draft)
			s.Scripts = slices.Delete(s.Scripts, i, i+1)
			if len(s.Scripts) == 0 {
				s.Enabled = false
			}
			save(s)
			open = false
		}
		if !open {
			m.hook.removing = ""
		}
	} else {
		m.hook.removing = ""
	}
	hookDialog(c, m, save, writable())
	if writable() {
		consentDialog(c, m, save)
	}
}

func clone(s HookSettings) HookSettings {
	s.Scripts = slices.Clone(s.Scripts)
	return s
}

func hookScope(m *Model) string {
	if m.Live && m.HooksSandboxed != nil && !m.HooksSandboxed() {
		return "hooks_scope_windows"
	}
	return "hooks_scope_workspace"
}

// hookDialog is the hook's name, phase, command and timeout. The reference
// saves as it is edited and Close ends the edit; here Close saves the draft
// once it is valid, a problem toasting and keeping the dialog open.
func hookDialog(c *ui.Context, m *Model, save func(HookSettings), writable bool) {
	e := &m.hook
	if !e.open {
		return
	}
	title := i18n.T(c, "hooks_configure")
	kit.Dialog(c.Key("hook"), &e.open, func() {
		ui.Text(c, title).FontSize(16).FontWeight(600)
		ui.Scroll(c.Key("body")).Grow(1).Shrink(1).Gap(16).Children(func() {
			label := i18n.T(c, "hooks_name")
			kit.Field(c, "name", label, "", func() {
				ui.TextInput(c.Key("input"), &e.script.Name).Label(label).Placeholder(i18n.T(c, "form_name_hint")).FillWidth()
			})
			label = i18n.T(c, "hooks_phase")
			kit.Field(c, "phase", label, "", func() {
				phase := int(e.script.Phase)
				if pick(c, "select", label, &phase, labels(c, phases), e.script.ID == "before_tool" || e.script.ID == "after_tool" || e.script.ID == "turn_end").Fill().Changed() {
					e.script.Phase = preview.HookPhase(phase)
				}
			})
			label = i18n.T(c, "hooks_command")
			kit.Field(c, "command", label, "", func() {
				ui.TextInput(c.Key("input"), &e.script.Command).Label(label).FillWidth().Font("monospace")
			})
			label = i18n.T(c, "hooks_timeout")
			kit.Field(c, "timeout", label, "", func() {
				ui.TextInput(c.Key("input"), &e.timeout).Label(label).FillWidth()
			})
		})
		ui.Row(c.Key("footer")).Justify(ui.End).Children(func() {
			kit.Secondary(c.Key("close"), i18n.T(c, "close")).Disabled(!writable).OnClick(func() {
				if problem := e.Check(); problem != "" {
					kit.Toast(c, kit.Failed, i18n.T(c, problem))
					return
				}
				s := clone(m.Hooks.Draft)
				if e.script.ID == "" {
					e.script.ID = protocol.NewRequestID().String()
					s.Scripts = append(s.Scripts, e.script)
				} else {
					index := slices.IndexFunc(s.Scripts, func(h preview.Hook) bool { return h.ID == e.script.ID })
					if index < 0 || s.Scripts[index] != e.original {
						kit.Toast(c, kit.Conflict, i18n.T(c, "hooks_conflict"))
						return
					}
					s.Scripts[index] = e.script
				}
				e.open = false
				save(s)
			})
		})
	}).Width(min(600, width(c)-48)).Label(title)
}

// consentDialog asks before scripts run: what they may do, where, and each
// enabled script's phase, command and timeout.
func consentDialog(c *ui.Context, m *Model, save func(HookSettings)) {
	k := &m.consent
	if !k.open {
		return
	}
	if m.Live && k.revision != m.Hooks.Revision {
		k.open = false
		kit.Toast(c, kit.Conflict, i18n.T(c, "hooks_conflict"))
		return
	}
	title := i18n.T(c, "hooks_enable")
	location := i18n.T(c, "composer_host_local") + " · ~/Projects/cheaprouter · main"
	if m.Live && m.HooksContext != nil {
		location, _ = m.HooksContext()
	}
	message := []string{i18n.T(c, "hooks_consent"), i18n.T(c, hookScope(m)), location}
	for _, x := range k.settings.Scripts {
		if x.Enabled {
			message = append(message, i18n.T(c, phases[x.Phase])+"\n"+x.Command+"\n"+i18n.T(c, "hooks_timeout")+": "+strconv.Itoa(x.TimeoutMS))
		}
	}
	if kit.AlertDialog(c.Key("consent"), &k.open, title, strings.Join(message, "\n\n"), i18n.T(c, "settings_cancel"), title) == 1 {
		save(k.settings)
	}
}
