package cheapauth

import (
	"fmt"
	"strings"

	core "github.com/ai-poet/cheaprouter-harness/internal/cheapauth"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/i18n"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/icons"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/kit"
	"github.com/ai-poet/cheaprouter-harness/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
	"github.com/lobehub/lobe-mygo/pkg/action"
	"github.com/lobehub/lobe-mygo/pkg/feedback"
	"github.com/lobehub/lobe-mygo/pkg/field"
	"github.com/lobehub/lobe-mygo/pkg/lucide"
	lobetheme "github.com/lobehub/lobe-mygo/pkg/theme"
)

// Page renders Settings → CheapRouter 账号 together with its standalone
// recharge overlay for callers outside the main shell.
func Page(c *ui.Context, state *State) {
	PageContent(c, state)
	RechargeOverlay(c, state)
}

// PageContent renders Settings → CheapRouter 账号 without mounting the
// recharge overlay, for hosts that mount RechargeOverlay at the shell layer.
func PageContent(c *ui.Context, state *State) {
	if state == nil {
		kit.Result(c, kit.ResultProps{Title: "CheapRouter 账号不可用", SubTitle: "请重启应用后重试", Card: true})
		return
	}
	kit.Page(c, "cheaprouter-account", i18n.T(c, "cheaprouter_account"), i18n.T(c, "cheaprouter_account_description"), func() {
		kit.Card(c, func() {
			if state.Stage == Ready {
				ui.Column(c).Padding(20).Gap(18).Children(func() { content(c, state) })
				return
			}
			ui.Themed(c, lobetheme.Default().Theme(c.Theme().Dark)).Padding(20).Gap(18).
				Children(func() { content(c, state) })
		})
	})
}

func content(c *ui.Context, state *State) {
	p := lobetheme.TokensOf(c)
	if state.Failure != "" {
		feedback.Alert(c.Key("failure"), state.Failure, feedback.AlertProps{Type: feedback.Error, Selectable: true})
	}
	if state.SignedIn() && state.AccountFailure != "" && state.AccountFailure != state.Failure {
		feedback.Alert(c.Key("account-failure"), state.AccountFailure, feedback.AlertProps{Type: feedback.Error, Selectable: true})
	}
	if state.AffectedRoles > 0 {
		kit.Row(c, "affected-roles", i18n.T(c, "cheaprouter_roles_affected", "count", state.AffectedRoles), "", func() {
			kit.Secondary(c.Key("migrate-roles"), i18n.T(c, "cheaprouter_roles_migrate")).
				Disabled(state.MigratingRoles || state.LoggingOut || state.SwitchingGroup || state.Stage == SyncingGateway || state.MigrateRoles == nil).
				OnClick(state.MigrateRoles)
		})
	}
	if state.MigrationFailure != "" {
		message := i18n.T(c, state.MigrationFailure) + "：" + i18n.T(c, "migration_failed_retry")
		if state.MigrationFailureDetail != "" {
			message += "\n" + state.MigrationFailureDetail
		}
		ui.Column(c.Key("migration-failure")).Gap(8).Children(func() {
			feedback.Alert(c.Key("migration-failure-alert"), message, feedback.AlertProps{Type: feedback.Error, Selectable: true})
			if state.RetryMigration != nil {
				action.Button(c.Key("retry-migration"), i18n.T(c, "migration_retry"), action.ButtonProps{}).OnClick(state.RetryMigration)
			}
		})
	}
	if state.SignedIn() && (state.Stage != Ready || state.AccountFailure != "") {
		ui.Row(c.Key("signed-in-payments")).Gap(8).Children(func() {
			kit.Secondary(c.Key("recharge"), i18n.T(c, "recharge_open")).Disabled(state.LoggingOut || state.OpenRecharge == nil || state.Recharge != nil).OnClick(state.OpenRecharge)
			kit.Secondary(c.Key("plans"), i18n.T(c, "pay_plan_title")).Disabled(state.LoggingOut || state.OpenPlans == nil || state.Recharge != nil).OnClick(state.OpenPlans)
		})
	}

	switch state.Stage {
	case SignedOut, Starting:
		disabled := state.Stage == Starting || state.LoggingOut
		ui.Column(c.Key("signed-out")).Gap(16).Children(func() {
			ui.Column(c).Gap(6).Children(func() {
				ui.Text(c, i18n.T(c, "cheaprouter_signed_out")).FontSize(16).FontWeight(600)
				ui.Text(c, i18n.T(c, "cheaprouter_signed_out_description")).FontSize(13).TextColor(p.TextSecondary)
			})
			ui.Column(c).Gap(6).Children(func() {
				ui.Text(c, "选择服务入口").FontSize(14).FontWeight(600)
				ui.Text(c, "主站与 CDN 连接同一账号和模型网关；网络失败时可切换后重试。").FontSize(13).TextColor(p.TextSecondary)
			})
			for _, endpoint := range []struct{ name, value string }{{"主站", core.PrimaryEndpoint}, {"CDN", core.CDNEndpoint}} {
				selected := state.Endpoint == endpoint.value
				variant := action.Default
				if selected {
					variant = action.Filled
				}
				action.Button(c.Key(endpoint.name), endpoint.name+" · "+endpoint.value, action.ButtonProps{Variant: variant, Size: lobetheme.Large}).
					Role(ui.RoleToggleButton).Checked(selected).FillWidth().Justify(ui.Start).
					Disabled(disabled).OnClick(func() { state.Endpoint = endpoint.value }).Children(func() {
					if selected {
						ui.Icon(c, lucide.Check()).FontSize(14)
					}
				})
			}
			if state.Stage == Starting {
				ui.Row(c.Key("starting")).Gap(8).AlignItems(ui.Center).Children(func() {
					feedback.Spin(c, feedback.SpinProps{Size: lobetheme.Small}, nil)
					ui.Text(c, "正在准备安全登录…").FontSize(14).TextColor(p.TextSecondary)
				})
			}
			action.Button(c.Key("start"), "在浏览器中登录", action.ButtonProps{Variant: action.Primary, Size: lobetheme.Large}).
				FillWidth().Disabled(disabled).Busy(state.Stage == Starting).OnClick(func() {
				if state.Start != nil {
					state.Start(false)
				}
			})
			action.Button(c.Key("manual"), "使用粘贴登录码方式", action.ButtonProps{Size: lobetheme.Large}).
				FillWidth().Disabled(disabled).OnClick(func() {
				if state.Start != nil {
					state.Start(true)
				}
			})
		})
	case WaitingForCode:
		ui.Column(c.Key("waiting")).Gap(14).Children(func() {
			ui.Text(c, "浏览器已打开登录页").FontSize(14).FontWeight(600)
			detail := "完成登录后会自动返回应用。若没有返回，请复制页面显示的一次性登录码并粘贴到下方。"
			if !state.Loopback {
				detail = "当前环境无法使用本地自动回跳。完成浏览器登录后，请复制一次性登录码并粘贴到下方。"
			}
			ui.Text(c, detail).FontSize(13).TextColor(p.TextSecondary)
			ui.Row(c.Key("browser-actions")).Gap(8).Children(func() {
				action.Button(c.Key("open"), "重新打开登录页", action.ButtonProps{Variant: action.Primary}).
					Grow(1).Disabled(state.LoggingOut).OnClick(func() { c.OpenURL(state.LoginURL) })
				action.Button(c.Key("copy"), "复制链接", action.ButtonProps{}).Disabled(state.LoggingOut).OnClick(func() {
					c.WriteClipboard(state.LoginURL)
					kit.Copied(c)
				})
			})
			ui.Text(c, "一次性登录码").FontSize(14).FontWeight(500)
			disabled := state.Exchanging || state.LoggingOut
			input := field.Input(c.Key("code"), &state.Pasted, field.InputProps{Placeholder: "例如 K7QM-3XPD", Size: lobetheme.Large}).
				Label("一次性登录码").Disabled(disabled)
			hasCode := strings.TrimSpace(state.Pasted) != ""
			if input.Submitted() && hasCode && !disabled && state.SubmitPaste != nil {
				state.SubmitPaste()
			}
			if state.Exchanging {
				ui.Row(c.Key("exchanging")).Gap(8).AlignItems(ui.Center).Children(func() {
					feedback.Spin(c, feedback.SpinProps{Size: lobetheme.Small}, nil)
					ui.Text(c, "正在兑换一次性登录码…").FontSize(13).TextColor(p.TextSecondary)
				})
			}
			action.Button(c.Key("submit"), "完成登录", action.ButtonProps{Variant: action.Primary, Size: lobetheme.Large}).
				FillWidth().Disabled(!hasCode || state.Exchanging || state.LoggingOut).Busy(state.Exchanging).OnClick(func() {
				if state.SubmitPaste != nil {
					state.SubmitPaste()
				}
			})
			action.Button(c.Key("cancel"), "取消本次登录", action.ButtonProps{Size: lobetheme.Large}).
				FillWidth().Disabled(state.LoggingOut).OnClick(func() {
				if state.Cancel != nil {
					state.Cancel()
				}
			})
		})
	case SyncingGateway:
		ui.Row(c.Key("syncing")).Gap(10).AlignItems(ui.Center).Children(func() {
			feedback.Spin(c, feedback.SpinProps{Size: lobetheme.Small}, nil)
			ui.Column(c).Gap(4).Children(func() {
				ui.Text(c, "正在连接模型网关…").FontSize(14).FontWeight(600)
				ui.Text(c, "正在读取账号、套餐、模型目录和网关密钥，并配置 CheapRouter。").FontSize(13).TextColor(p.TextSecondary)
			})
		})
		action.Button(c.Key("stop-sync"), "取消本次配置", action.ButtonProps{}).
			FillWidth().Disabled(state.LoggingOut).OnClick(func() {
			if state.StopSync != nil {
				state.StopSync()
			}
		})
	case GatewayFailed:
		p := theme.Of(c)
		ui.Row(c.Key("sync-actions")).Gap(8).Children(func() {
			action.Button(c.Key("retry"), "重试网关配置", action.ButtonProps{Variant: action.Primary}).
				Grow(1).Disabled(state.LoggingOut).OnClick(func() {
				if state.Retry != nil {
					state.Retry()
				}
			})
			action.Button(c.Key("relogin"), "返回重新登录", action.ButtonProps{}).
				Disabled(state.LoggingOut).OnClick(func() {
				if state.Logout != nil {
					state.Logout()
				}
			})
		})
		if state.SelectOrigin != nil || state.TestOrigins != nil || state.AddOrigin != nil {
			gatewayOriginsSection(c, p, state)
		}
	case Ready:
		p := theme.Of(c)
		ui.Column(c.Key("account")).Gap(14).Children(func() {
			ui.Column(c).Gap(5).Children(func() {
				ui.Text(c, "已连接 CheapRouter").FontSize(16).FontWeight(600)
				name := state.Account.DisplayName()
				if name != "" {
					ui.Text(c, name).FontSize(14)
				}
				if state.Account.Email != "" && state.Account.Email != name {
					ui.Text(c, state.Account.Email).FontSize(13).TextColor(p.MutedForeground)
				}
			})
			if state.AccountFailure == "" {
				kit.Row(c, "plan", "当前套餐", "", func() {
					ui.Row(c).AlignItems(ui.Center).Gap(8).Children(func() {
						ui.Text(c, state.Account.PlanDisplay()).FontSize(14)
						kit.Secondary(c.Key("browse-plans"), i18n.T(c, "pay_plan_title")).Disabled(state.LoggingOut || state.OpenPlans == nil || state.Recharge != nil).OnClick(state.OpenPlans)
					})
				})
				kit.Row(c, "balance", "账户余额", "", func() {
					ui.Row(c).AlignItems(ui.Center).Gap(8).Children(func() {
						ui.Textf(c, "$%.2f", state.Account.Balance).FontSize(14)
						kit.Secondary(c.Key("recharge"), i18n.T(c, "recharge_open")).
							Disabled(state.LoggingOut || state.OpenRecharge == nil || state.Recharge != nil).OnClick(state.OpenRecharge)
					})
				})
			}
			kit.Row(c, "models", "可用聊天模型", "", func() { ui.Text(c, fmt.Sprintf("%d 个", state.ModelCount)).FontSize(14) })
			kit.Row(c, "endpoint", "服务入口", "", func() { ui.Text(c, state.Endpoint).FontSize(13).Selectable() })
			gatewayOriginsSection(c, p, state)
			if lanes := state.Lanes(); len(lanes) > 0 || state.GroupsLoading || state.GroupsError != "" {
				accountGroupsSection(c, p, state, lanes)
			} else if groupName := state.ActiveGroupName(); groupName != "" {
				kit.Row(c, "group", "当前分组", "", func() { ui.Text(c, groupName).FontSize(14) })
			}
			kit.DangerBusy(c.Key("logout"), "退出登录", state.LoggingOut).FillWidth().OnClick(func() {
				if state.Logout != nil {
					state.Logout()
				}
			})
		})
	}
}

func accountGroupsSection(c *ui.Context, p *theme.Palette, state *State, lanes []string) {
	disabled := state.SwitchingGroup || state.LoggingOut || state.Stage == SyncingGateway || (state.SelectLaneGroup == nil && state.SelectGroup == nil)
	ui.Box(c).FillWidth().Height(1).Background(p.Border)
	ui.Column(c.Key("account-groups")).FillWidth().Gap(10).Children(func() {
		ui.Row(c.Key("account-groups-header")).FillWidth().AlignItems(ui.Center).Gap(8).Children(func() {
			ui.Text(c, "模型分组").FontSize(14).FontWeight(600).TextColor(p.Foreground)
			if state.GroupStatusFailed() {
				StatusUnknownBadge(c, state.GroupStatusError, false)
			}
			ui.Spacer(c)
			if state.RefreshGroups != nil {
				kit.IconButton(c.Key("account-refresh-groups"), icons.Reload, "刷新分组", 28, false).
					Disabled(state.GroupsLoading || state.SwitchingGroup || state.LoggingOut).
					OnClick(state.RefreshGroups)
			}
		})

		if state.GroupsLoading && len(lanes) == 0 {
			ui.Row(c.Key("account-groups-loading")).Gap(6).AlignItems(ui.Center).Children(func() {
				kit.Busy(c)
				ui.Text(c, "正在加载可用分组…").FontSize(13).TextColor(p.MutedForeground)
			})
		}
		if state.GroupsError != "" {
			ui.Text(c, state.GroupsError).FontSize(12).TextColor(p.Danger)
		}
		if state.SwitchingGroup {
			ui.Row(c.Key("account-groups-switching")).Gap(6).AlignItems(ui.Center).Children(func() {
				kit.Busy(c)
				text := "正在切换分组并刷新模型…"
				if state.SwitchingLane != "" {
					text = "正在更新 " + core.LaneDisplayName(state.SwitchingLane) + " 分组并刷新模型…"
				}
				ui.Text(c, text).FontSize(12).TextColor(p.MutedForeground)
			})
		}
		if state.GroupError != "" {
			ui.Column(c.Key("account-group-error")).FillWidth().Padding(8).Radius(p.Radius).
				Background(p.Danger.Alpha(0.12)).Border(1, p.Danger.Alpha(0.35)).Children(func() {
				ui.Text(c, state.GroupError).FontSize(12).TextColor(p.Danger)
			})
		}

		for _, l := range lanes {
			lane := l
			laneLabel := core.LaneDisplayName(lane)
			boundID := state.BoundGroupID(lane)
			sections := state.GroupSectionsForLane(lane)
			choose := func(groupID *int64) {
				if state.SelectLaneGroup != nil {
					state.SelectLaneGroup(lane, groupID)
				} else if state.SelectGroup != nil {
					state.SelectGroup(groupID)
				}
			}
			ui.Column(c.Key("account-lane-" + lane)).FillWidth().Gap(6).Children(func() {
				ui.Row(c.Key("account-lane-header-" + lane)).FillWidth().AlignItems(ui.Center).Gap(6).Children(func() {
					ui.Icon(c, icons.Network).FontSize(13).Shrink(0).TextColor(p.MutedForeground)
					ui.Text(c, laneLabel).FontSize(13).FontWeight(600).SingleLine().TextColor(p.Foreground)
					ui.Spacer(c)
					if boundID == nil {
						ui.Text(c, "未绑定（自动）").FontSize(12).SingleLine().TextColor(p.MutedForeground)
					} else if !disabled {
						kit.Ghost(c.Key("account-unbind-"+lane), "自动（未绑定）", false).Height(24).PaddingX(8).OnClick(func() {
							choose(nil)
						})
					}
				})
				for _, sec := range sections {
					section := sec
					GroupSectionHeader(c, fmt.Sprintf("account-section-%s-%d", lane, section.Tier), section.Tier)
					for _, g := range section.Groups {
						group := g
						selected := boundID != nil && *boundID == group.ID
						desc := state.DescribeGroup(group)
						accountGroupRow(c, p, fmt.Sprintf("account-group-%s-%d", lane, group.ID), desc, selected, disabled, func() {
							id := group.ID
							choose(&id)
						})
					}
				}
			})
		}
	})
}

func accountGroupRow(c *ui.Context, p *theme.Palette, key string, desc core.GroupPresentation, selected, disabled bool, onClick func()) {
	name := desc.DisplayName()
	border := p.Border
	bg := p.Surface
	if selected {
		border = p.Primary
		bg = p.Primary.Alpha(0.08)
	}
	row := ui.ButtonBase(c.Key(key)).Label(name).Description(desc.CompactSummary()).FillWidth().Padding(10, 12).
		Radius(p.Radius).Border(1, border).Background(bg).Cursor(ui.CursorPointer).
		Checked(selected).Disabled(disabled)
	if !selected && row.Hovered() {
		row.Background(p.Hover)
	}
	row.Children(func() {
		ui.Row(c).FillWidth().Grow(1).Shrink(1).MinWidth(0).AlignItems(ui.Center).Gap(10).Children(func() {
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(4).Children(func() {
				ui.Row(c).FillWidth().MinWidth(0).AlignItems(ui.Center).Gap(6).Children(func() {
					ui.Text(c, name).FontSize(13).FontWeight(500).SingleLine().Shrink(1).MinWidth(0).TextColor(p.Foreground)
					GroupTagsRow(c, desc.Tags, false)
					if desc.Rate != "" {
						ui.Text(c, desc.Rate).FontSize(11).SingleLine().Shrink(0).TextColor(p.MutedForeground)
					}
				})
				if summary := strings.TrimSpace(desc.Group.Description); summary != "" {
					ui.Text(c, summary).FontSize(11).SingleLine().TextColor(p.MutedForeground)
				}
			})
			GroupStatusBadge(c, desc.Status, false)
			if selected {
				kit.StatusPill(c, "使用中", icons.Check, kit.Info, false)
			}
		})
	}).OnClick(onClick)
}
