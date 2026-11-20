package shell

import (
	"fmt"
	"strings"
	"unicode/utf8"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	cheapauthui "github.com/ai-poet/agent-client/desktop/internal/desktop/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/icons"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/settings"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// accountFooter renders the bottom-left CheapRouter account avatar entry and
// its popover panel (login/logout, account summary, and group switching).
func (a *App) accountFooter(c *ui.Context) {
	state := a.Settings.CheapRouter
	if state == nil {
		return
	}
	p := theme.Of(c)
	signedIn := state.SignedIn()
	primaryText := "登录 CheapRouter"
	secondaryText := ""
	if signedIn {
		primaryText = strings.TrimSpace(state.Account.Email)
		if primaryText == "" {
			primaryText = state.Account.DisplayName()
		}
		if primaryText == "" {
			primaryText = "已登录 CheapRouter"
		}
		secondaryText = state.Account.PlanDisplay()
		if groupName := state.ActiveGroupName(); groupName != "" {
			secondaryText += " · " + groupName
		}
	} else if state.Stage == cheapauthui.Starting || state.Stage == cheapauthui.WaitingForCode {
		secondaryText = "正在登录…"
	}

	var trigger ui.Element
	ui.Row(c.Key("sidebar-account")).Shrink(0).Padding(8, 10).BorderWidth(1, 0, 0, 0).BorderColor(p.SidebarBorder).Children(func() {
		trigger = ui.ButtonBase(c.Key("account-trigger")).Label(primaryText).FillWidth().Height(40).PaddingX(8).Gap(10).
			Radius(p.Radius).Cursor(ui.CursorPointer).AlignItems(ui.Center).
			HasPopup(ui.PopupMenu).Expanded(a.accountOpen)
		switch {
		case trigger.Pressed() || a.accountOpen:
			trigger.Background(p.Active)
		case trigger.Hovered():
			trigger.Background(p.Hover)
		}
		trigger.Children(func() {
			accountAvatar(c, p, state, 28)
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
				ui.Text(c, primaryText).FontSize(13).FontWeight(500).SingleLine().TextColor(p.SidebarForeground)
				if secondaryText != "" {
					ui.Text(c, secondaryText).FontSize(11).SingleLine().TextColor(p.MutedForeground)
				}
			})
			chevron := icons.ChevronUp
			if a.accountOpen {
				chevron = icons.ChevronDown
			}
			ui.Icon(c, chevron).FontSize(14).Shrink(0).TextColor(p.MutedForeground)
		}).OnClick(func() {
			opening := !a.accountOpen
			a.accountOpen = opening
			if opening && state.SignedIn() && state.RefreshGroups != nil {
				state.RefreshGroups()
			}
		})
	})

	panel := kit.Popover(c, trigger, &a.accountOpen, func() {
		a.accountPopoverContent(c, state)
	})
	if panel != (ui.Element{}) {
		panel.AttachTo(trigger, ui.AnchorTopStart, ui.AnchorBottomStart).Margin(0, 0, 6, 0).Width(288).MaxHeight(460)
	}
}

func accountAvatar(c *ui.Context, p *theme.Palette, state *cheapauthui.State, size float32) {
	signedIn := state.SignedIn()
	bg := p.Muted
	fg := p.MutedForeground
	if signedIn {
		bg = p.Primary
		fg = p.PrimaryForeground
	}
	circle := ui.Row(c.Key("avatar")).Size(size, size).Shrink(0).Radius(size / 2).Background(bg).Center()
	if signedIn {
		statusColor := p.Success
		if state.Stage == cheapauthui.GatewayFailed {
			statusColor = p.Warning
		}
		circle.DrawOver(func(pt *ui.Painter, r ui.Rect) {
			pt.Fill(ui.Rect{X: r.X + r.W - 8, Y: r.Y + r.H - 8, W: 8, H: 8}, p.Sidebar, 4)
			pt.Fill(ui.Rect{X: r.X + r.W - 7, Y: r.Y + r.H - 7, W: 6, H: 6}, statusColor, 3)
		})
	}
	circle.Children(func() {
		if signedIn {
			initial := accountInitial(state.Account)
			if initial != "" {
				ui.Text(c, initial).FontSize(size * 0.45).FontWeight(600).TextColor(fg)
				return
			}
		}
		ui.Icon(c, icons.CircleUser).FontSize(size * 0.6).TextColor(fg)
	})
}

func accountInitial(user core.User) string {
	source := strings.TrimSpace(user.DisplayName())
	if source == "" {
		source = strings.TrimSpace(user.Email)
	}
	if source == "" {
		return ""
	}
	r, _ := utf8.DecodeRuneInString(source)
	if r == utf8.RuneError {
		return ""
	}
	return strings.ToUpper(string(r))
}

func (a *App) accountPopoverContent(c *ui.Context, state *cheapauthui.State) {
	p := theme.Of(c)
	openAccountPage := func() {
		a.accountOpen = false
		a.Navigate(SettingsPage)
		a.Settings.Open(settings.CheapAccount)
	}

	if !state.SignedIn() {
		ui.Column(c.Key("signed-out-panel")).Padding(12).Gap(12).Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				accountAvatar(c, p, state, 34)
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
					ui.Text(c, "CheapRouter 账号").FontSize(14).FontWeight(600).TextColor(p.PopoverForeground)
					ui.Text(c, "登录后自动同步模型网关与可用分组").FontSize(12).TextColor(p.MutedForeground)
				})
			})
			kit.Primary(c.Key("popover-login"), "在浏览器中登录", false).FillWidth().OnClick(func() {
				openAccountPage()
				if state.Start != nil && state.Stage == cheapauthui.SignedOut {
					state.Start(false)
				}
			})
			kit.Ghost(c.Key("popover-settings"), "账号设置", false).FillWidth().Height(32).OnClick(openAccountPage)
		})
		return
	}

	ui.Scroll(c.Key("signed-in-panel")).Padding(10).Gap(10).Children(func() {
		// Account info summary.
		ui.Column(c.Key("account-summary")).Padding(4).Gap(6).Children(func() {
			ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
				accountAvatar(c, p, state, 34)
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
					title := state.Account.DisplayName()
					if title == "" {
						title = "CheapRouter 账号"
					}
					ui.Text(c, title).FontSize(14).FontWeight(600).SingleLine().TextColor(p.PopoverForeground)
					if email := strings.TrimSpace(state.Account.Email); email != "" && email != title {
						ui.Text(c, email).FontSize(12).SingleLine().TextColor(p.MutedForeground)
					}
				})
			})
			ui.Row(c).Gap(8).AlignItems(ui.Center).Children(func() {
				ui.Text(c, state.Account.PlanDisplay()).FontSize(12).FontWeight(500).Padding(2, 6).Radius(4).
					Background(p.Muted).TextColor(p.PopoverForeground)
				if state.AccountFailure == "" {
					ui.Textf(c, "余额 $%.2f", state.Account.Balance).FontSize(12).TextColor(p.MutedForeground)
				}
				ui.Spacer(c)
				ui.Textf(c, "%d 个模型", state.ModelCount).FontSize(12).TextColor(p.MutedForeground)
			})
		})

		ui.Box(c).FillWidth().Height(1).Background(p.Border)

		// Group switching section.
		ui.Column(c.Key("groups-section")).Gap(6).Children(func() {
			ui.Row(c).AlignItems(ui.Center).Justify(ui.SpaceBetween).PaddingX(4).Children(func() {
				ui.Text(c, "模型分组").FontSize(12).FontWeight(600).TextColor(p.MutedForeground)
				if state.RefreshGroups != nil {
					refreshBtn := kit.IconButton(c.Key("refresh-groups"), icons.Reload, "刷新分组", 22, false).
						Disabled(state.GroupsLoading || state.SwitchingGroup)
					refreshBtn.OnClick(state.RefreshGroups)
				}
			})

			if state.GroupError != "" {
				ui.Column(c.Key("group-error")).Padding(8).Radius(p.Radius).Background(p.Danger.Alpha(0.12)).Border(1, p.Danger.Alpha(0.35)).Children(func() {
					ui.Text(c, state.GroupError).FontSize(12).TextColor(p.Danger)
				})
			}

			if state.SwitchingGroup {
				ui.Row(c.Key("switching")).Padding(4, 8).Gap(6).AlignItems(ui.Center).Children(func() {
					kit.Busy(c)
					ui.Text(c, "正在切换分组并刷新模型…").FontSize(12).TextColor(p.MutedForeground)
				})
			}

			switch {
			case state.GroupsLoading && len(state.Groups) == 0:
				ui.Text(c, "正在加载可用分组…").FontSize(12).Padding(4, 8).TextColor(p.MutedForeground)
			case state.GroupsError != "" && len(state.Groups) == 0:
				ui.Column(c.Key("groups-load-error")).Padding(6, 8).Gap(6).Children(func() {
					ui.Text(c, state.GroupsError).FontSize(12).TextColor(p.Danger)
					if state.RefreshGroups != nil {
						kit.Ghost(c.Key("retry-groups"), "重试加载分组", false).Height(28).OnClick(state.RefreshGroups)
					}
				})
			default:
				groupRow(c, p, "group-default", "默认分组（自动）", "", state.SelectedGroupID == nil, state.SwitchingGroup, func() {
					if state.SelectGroup != nil {
						state.SelectGroup(nil)
					}
				})
				for _, g := range state.Groups {
					group := g
					selected := state.SelectedGroupID != nil && *state.SelectedGroupID == group.ID
					meta := formatGroupMeta(group)
					groupRow(c, p, fmt.Sprintf("group-%d", group.ID), group.Name, meta, selected, state.SwitchingGroup, func() {
						if state.SelectGroup != nil {
							id := group.ID
							state.SelectGroup(&id)
						}
					})
				}
			}
		})

		ui.Box(c).FillWidth().Height(1).Background(p.Border)

		// Footer actions: Account details & Sign out.
		ui.Row(c.Key("account-actions")).Gap(6).Children(func() {
			kit.Ghost(c.Key("popover-account-page"), "账号详情", false).Grow(1).Height(30).OnClick(openAccountPage)
			kit.DangerBusy(c.Key("popover-logout"), "退出登录", state.LoggingOut).Grow(1).Height(30).OnClick(func() {
				a.accountOpen = false
				if state.Logout != nil {
					state.Logout()
				}
			})
		})
	})
}

func groupRow(c *ui.Context, p *theme.Palette, key, name, meta string, selected, disabled bool, onClick func()) {
	row := ui.ButtonBase(c.Key(key)).Label(name).FillWidth().Height(32).PaddingX(8).Gap(8).
		Radius(p.Radius).Cursor(ui.CursorPointer).AlignItems(ui.Center).
		Checked(selected).Disabled(disabled)
	switch {
	case selected:
		row.Background(p.Active)
	case row.Hovered():
		row.Background(p.Hover)
	}
	row.Children(func() {
		ui.Text(c, name).FontSize(13).SingleLine().Grow(1).Shrink(1).TextColor(p.PopoverForeground)
		if meta != "" {
			ui.Text(c, meta).FontSize(11).SingleLine().TextColor(p.MutedForeground)
		}
		if selected {
			ui.Icon(c, icons.Check).FontSize(14).Shrink(0).TextColor(p.Primary)
		}
	}).OnClick(onClick)
}

func formatGroupMeta(g core.Group) string {
	var parts []string
	if strings.EqualFold(g.SubscriptionType, "subscription") {
		parts = append(parts, "订阅")
	} else if g.RateMultiplier > 0 {
		parts = append(parts, fmt.Sprintf("%gx", g.RateMultiplier))
	}
	if platform := strings.TrimSpace(g.Platform); platform != "" {
		parts = append(parts, platform)
	}
	return strings.Join(parts, " · ")
}
