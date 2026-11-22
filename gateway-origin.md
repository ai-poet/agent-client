# 网关端点切换（Gateway Origin Switching）

参考版本：`b2d93c15b^`。本次仅改客户端，不改动 `sub2api` 或线协议，不引入新依赖（HTTP 并发测速直接使用 Go 标准库 `net/http`）。

## Rust → Go 映射表

| Rust 源码 (`b2d93c15b^`) | Go 实现 | 说明 |
| --- | --- | --- |
| `crates/sub2api/src/gateway_origin.rs::GatewayOriginConfig`, `ServerNode`, `NetworkTimeouts`, `NodeLabel` | `internal/cheapauth/gateway_origin.go` 同名结构体与方法 | 配置落盘于 `<profile>/gateway-origin.json`（`0600` 权限、原子写），字段 `candidates` / `chosen` / `auto_select` / `server_nodes` / `server_nodes_fetched_at` / `fallback_from` / `network` 与 Rust 一致；文件缺失或损坏时回退默认配置 |
| `gateway_origin.rs::effective_candidates`, `is_removable`, `node_label`, `server_nodes_due`, `adopt_server_nodes`, `selected_origin`, `origin`, `select`, `fall_back`, `restore_target`, `add`, `remove`, `probe_target` | `internal/cheapauth/gateway_origin.go` 对应方法与 `ProbeTarget`, `ApplyOriginTest` | 候选合并顺序：构建内置（主站 + CDN）→ 服务端公开节点（仅裸 HTTPS、去重、最多 10 条、6 小时 TTL 缓存）→ 用户手动添加；`chosen` 为空时走默认/登录端点 |
| `custom_api.rs::normalize_base_url`, `ProbeVerdict`, `ProbeResult` | `internal/cheapauth/gateway_origin.go::NormalizeGatewayURL`、`speedtest.go::ProbeVerdict`, `ProbeResult` | 补全 `https://`、小写 scheme、去除尾 `/` 与 `/v1`、拒绝空白/非 http(s)/含 userinfo 或 query/fragment 的地址 |
| `crates/sub2api/src/speedtest.rs::test_candidates`, `probe_twice`, `fastest_ok`, `latency_tier`, `clamp_timeout`, `model_ids_from_body` | `internal/cheapauth/speedtest.go::TestCandidates`, `probeTwice`, `FastestOK`, `SortCandidateResults`, `SortOriginsByResults`, `ClassifyLatency`, `ClampSpeedTestTimeout`, `ModelIDsFromBody` | 并发有界测速（最大并发 6，超时范围 2–30s 默认 8s）；先预热再计时，使用账号网关 key 请求模型列表（Claude 走 `x-api-key` + `anthropic-version`，Codex/通用 key 走 `Authorization: Bearer`），结果按可用性 + 延迟升序排列 |
| `crates/sub2api/src/client.rs::public_custom_endpoints`, `PublicEndpoint` | `internal/cheapauth/client.go::Client.PublicCustomEndpoints`、`gateway_origin.go::PublicEndpoint` | 匿名请求 `GET /api/v1/settings/public` 读取 `custom_endpoints`，兼容 `null` 或缺省字段 |
| `crates/sub2api/src/lib.rs::api_origin_with`, `api_origin`, `authenticated` | `internal/cheapauth/service.go::Service.gatewayOrigin`, `Service.apiSession`、`pay_origin.go::gatewayOriginForProfile` | 统一业务请求 origin 解析：认证续期仍在登录端点完成，业务客户端改走 `gatewayOrigin(credentials.Endpoint)`；未显式选择节点时保留自建登录端点 |
| `src/app/cloud_origins.rs::select_gateway_origin`, `fall_back_gateway_origin`, `add_gateway_origin`, `remove_gateway_origin`, `refresh_gateway_nodes`, `verify_gateway_origin`, `run_gateway_origin_test` | `internal/desktop/live/gateway_origin.go` 对应 `Runtime` 方法 | 切换节点时关闭已打开的充值/套餐弹窗，向新节点刷新账号/分组/目录并更新 Node Provider；若新节点不可用或写回失败，自动回退 `gateway-origin.json` 与 UI 到原节点并提示错误 |
| `src/app/cloud_origins.rs::render_gateway_origins`, `NodeMenu` & `src/app/cloud_menu.rs` 节点子菜单 | `internal/desktop/cheapauth/gateway_origins.go`、`state.go`、`view.go` & `internal/desktop/shell/account_footer.go` | 沿用现有 MyGo + Kit 视觉框架，在账号设置页与侧边栏账号弹层提供节点选择、测速、自动选择和自定义节点管理 |

## 登录端点 vs 业务节点边界

### 始终留在登录端点（`credentials.Endpoint`，切换节点绝不重写）

- 浏览器登录桥接与本地回调校验：`Service.StartLogin`、`Service.StartManualLogin`、`LoginURL`、`ManualLoginURL`、`LoginFlow.serveHTTP`（`/callback`、`/deliver`）、`LoginFlow.ExchangePasted`
- 一次性登录码兑换：`Client.Exchange`（`POST /api/v1/auth/desktop-session/exchange`）
- 令牌续期：`Service.Refresh`、`Service.refreshLocked`、`Client.Refresh`（`POST /api/v1/auth/refresh`，由 `Service.session` 在构建业务客户端之前执行）
- 退出撤销：`Service.Logout`、`Client.Logout`（`POST /api/v1/auth/logout`）
- 凭据持久化：`Credentials.Endpoint` 在 `cheaprouter-account.json` 中保持不变，`Service.Endpoint()` 始终返回登录端点

### 跟随选定网关节点（经 `Service.gatewayOrigin` / `Service.apiSession` 解析）

- 账号信息：`Service.Account`（`GET /api/v1/auth/me`）
- 模型目录、密钥与内置 Agent 路由：`Service.ReadSetup`、`Service.Gateway`、`Service.GatewayWithGroups`、`Service.GatewayForGroup`、`Service.GatewayForLaneChange`、`Service.AutoBindUnboundLanes`（显式选择节点时 `Gateway.BaseURL` 覆盖为 `<chosen>/v1`，供 Node Provider 和内置 Agent 使用）
- 模型分组与健康状态：`Service.AvailableGroups`（`GET /api/v1/groups/available`）、`Service.GroupStatuses`（`GET /api/v1/group-status`）
- 用量统计：`Service.UsageStats`（`GET /api/v1/usage/stats`）
- 充值、套餐、兑换码与订阅进度：`Service.PaySession`（`PayClient` 的 `/pay/api/*`、`/pay`、`/api/v1/redeem`、`/api/v1/subscriptions/progress`）
- 画图（Image Studio）：`Service.ImageRouteKey`（在选定节点拉取/创建密钥并返回选定节点 origin 供画图请求使用）
- 公开节点刷新：`Service.RefreshServerNodes`（`GET /api/v1/settings/public` 优先请求登录端点，主站不可达时回退到当前已选节点）

## UI 说明

不改动现有视觉框架，复用 `internal/desktop/kit` 与 `internal/desktop/theme`：

1. **设置 → CheapRouter 账号（`internal/desktop/cheapauth/gateway_origins.go`）**
   - 在已登录态（`Ready` 及 `GatewayFailed` 恢复界面）展示「云端 API 节点」分区，并保留原有「服务入口」行显示登录端点。
   - 展示所有候选节点（主站、CDN、服务端公开节点、用户自定义节点），测速后按「可用且延迟升序 → 不可用 → 未测速」排序；每行显示节点名称、URL、说明、延迟/状态标签（`ms` / `密钥被拒绝` / `HTTP xxx` / `无法连通` / `URL 无效`）、「使用中」或「使用」按钮，以及仅对用户自建节点显示的「移除」按钮。
   - 提供「全部测速」按钮、自定义域名输入框 +「添加域名」按钮、以及「测速后自动选择最快节点」开关；处于自动回退状态时提示原节点恢复后将自动切回；切换失败时展示回退错误提示与 Toast。
2. **侧边栏左下角账号弹层（`internal/desktop/shell/account_footer.go`）**
   - 在模型分组入口下方新增「API 节点」二级菜单入口，显示当前节点简称。
   - 展开后可查看候选节点列表及测速结果、点击切换节点、开关「测速后自动选择最快节点」、点击「全部测速」或「管理节点…」跳转至账号设置页。
3. **多语言文案**
   - 新增键写入 `locales/cheaprouter-account.en.yml`、`locales/cheaprouter-account.zh-CN.yml` 与 `internal/desktop/i18n/keys.txt`，并通过 `go generate ./internal/desktop/i18n` 重新生成嵌入目录。

## 测试与验证结果

环境：Linux amd64，`unset GOROOT`，`export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。

### 构建与静态检查

- `go build ./...`：通过（exit 0）
- `go vet ./...`：通过（exit 0）
- `go generate ./internal/desktop/i18n`：通过

### 定向测试明细

| 包 | 测试名 | 结果 | 覆盖点 |
| --- | --- | --- | --- |
| `internal/cheapauth` | `TestGatewayOriginBuiltInDedupAndLabels` | PASS | 内置主站/CDN 顺序、归一化去重、可移除性、Primary/Server/CDN/Custom 标签优先级 |
| `internal/cheapauth` | `TestGatewayOriginServerNodesFilteringTTLAndCap` | PASS | 仅保留裸 HTTPS、去 `/v1`、去重、最多 10 条上限、6 小时 TTL 判定、服务端移除节点后回退默认 |
| `internal/cheapauth` | `TestGatewayOriginRefreshServerNodesTTLAndFallbackOrigin` | PASS | TTL 内不重复拉取 `/settings/public`、过期后主站不可达时回退已选节点拉取 |
| `internal/cheapauth` | `TestGatewayOriginAutoSelectAndFallbackRestore` | PASS | `auto_select` 选最快可用、全部不可用时保留原节点并报错、`fallback_from` 记录并在原节点恢复后自动回切、手动选择清除回退标记 |
| `internal/cheapauth` | `TestGatewayOriginStorePermissionsAtomicAndCorruptRecovery` | PASS | `gateway-origin.json` `0600` 权限、原子写无残留临时文件、全字段往返、损坏 JSON 回退默认、旧版 JSON 兼容 |
| `internal/cheapauth` | `TestAuthStaysOnLoginOriginWhileBusinessTrafficUsesChosenOrigin` | PASS | 切换 `chosen` 后刷新令牌与退出撤销始终打到登录端点、凭证 `Endpoint` 不变；账号/目录/密钥/分组/分组状态/用量/支付/画图及 `Gateway.BaseURL` 全部打到 `chosen` |
| `internal/cheapauth` | `TestSpeedTestConcurrencyHeadersOrderAndSorting` | PASS | 并发测速、Claude(`x-api-key` + `anthropic-version`) 与 Codex(`Bearer`) 请求头、输入顺序保留、可用性+延迟排序、无效 URL/401/模型解析 |
| `internal/cheapauth` | `TestPaymentOriginSelection` 及既有全部 `internal/cheapauth` 测试 | PASS（1 skipped：`TestSignOutEmptiesFileItCannotRemove` 因当前 root 环境可在只读目录删文件） | 支付 origin 复用统一解析，既有认证/自动绑定/支付测试全部通过 |
| `internal/desktop/cheapauth` | `TestAccountGatewayOriginsUI` 及既有全部 `internal/desktop/cheapauth` 测试 | PASS（1 skipped：`TestLoginSnapshots` 未设环境变量） | 账号页节点排序展示、测速、切换、移除、添加域名、自动选择开关、`fallback_from` 提示 |
| `internal/desktop/i18n` | `TestCatalogKeys`, `TestPlaceholdersAndMatch` | PASS | 全部新增 i18n 键在中英文目录和源码中完整匹配 |
| `internal/desktop/shell` | `TestSidebarAccountPopoverGatewayOriginSubmenu`、`TestSidebarAccountFooterSignedOutAndSignedInPopover`、`TestSidebarAccountPopoverOpensRechargeAndPlansDirectly`、`TestShellButtonCaptionsAppearOnce` | PASS | 账号弹层「API 节点」二级菜单打开/测速/选节点/跳转设置页，且未引入重复按钮标题 |
| `internal/desktop/live` | `TestCheapGatewayOriginSwitchUpdatesProvidersAndRollsBackOnFailure` 及全部 `TestCheap*`、`TestRechargeWindowOrderLifecycle` | PASS | 真实 Node + Live 控制器切换节点更新 Provider `Endpoint`、切到故障节点时回退配置与 Provider 并显示错误提示、退出登录仍向原登录端点撤销 |

注：未运行全量仓库测试（按要求仅执行上述定向包测试）；未连接真实外部 CheapRouter 主站或 CDN。

## 需人工确认项

1. **真实环境连通与测速表现**：在 macOS / Windows 实机使用真实 CheapRouter 账号，确认 `GET /api/v1/settings/public` 返回的 `custom_endpoints` 正确展示在设置页与账号弹层中，并核对主站、CDN 与自定义节点的实测延迟与切换速度。
2. **弱网/主站不可达场景**：实测主站不可达时，确认已缓存的 `server_nodes` 仍可选、启动检测能自动回退至可用节点（写入 `fallback_from`），以及主站恢复后再次测速能自动切回。
3. **跨业务一致性**：切换节点后人工验证聊天对话（内置 Agent）、画图（Image Studio）、充值/套餐弹窗与分组切换均走新节点，且令牌自动续期与退出登录仍走登录端点。
4. **窗口视觉与小尺寸适配**：在高 DPI 与窄窗口下确认设置页节点行、自定义输入框及侧边栏账号弹层「API 节点」二级菜单排版与滚动正常。
