package settings

import (
	"fmt"

	"github.com/ai-poet/agent-client/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/internal/desktop/kit"
	"github.com/ai-poet/agent-client/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// nativeUpdates is Settings' MyGo updater. Install downloads and verifies in
// one action; a successful installation is held until the user restarts.
func nativeUpdates(c *ui.Context, m *Model) {
	u := &m.Updater
	a := m.UpdateActions
	if u.Attention() {
		u.seen = u.Version
	}
	// Without a feed the switches have nothing to govern, so they stay off
	// until one is configured.
	locked := u.Locked() || u.State == UpdateReady || !u.Configured || !u.NativeEnabled
	kit.Group(c, "updates", i18n.T(c, "updates"), "", nil, func() {
		kit.Card(c, func() {
			for _, option := range []struct {
				key, title string
				value      *bool
			}{
				{"automatic", "updates_automatic", &u.Automatic},
				{"prereleases", "updates_prereleases", &u.Prereleases},
			} {
				if settingSwitch(c, option.key, i18n.T(c, option.title), "", option.value, locked) && a != nil && a.SaveOptions != nil {
					a.SaveOptions(u.Automatic, u.Prereleases)
				}
			}
			status := func(key any, title, description string, actions func()) {
				kit.Row(c, key, title, description, actions)
			}
			switch {
			case u.Failure != nil && (u.State == UpdateIdle || u.State == UpdateAvailable):
				status("failure", i18n.T(c, u.Failure.Key), u.Failure.Detail, func() {
					kit.Secondary(c.Key("check"), i18n.T(c, "updates_check")).Disabled(!u.Configured || !u.NativeEnabled).OnClick(func() {
						if a != nil && a.Check != nil {
							a.Check()
						}
					})
				})
			case !u.Configured:
				status("version", i18n.T(c, "updates_version"), m.Version, func() {
					kit.Secondary(c.Key("check"), i18n.T(c, "updates_check")).Disabled(true)
				})
				ui.Text(c, i18n.T(c, "updates_not_configured")).FontSize(13).Padding(0, kit.RowPad, kit.RowPad).TextColor(theme.Of(c).MutedForeground)
			case !u.NativeEnabled:
				status("version", i18n.T(c, "updates_version"), m.Version, func() {
					kit.Secondary(c.Key("check"), i18n.T(c, "updates_check")).Disabled(true)
				})
				ui.Text(c, i18n.T(c, "updates_build_unsupported")).FontSize(13).Padding(0, kit.RowPad, kit.RowPad).TextColor(theme.Of(c).MutedForeground)
			case u.State == UpdateChecking:
				status("checking", i18n.T(c, "updates_checking"), m.Version, func() {
					kit.Secondary(c.Key("cancel"), i18n.T(c, "settings_cancel")).OnClick(func() {
						if a != nil && a.Cancel != nil {
							a.Cancel()
						}
					})
				})
			case u.State == UpdateAvailable:
				status("available", i18n.T(c, "updates_available", "version", u.Version), m.Version, func() {
					iconButton(c, "install", nil, i18n.T(c, "updates_install"), true, false).OnClick(func() {
						if a != nil && a.Download != nil {
							a.Download()
						}
					})
				})
			case u.State == UpdateDownloading:
				title := i18n.T(c, "updates_downloading", "version", u.Version)
				detail := ""
				if u.Total > 0 {
					detail = fmt.Sprintf("%s / %s · %.0f%%", megabytes(u.Copied), megabytes(u.Total), u.Percent())
				}
				ui.Column(c.Key("downloading")).Padding(kit.RowPad, 0).Gap(10).Children(func() {
					kit.Row(c, "status", title, detail, func() {
						kit.Secondary(c.Key("cancel"), i18n.T(c, "settings_cancel")).OnClick(func() {
							if a != nil && a.Cancel != nil {
								a.Cancel()
							}
						})
					})
					ui.Progress(c.Key("bar"), u.Percent()/100).FillWidth().Height(6).Label(title)
				})
			case u.State == UpdateReady:
				status("ready", i18n.T(c, "updates_ready_now", "version", u.Version), i18n.T(c, "updates_restart_description"), func() {
					iconButton(c, "restart", nil, i18n.T(c, "updates_restart_now"), true, false).OnClick(func() {
						if a != nil && a.Restart != nil {
							a.Restart()
						}
					})
				})
			case u.State == UpdateRestarting:
				status("restarting", i18n.T(c, "updates_restarting"), u.Version, func() {
					kit.SecondaryBusy(c.Key("busy"), i18n.T(c, "updates_restarting"), true)
				})
			case u.State == UpdateIdle && u.Current:
				status("current", i18n.T(c, "updates_current"), m.Version, func() {
					kit.Secondary(c.Key("check"), i18n.T(c, "updates_check")).OnClick(func() {
						if a != nil && a.Check != nil {
							a.Check()
						}
					})
				})
			default:
				status("version", i18n.T(c, "updates_version"), m.Version, func() {
					kit.Secondary(c.Key("check"), i18n.T(c, "updates_check")).OnClick(func() {
						if a != nil && a.Check != nil {
							a.Check()
						}
					})
				})
			}
		})
	})
}
