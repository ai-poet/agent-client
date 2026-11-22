# 客户端充值与套餐购买

参考版本：`b2d93c15b^`（`609f91ac`）。本次只改客户端；服务端不是本次接口的参考源。上一轮的账号页入口、Kit 弹窗、会话失效处理和测试骨架可以复用；原来只读 `/api/user`、跳过 Stripe 和用退避轮询判断到账的实现已替换。

## Rust → Go 对照

| Rust 文件 / 函数 | Go 文件 / 函数 |
| --- | --- |
| `crates/sub2api/src/lib.rs::api_origin_with`、`gateway_origin.rs::selected_origin`、`cloud_pay.rs::pay_session` | `internal/cheapauth/pay_origin.go::paymentOrigin`、`pay.go::Service.PaySession` |
| `pay.rs::PayConfig/MethodLimit/SubscriptionPlan`、`effective_min/max`、`fee_rate`、`help_lines` | `internal/cheapauth/pay_models.go` 的同名模型及方法 |
| `pay.rs::authed_get/load_config/list_plans/create_order/order_status/cancel_order`、`pay_json/PayError` | `internal/cheapauth/pay.go::PayClient.get/LoadConfig/ListPlans/CreateOrder/OrderStatus/CancelOrder`、`request/PayError` |
| `pay.rs::absolute_url/normalize_order_urls/pay_center_url` | `pay.go::AbsoluteURL/CreateOrder/PayCenterURL` |
| `pay.rs::resolve_flow/payment_label/is_stripe/stablecoin_token/stablecoin_network`、`PayOrder/OrderStatus` | `pay_models.go`、`pay_labels.go` 的同名类型和函数；订单通过 `Seed()` 生成首帧状态 |
| `promotion.rs::compute_bonus/pick_best_promotion/next_promotion_hint/merge_quick_amounts/active_promotions/describe_promotion` | `internal/cheapauth/promotion.go`、`pay_labels.go`；订单实际赠送额取 `PayOrder.Bonus()` |
| `cloud_account.rs::CloudAction::TopUp`、`cloud_pay.rs::open_cloud_pay/open_plan_purchase` | `internal/desktop/cheapauth/view.go` 的余额、套餐入口；`internal/desktop/live/recharge.go::openPayment` |
| `cloud_pay.rs::render_pay_form/render_top_up_amount/render_pay_methods/render_stablecoin_notice` | `internal/desktop/cheapauth/recharge.go`、`pay_form.go`；套餐列表与报价在 `pay_plans.go` |
| `pay.rs::help_image`、`cloud_pay.rs::cloud_pay_load_help_image/render_pay_help` | `internal/cheapauth/pay_image.go::LoadHelpImage`，live 解码，Kit 弹窗显示 |
| `cloud_pay.rs::cloud_pay_create_order/render_pay_paying/cloud_pay_start_polling/cloud_pay_apply_status/cloud_pay_cancel_order` | `internal/desktop/live/recharge.go::submit/watch/applyStatus/cancelOrder`；`internal/desktop/cheapauth/pay_order.go::orderView/PaymentSummary` |
| `cloud_pay.rs::cloud_pay_settled/cloud_pay_open_full_center`、账户刷新与套餐激活 | `recharge.go::applyStatus/refreshAccount/loadSubscriptions/openCenter/watchHosted`；`internal/cheapauth/pay_account.go` 读取订阅、兑换充值码 |

## 实现要点

- `PayConfig` 是聚合视图，不是单个 JSON 响应。先读 `/pay/api/orders/my?page=1&page_size=20` 得到账户和待付数，再用服务返回的账户 ID 读 `/pay/api/user`。配置和套餐读取先发 Bearer，仅在 400/401 时用 `?token=` 重试一次。下单、取消按 Rust 契约用 JSON body token；查单只用 `?access_token=<订单令牌>`，不发账号 Bearer。
- 刷新会话仍走登录端点。只有当前 profile 的 `gateway-origin.json` 指定仍在候选列表中的节点，业务请求才改用该节点；否则用登录端点。收银台根相对链接按站点 origin 补全，`//` 链接补 `https:`；`weixin://` 等扫码载荷不作为系统浏览器命令执行。
- 原生表单默认 `$10`，快捷金额合并活动门槛并按分去重。费率、单笔上下限、每日额度、剩余额度和待付数取自服务。可领取且在有效期内的优惠才参与预估；同额按 `sortOrder`、`id` 决胜，不叠加。赠金按分计算，百分比向下取整，总到账不超过 `$99,999,999.99`。指定 Rust 源中没有名为 `buyer_bonus` 的字段或函数；买方赠金对应预估的 `compute_bonus` 和订单快照 `bonusAmount`，未另造协议字段。
- 复用仓库已固定的 `rsc.io/qr` 在本地生成二维码，不把订单载荷发给图片服务，不新增依赖。支持复制载荷、打开合法支付链接；微信直连只有二维码时保留扫码流程。Stripe 走带已续期会话的托管收银台，不在桌面伪造 checkout URL。稳定币转账在确认币种和网络后才创建订单。
- 每秒更新倒计时，每两秒用订单令牌查单，最长五分钟。暂时查单失败继续观察；`PAID` 不等于已到账。关闭页面只停止观察。用户取消时先查最后状态，已支付则不发取消请求。创建响应丢失时不自动重发；用户可去收银台检查原订单。
- 余额到账刷新账户；套餐到账保留激活页，刷新订阅、到期时间和分组。Go 版本在现有账号页的 Kit 弹窗中选购套餐，不恢复 Rust 的整个设置页。托管支付没有浏览器回调，桌面短时观察余额或订阅到期时间变化，不把打开浏览器当作支付成功。

## 验证

环境：Linux amd64，`unset GOROOT`，`GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。

| 命令 | 覆盖内容 |
| --- | --- |
| `go generate ./internal/desktop/i18n` | 从中英文源目录生成嵌入目录；其他语言缺失的键按现有机制回退 |
| `go build ./...`、`go vet ./...` | 全仓编译、静态检查 |
| `go test ./internal/cheapauth ./internal/desktop/cheapauth ./internal/desktop/i18n` | 新增/重做本地 HTTP 契约、凭据隔离、错误、URL、活动、表单与状态测试；同时运行已有账号与语言目录测试 |
| `go test ./internal/desktop/live -run '^(TestRecharge\|TestCheap)'` | 新增 `TestRechargeWindowOrderLifecycle`，并运行现有 CheapRouter live 测试。真实窗口控制器连接本地 fixture：展示二维码、自动查单、取消待付单、取消前发现已到账而不再取消、刷新余额、买套餐并读回订阅到期时间 |

上述命令全部通过；全仓 build、vet 无诊断，四个定向测试包均返回 `ok`。未运行全仓测试、真实模型或真实支付。fixture 使用临时 profile 和环回 HTTP，不读取个人凭据。

## 需要人工确认

- 在目标操作系统上确认小窗口、高 DPI 下的二维码可扫，帮助图片清晰，系统浏览器与原生取消确认框正常。
- 在获准使用的预发布收银台确认所部署 Stripe / 支付渠道能打开并接受兼容参数；核对服务端配置的费率、优惠档位和套餐到账延迟。自动化结果不代表真实渠道已清算。
