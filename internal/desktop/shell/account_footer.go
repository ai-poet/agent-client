package shell

import (
	"fmt"
	"strings"
	"unicode/utf8"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	cheapauthui "github.com/ai-poet/cheaprouter-harness/internal/desktop/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/settings"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// accountFooter renders the bottom-left CheapRouter account avatar entry and
// its two-level popover panel (account summary + per-lane group entries +
// level-2 single-select group submenu per lane).
func (a *App) accountFooter(c *ui.Context) {
	state := a.Settings.CheapRouter
	if state == nil {
		return
	}
	p := theme.Of(c)
	signedIn := state.SignedIn()
	if !a.accountOpen || !signedIn {
		a.accountGroupLane = ""
	}
	width, height := c.Size()
	panelWidth := max(0, min(304, width-32))
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
			a.accountGroupLane = ""
			if opening && state.SignedIn() && state.RefreshGroups != nil {
				state.RefreshGroups()
			}
		})
	})

	if !signedIn {
		panel := kit.Popover(c, trigger, &a.accountOpen, func() {
			a.accountSignedOutContent(c, state)
		})
		if panel != (ui.Element{}) {
			panel.AttachTo(trigger, ui.AnchorTopStart, ui.AnchorBottomStart).Margin(0, 0, 6, 0).
				Width(panelWidth).MinWidth(0).MaxHeight(max(0, min(460, height-32)))
		}
		return
	}

	panel := kit.Popover(c, trigger, &a.accountOpen, func() {
		a.accountSignedInContent(c, state)
	})
	if panel != (ui.Element{}) {
		kit.Snapped(panel).Padding(0).MinWidth(0).AttachTo(trigger, ui.AnchorTopStart, ui.AnchorBottomStart).
			Margin(0, 0, 6, 0).Width(panelWidth).MaxHeight(max(0, height-72))
		if panel.Dismissed() {
			a.accountGroupLane = ""
		}
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

func (a *App) openCheapAccountSettings() {
	a.accountOpen = false
	a.accountGroupLane = ""
	a.Navigate(SettingsPage)
	a.Settings.Open(settings.CheapAccount)
}

func (a *App) accountSignedOutContent(c *ui.Context, state *cheapauthui.State) {
	p := theme.Of(c)
	ui.Column(c.Key("signed-out-panel")).Padding(12).Gap(12).Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			accountAvatar(c, p, state, 34)
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
				ui.Text(c, "CheapRouter 账号").FontSize(14).FontWeight(600).TextColor(p.PopoverForeground)
				ui.Text(c, "登录后自动同步模型网关与可用分组").FontSize(12).TextColor(p.MutedForeground)
			})
		})
		kit.Primary(c.Key("popover-login"), "在浏览器中登录", false).FillWidth().OnClick(func() {
			a.openCheapAccountSettings()
			if state.Start != nil && state.Stage == cheapauthui.SignedOut {
				state.Start(false)
			}
		})
		kit.Ghost(c.Key("popover-settings"), "账号设置", false).FillWidth().Height(32).OnClick(a.openCheapAccountSettings)
	})
}

func (a *App) accountSignedInContent(c *ui.Context, state *cheapauthui.State) {
	p := theme.Of(c)
	lanes := state.Lanes()

	_, height := c.Size()
	ui.Scroll(c.Key("signed-in-panel")).FillWidth().MaxHeight(max(0, height-74)).Padding(10).Gap(10).Children(func() {
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

		ui.Column(c.Key("groups-entry-section")).Gap(4).Children(func() {
			ui.Row(c.Key("groups-section-header")).AlignItems(ui.Center).Justify(ui.SpaceBetween).PaddingX(4).Gap(6).Children(func() {
				ui.Row(c).AlignItems(ui.Center).Gap(6).Children(func() {
					ui.Text(c, "模型分组").FontSize(12).FontWeight(600).TextColor(p.MutedForeground)
					if state.GroupStatusFailed() {
						cheapauthui.StatusUnknownBadge(c, state.GroupStatusError, true)
					}
				})
				if state.RefreshGroups != nil {
					kit.IconButton(c.Key("refresh-groups"), icons.Reload, "刷新分组", 22, false).
						Disabled(state.GroupsLoading || state.SwitchingGroup || state.LoggingOut).
						OnClick(state.RefreshGroups)
				}
			})

			switch {
			case state.GroupsLoading && len(lanes) == 0:
				ui.Text(c, "正在加载可用分组…").FontSize(12).Padding(4, 8).TextColor(p.MutedForeground)
			case state.GroupsError != "" && len(lanes) == 0:
				ui.Text(c, state.GroupsError).FontSize(12).Padding(4, 8).TextColor(p.Danger)
				if state.RefreshGroups != nil {
					kit.Ghost(c.Key("retry-groups"), "重试加载分组", false).Height(28).
						Disabled(state.LoggingOut).OnClick(state.RefreshGroups)
				}
			case len(lanes) == 0:
				ui.Text(c, "暂无可用分组").FontSize(12).Padding(4, 8).TextColor(p.MutedForeground)
			default:
				if state.GroupsError != "" {
					ui.Text(c, state.GroupsError).FontSize(12).Padding(2, 8).TextColor(p.Danger)
				}
				for _, l := range lanes {
					lane := l
					laneLabel := core.LaneDisplayName(lane)
					boundGroup, hasBound := state.BoundGroup(lane)
					open := a.accountGroupLane == lane
					laneTrigger := ui.ButtonBase(c.Key("lane-menu-" + lane)).Label(laneLabel).FillWidth().Padding(6, 8).
						Radius(p.Radius).Cursor(ui.CursorPointer).
						HasPopup(ui.PopupMenu).Expanded(open)
					if hasBound {
						desc := state.DescribeGroup(boundGroup)
						laneTrigger.Description(desc.CompactSummary())
					} else {
						laneTrigger.Height(32).AlignItems(ui.Center).Gap(8)
					}
					switch {
					case laneTrigger.Pressed() || open:
						laneTrigger.Background(p.Active)
					case laneTrigger.Hovered():
						laneTrigger.Background(p.Hover)
					}
					laneTrigger.Children(func() {
						if !hasBound {
							ui.Icon(c, icons.Network).FontSize(13).Shrink(0).TextColor(p.MutedForeground)
							ui.Text(c, laneLabel).FontSize(13).FontWeight(500).SingleLine().Shrink(0).TextColor(p.PopoverForeground)
							ui.Spacer(c)
							ui.Text(c, "未绑定").FontSize(12).SingleLine().Shrink(1).MinWidth(0).TextColor(p.MutedForeground)
							ui.Icon(c, icons.ChevronRight).FontSize(14).Shrink(0).TextColor(p.MutedForeground)
							return
						}
						desc := state.DescribeGroup(boundGroup)
						ui.Column(c).FillWidth().Grow(1).Shrink(1).MinWidth(0).Gap(4).Children(func() {
							ui.Row(c).FillWidth().MinWidth(0).AlignItems(ui.Center).Gap(8).Children(func() {
								ui.Icon(c, icons.Network).FontSize(13).Shrink(0).TextColor(p.MutedForeground)
								ui.Text(c, laneLabel).FontSize(13).FontWeight(500).SingleLine().Shrink(0).TextColor(p.PopoverForeground)
								ui.Spacer(c)
								ui.Text(c, desc.DisplayName()).FontSize(12).SingleLine().Shrink(1).MinWidth(0).TextColor(p.PopoverForeground)
								ui.Icon(c, icons.ChevronRight).FontSize(14).Shrink(0).TextColor(p.MutedForeground)
							})
							ui.Row(c).FillWidth().MinWidth(0).AlignItems(ui.Center).Gap(5).PaddingStart(21).Children(func() {
								cheapauthui.GroupTagsRow(c, desc.Tags, true)
								if desc.Rate != "" {
									ui.Text(c, desc.Rate).FontSize(10).SingleLine().Shrink(0).TextColor(p.MutedForeground)
								}
								ui.Spacer(c)
								cheapauthui.GroupStatusBadge(c, desc.Status, true)
							})
						})
					}).OnClick(func() {
						if a.accountGroupLane == lane {
							a.accountGroupLane = ""
						} else {
							a.accountGroupLane = lane
						}
					})
					if laneTrigger.Shortcut(0, ui.KeyRight) {
						a.accountGroupLane = lane
					}
					a.accountLaneSubmenu(c, p, state, lane, laneTrigger)
				}
			}

			if state.SwitchingGroup {
				ui.Row(c.Key("switching")).Padding(4, 8).Gap(6).AlignItems(ui.Center).Children(func() {
					kit.Busy(c)
					text := "正在切换分组并刷新模型…"
					if state.SwitchingLane != "" {
						text = "正在更新 " + core.LaneDisplayName(state.SwitchingLane) + " 分组并刷新模型…"
					}
					ui.Text(c, text).FontSize(12).TextColor(p.MutedForeground)
				})
			}

			if state.GroupError != "" {
				ui.Column(c.Key("group-error")).Padding(8).Radius(p.Radius).Background(p.Danger.Alpha(0.12)).Border(1, p.Danger.Alpha(0.35)).Children(func() {
					ui.Text(c, state.GroupError).FontSize(12).TextColor(p.Danger)
				})
			}
		})

		ui.Box(c).FillWidth().Height(1).Background(p.Border)

		ui.Row(c.Key("account-actions")).Gap(6).Children(func() {
			kit.Ghost(c.Key("popover-account-page"), "账号详情", false).Grow(1).Height(30).OnClick(a.openCheapAccountSettings)
			kit.DangerBusy(c.Key("popover-logout"), "退出登录", state.LoggingOut).Grow(1).Height(30).OnClick(func() {
				a.accountOpen = false
				a.accountGroupLane = ""
				if state.Logout != nil {
					state.Logout()
				}
			})
		})
	})
}

func (a *App) accountLaneSubmenu(c *ui.Context, p *theme.Palette, state *cheapauthui.State, lane string, trigger ui.Element) {
	disabled := state.SwitchingGroup || state.LoggingOut || state.Stage == cheapauthui.SyncingGateway || (state.SelectLaneGroup == nil && state.SelectGroup == nil)
	width, height := c.Size()
	menuHeight := max(0, min(400, height-32))
	open := a.accountGroupLane == lane
	laneLabel := core.LaneDisplayName(lane)
	boundID := state.BoundGroupID(lane)
	sections := state.GroupSectionsForLane(lane)
	choose := func(groupID *int64) {
		a.accountGroupLane = ""
		trigger.Focus()
		if state.SelectLaneGroup != nil {
			state.SelectLaneGroup(lane, groupID)
		} else if state.SelectGroup != nil {
			state.SelectGroup(groupID)
		}
	}

	panel := kit.Popover(c, trigger, &open, func() {
		ui.Column(c.Key("group-submenu-panel-" + lane)).FillWidth().Shrink(1).Padding(8).Gap(6).Children(func() {
			ui.Row(c.Key("group-submenu-header-" + lane)).AlignItems(ui.Center).Justify(ui.SpaceBetween).PaddingX(4).Gap(6).Children(func() {
				ui.Row(c).AlignItems(ui.Center).Gap(6).Children(func() {
					ui.Text(c, laneLabel+" 分组").FontSize(12).FontWeight(600).TextColor(p.MutedForeground)
					if state.GroupStatusFailed() {
						cheapauthui.StatusUnknownBadge(c, state.GroupStatusError, true)
					}
				})
				if state.RefreshGroups != nil {
					refreshBtn := kit.IconButton(c.Key("refresh-groups-"+lane), icons.Reload, "刷新分组", 22, false).
						Disabled(state.GroupsLoading || state.SwitchingGroup || state.LoggingOut)
					refreshBtn.OnClick(state.RefreshGroups)
				}
			})

			ui.Box(c).FillWidth().Height(1).Background(p.Border)

			ui.Scroll(c.Key("groups-list-" + lane)).MaxHeight(max(0, menuHeight-53)).Gap(2).Children(func() {
				unboundGroupRow(c, p, "group-unbound-"+lane, "未绑定（留空）", boundID == nil, disabled, func() {
					choose(nil)
				})
				for _, sec := range sections {
					section := sec
					cheapauthui.GroupSectionHeader(c, fmt.Sprintf("group-section-%s-%d", lane, section.Tier), section.Tier)
					for _, g := range section.Groups {
						group := g
						selected := boundID != nil && *boundID == group.ID
						desc := state.DescribeGroup(group)
						groupOptionRow(c, p, fmt.Sprintf("group-%s-%d", lane, group.ID), desc, selected, disabled, func() {
							id := group.ID
							choose(&id)
						})
					}
				}
			})
		})
	})
	if !open && a.accountGroupLane == lane {
		a.accountGroupLane = ""
	}
	if panel == (ui.Element{}) {
		return
	}
	kit.Snapped(panel).Padding(0).MinWidth(0).Width(max(0, min(304, width-32))).MaxHeight(menuHeight)
	if width >= 660 {
		panel.AttachTo(trigger, ui.AnchorRight, ui.AnchorLeft).Margin(0, 0, 0, 16)
	} else {
		// Stack above the trigger when there is no room to cascade sideways.
		panel.AttachTo(trigger, ui.AnchorTopStart, ui.AnchorBottomStart).Margin(0, 0, 6, 0)
	}
	if panel.Shortcut(0, ui.KeyLeft) {
		a.accountGroupLane = ""
		trigger.Focus()
	}
}

func unboundGroupRow(c *ui.Context, p *theme.Palette, key, name string, selected, disabled bool, onClick func()) {
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
		ui.Text(c, name).FontSize(13).SingleLine().Grow(1).Shrink(1).MinWidth(0).TextColor(p.PopoverForeground)
		if selected {
			ui.Icon(c, icons.Check).FontSize(14).Shrink(0).TextColor(p.Primary)
		}
	}).OnClick(onClick)
}

func groupOptionRow(c *ui.Context, p *theme.Palette, key string, desc core.GroupPresentation, selected, disabled bool, onClick func()) {
	name := desc.DisplayName()
	row := ui.ButtonBase(c.Key(key)).Label(name).Description(desc.CompactSummary()).FillWidth().Padding(6, 8).
		Radius(p.Radius).Cursor(ui.CursorPointer).
		Checked(selected).Disabled(disabled)
	switch {
	case selected:
		row.Background(p.Active)
	case row.Hovered():
		row.Background(p.Hover)
	}
	row.Children(func() {
		ui.Column(c).FillWidth().Grow(1).Shrink(1).MinWidth(0).Gap(4).Children(func() {
			ui.Row(c).FillWidth().MinWidth(0).AlignItems(ui.Center).Gap(6).Children(func() {
				ui.Text(c, name).FontSize(13).SingleLine().Grow(1).Shrink(1).MinWidth(0).TextColor(p.PopoverForeground)
				cheapauthui.GroupStatusBadge(c, desc.Status, true)
				if selected {
					ui.Icon(c, icons.Check).FontSize(14).Shrink(0).TextColor(p.Primary)
				}
			})
			ui.Row(c).FillWidth().MinWidth(0).AlignItems(ui.Center).Gap(5).Children(func() {
				cheapauthui.GroupTagsRow(c, desc.Tags, true)
				if desc.Rate != "" {
					ui.Text(c, desc.Rate).FontSize(10).SingleLine().Shrink(0).TextColor(p.MutedForeground)
				}
			})
		})
	}).OnClick(onClick)
}
