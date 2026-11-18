package cheapauth

import (
	"fmt"

	core "github.com/ai-poet/agent-client/desktop/internal/cheapauth"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/kit"
	"github.com/ai-poet/agent-client/desktop/internal/desktop/theme"
	"github.com/egoist/mygo/ui"
)

// Gate renders the full-window sign-in page until the account and provider
// are both usable. Returning true tells the shell not to render other pages.
func Gate(c *ui.Context, state *State) bool {
	if !state.RequiresLogin() {
		return false
	}
	p := theme.Of(c)
	c.Root().Background(p.Background)
	ui.Column(c.Key("cheaprouter-login")).Fill().Padding(32).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
		ui.Column(c).Width(480).MaxWidth(480).Padding(28).Gap(20).Border(1, p.Border).Radius(p.RadiusLg).Background(p.Surface).Children(func() {
			ui.Column(c).Gap(6).Children(func() {
				ui.Text(c, "登录 CheapRouter").FontSize(22).FontWeight(600)
				ui.Text(c, "使用 CheapRouter 账号连接模型网关。登录码通过 PKCE 保护，应用不会接触你的账号密码。").FontSize(14).TextColor(p.MutedForeground)
			})
			content(c, state, true)
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
	kit.Page(c, "cheaprouter-account", "CheapRouter 账号", "管理账号登录、套餐信息和模型网关连接。", func() {
		kit.Card(c, func() {
			ui.Column(c).Padding(20).Gap(18).Children(func() { content(c, state, false) })
		})
	})
}

func content(c *ui.Context, state *State, gate bool) {
	p := theme.Of(c)
	if state.Failure != "" {
		ui.Row(c.Key("failure")).Padding(10, 12).Border(1, p.Warning).Radius(p.Radius).Children(func() {
			ui.Text(c, state.Failure).FontSize(14).TextColor(p.Warning)
		})
	}
	switch state.Stage {
	case SignedOut, Starting:
		ui.Column(c.Key("signed-out")).Gap(16).Children(func() {
			ui.Column(c).Gap(6).Children(func() {
				ui.Text(c, "选择服务入口").FontSize(14).FontWeight(600)
				ui.Text(c, "主站与 CDN 连接同一账号和模型网关；网络失败时可切换后重试。").FontSize(13).TextColor(p.MutedForeground)
			})
			for _, endpoint := range []struct{ name, value string }{{"主站", core.PrimaryEndpoint}, {"CDN", core.CDNEndpoint}} {
				selected := state.Endpoint == endpoint.value
				kit.Ghost(c.Key(endpoint.name), endpoint.name+" · "+endpoint.value, selected).FillWidth().Justify(ui.Start).PaddingX(12).Height(36).
					Disabled(state.Stage == Starting).OnClick(func() { state.Endpoint = endpoint.value })
			}
			if state.Stage == Starting {
				ui.Row(c.Key("starting")).Gap(8).AlignItems(ui.Center).Children(func() {
					kit.Busy(c)
					ui.Text(c, "正在准备安全登录…").FontSize(14).TextColor(p.MutedForeground)
				})
			}
			kit.PrimaryBusy(c.Key("start"), "在浏览器中登录", false, state.Stage == Starting).FillWidth().OnClick(func() {
				if state.Start != nil {
					state.Start(false)
				}
			})
			kit.Secondary(c.Key("manual"), "使用粘贴登录码方式").FillWidth().Disabled(state.Stage == Starting).OnClick(func() {
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
			ui.Text(c, detail).FontSize(13).TextColor(p.MutedForeground)
			ui.Row(c.Key("browser-actions")).Gap(8).Children(func() {
				kit.Primary(c.Key("open"), "重新打开登录页", false).Grow(1).OnClick(func() { c.OpenURL(state.LoginURL) })
				kit.Secondary(c.Key("copy"), "复制链接").OnClick(func() {
					c.WriteClipboard(state.LoginURL)
					kit.Copied(c)
				})
			})
			input := ui.TextInput(c.Key("code"), &state.Pasted).Label("一次性登录码").Placeholder("例如 K7QM-3XPD").FillWidth().Disabled(state.Exchanging)
			if input.Submitted() && !state.Exchanging && state.SubmitPaste != nil {
				state.SubmitPaste()
			}
			if state.Exchanging {
				ui.Row(c.Key("exchanging")).Gap(8).AlignItems(ui.Center).Children(func() {
					kit.Busy(c)
					ui.Text(c, "正在兑换一次性登录码…").FontSize(13).TextColor(p.MutedForeground)
				})
			}
			kit.PrimaryBusy(c.Key("submit"), "完成登录", false, state.Exchanging).FillWidth().OnClick(func() {
				if state.SubmitPaste != nil {
					state.SubmitPaste()
				}
			})
			if gate {
				kit.Secondary(c.Key("cancel"), "取消本次登录").FillWidth().OnClick(func() {
					if state.Cancel != nil {
						state.Cancel()
					}
				})
			}
		})
	case SyncingGateway:
		ui.Row(c.Key("syncing")).Gap(10).AlignItems(ui.Center).Children(func() {
			kit.Busy(c)
			ui.Column(c).Gap(4).Children(func() {
				ui.Text(c, "正在连接模型网关…").FontSize(14).FontWeight(600)
				ui.Text(c, "正在读取账号、套餐、模型目录和网关密钥，并配置 sailry。").FontSize(13).TextColor(p.MutedForeground)
			})
		})
		if state.Failure != "" {
			ui.Row(c.Key("sync-actions")).Gap(8).Children(func() {
				kit.Primary(c.Key("retry"), "重试网关配置", false).Grow(1).OnClick(func() {
					if state.Retry != nil {
						state.Retry()
					}
				})
				kit.Secondary(c.Key("relogin"), "返回重新登录").OnClick(func() {
					if state.Cancel != nil {
						state.Cancel()
					}
				})
			})
		}
	case Ready:
		ui.Column(c.Key("account")).Gap(14).Children(func() {
			ui.Column(c).Gap(5).Children(func() {
				ui.Text(c, "已连接 CheapRouter").FontSize(16).FontWeight(600)
				ui.Text(c, state.Account.DisplayName()).FontSize(14)
				if state.Account.Email != "" && state.Account.Email != state.Account.DisplayName() {
					ui.Text(c, state.Account.Email).FontSize(13).TextColor(p.MutedForeground)
				}
			})
			kit.Row(c, "plan", "当前套餐", "", func() { ui.Text(c, state.Account.PlanDisplay()).FontSize(14) })
			kit.Row(c, "balance", "账户余额", "", func() { ui.Textf(c, "$%.2f", state.Account.Balance).FontSize(14) })
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
