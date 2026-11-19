package cheapauth

import (
	"fmt"
	"strings"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/i18n"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
	"github.com/lobehub/lobe-mygo/pkg/action"
	"github.com/lobehub/lobe-mygo/pkg/feedback"
	"github.com/lobehub/lobe-mygo/pkg/field"
	"github.com/lobehub/lobe-mygo/pkg/lucide"
	lobetheme "github.com/lobehub/lobe-mygo/pkg/theme"
)

// Gate renders the full-window sign-in page until credentials are established.
// Returning true tells the shell not to render other pages.
func Gate(c *ui.Context, state *State) bool {
	if !state.RequiresLogin() {
		return false
	}
	lobetheme.Apply(c)
	p := lobetheme.TokensOf(c)
	c.Root().Background(p.BgLayout)
	ui.Column(c.Key("cheaprouter-login")).Fill().Padding(32).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
		ui.Column(c).FillWidth().MaxWidth(480).Padding(28).Gap(20).Border(1, p.BorderSecondary).Radius(p.BorderRadiusLG).Background(p.BgContainer).Children(func() {
			ui.Column(c).Gap(6).Children(func() {
				ui.Text(c, "登录 CheapRouter").FontSize(22).FontWeight(600)
				ui.Text(c, "使用 CheapRouter 账号连接模型网关。登录码通过 PKCE 保护，应用不会接触你的账号密码。").FontSize(14).TextColor(p.TextSecondary)
			})
			if state != nil {
				content(c, state)
			}
		})
	})
	return true
}

// Page renders Settings → CheapRouter 账号.
func Page(c *ui.Context, state *State) {
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
	if !state.RequiresLogin() && state.AccountFailure != "" {
		feedback.Alert(c.Key("account-failure"), state.AccountFailure, feedback.AlertProps{Type: feedback.Error, Selectable: true})
	}
	switch state.Stage {
	case SignedOut, Starting:
		disabled := state.Stage == Starting || state.LoggingOut
		ui.Column(c.Key("signed-out")).Gap(16).Children(func() {
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
				ui.Text(c, "正在读取账号、套餐、模型目录和网关密钥，并配置 sailry。").FontSize(13).TextColor(p.TextSecondary)
			})
		})
		action.Button(c.Key("stop-sync"), "取消本次配置", action.ButtonProps{}).
			FillWidth().Disabled(state.LoggingOut).OnClick(func() {
			if state.StopSync != nil {
				state.StopSync()
			}
		})
	case GatewayFailed:
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
				kit.Row(c, "plan", "当前套餐", "", func() { ui.Text(c, state.Account.PlanDisplay()).FontSize(14) })
				kit.Row(c, "balance", "账户余额", "", func() { ui.Textf(c, "$%.2f", state.Account.Balance).FontSize(14) })
			}
			kit.Row(c, "models", "可用聊天模型", "", func() { ui.Text(c, fmt.Sprintf("%d 个", state.ModelCount)).FontSize(14) })
			kit.Row(c, "endpoint", "服务入口", "", func() { ui.Text(c, state.Endpoint).FontSize(13).Selectable() })
			kit.DangerBusy(c.Key("logout"), "退出登录", state.LoggingOut).FillWidth().OnClick(func() {
				if state.Logout != nil {
					state.Logout()
				}
			})
		})
	}
}
