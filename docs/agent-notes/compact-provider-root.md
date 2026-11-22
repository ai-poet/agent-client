# 旧会话已停用统一 Provider 的 Lane 继任修复结论

## 真因

早期版本把 CheapRouter 的全部模型挂载在统一 provider（`providers.CheapRouterProviderID`）下，旧会话的 `Session.Config.Provider` 因此持久化指向该 ID。
引入分 lane 托管 provider（`providers.CheapRouterLaneID(lane)`）后，路由同步（`applyCheapRoutes`）会创建或更新各绑定 lane 的独立 provider，并将不再承载流量的统一 provider 标记为 `Enabled = false`（停用而非删除，以保留历史引用）。

当用户继续在旧会话中发起对话或执行 `/compact` 时：
- Node 侧严格按会话保存的 `Config.Provider` 解析，不静默替换其他 provider；
- 停用后的 `CheapRouterProviderID` 在准入阶段返回 `not_configured / provider_disabled`（压缩路径报错 `Select an available model before compacting this conversation.`）；
- 即使对应模型已由某个启用中的 lane provider 承载，旧会话仍停留在已停用的统一 provider 上，导致压缩或发送失败。

## 继任规则与边界

1. **继任判定（`providers.CheapRouterSuccessor(provider, model, lanes, current)`）**
   - 仅当 `provider == CheapRouterProviderID` 且该记录仍在 `current` 中且 `Enabled == false` 时才尝试寻找继任。
   - 若统一 provider 仍为 `Enabled == true`（尚未被 lane 替换），不继任。
   - 若统一 provider 已从 `current` 中被移除，保持移除状态，不继任。
   - 若传入的是某个 lane provider（`CheapRouterLaneID(...)`）或用户自定义 provider，没有继任。
   - 归属 lane 解析顺序：先查账号路由表 `lanes[strings.ToLower(strings.TrimSpace(model))]`（经 `strings.TrimSpace` 过滤空白）；未命中时，用停用记录里保存的模型 `Platform` 结合模型 ID 调用 `cheapauth.ModelLane` 推断自身 lane；仍为空则不继任。
   - 仅当目标 lane 对应的 `CheapRouterLaneID(lane)` 在 `current` 中处于 `Enabled == true` 且明确包含该 `model` 时返回继任；目标 lane 被禁用、缺失或不含该模型时，绝不跨到其他恰好提供同名模型的 lane 顶替。

2. **分层职责与执行路径**
   - **Node / Store 层**：维持原契约，不在 `captureSessionProvider` 或 `ClaimNext` 静默代换 provider；未保存继任前对停用/移除的 provider 明确返回 `not_configured`（`provider_disabled` / `provider_removed`），手动压缩返回 `Select an available model before compacting this conversation.`；桌面保存 `set_session_config` 后按新 revision 与新 lane provider/credential 准入，旧 revision 返回 `revision_conflict`。
   - **Desktop Live 层**：
     - 路由同步与改绑 lane 完成后（`saveCheapGateway`、`bindCheapLaneGroup`）调用 `followCheapRoutes`，将 `Config.Provider == CheapRouterProviderID`（排除转发会话 `Profile != nil` 与委托子会话 `delegated(session)`）且配置可直接通过 `ValidateSelection` 的旧会话通过 `set_session_config` 迁至继任 lane。
     - 若旧会话的 `Effort` 等参数与目标 lane 不兼容、或在同步期间发生 revision 变化，则在会话展示（`conversation.go`，当 `cheapRoutesSettled()` 为真）或手动压缩（`conversation_actions.go` 的 `compact`）时由 `followCheapSession` 接管：通过 `applyComposer` 将不支持的 effort 收敛到 lane 默认值，先保存 `set_session_config` 再提交 `compact_context`。
     - 若后台同步已把会话迁至 lane 而当前展示中的 composer 仍停留在旧的统一 route，且统一 route 已停用，则 `followCheapSession` 同步采纳新 `session.Config` 而不重复保存。

## 收尾审查与补充改动

通读已有工作区改动后，`cheaprouter.go` 的导入（`cheapauth.ModelLane`、`slices`、`strings`）及 `IsCheapRouterProvider` 与 `CheapRouterLaneID` 的命名空间语义（`id[6] == 0x80` 保留托管 lane 空间，与 `CheapRouterProviderID` 的 `0x40` 互斥）一致，逻辑自洽。本轮补充了以下收尾项：

- `internal/node/providers/cheaprouter.go`：对 `lanes[...]` 查找结果做 `strings.TrimSpace`，避免仅含空白的映射值跳过 `ModelLane` 回退或生成空 lane ID。
- `internal/node/providers/cheaprouter_test.go`：在 `TestCheapRouterSuccessor` 补齐「模型无 lane → 不继任（`model without a lane`）」以及「路由表指定 lane 被禁用时不跨 lane 顶替（`disabled table lane keeps its model from another lane`）」两条用例。
- `internal/node/store/compaction_provider_test.go`：在 `TestSessionProviderLifecycleAtAdmission` 中补齐保存继任 lane 后对 `compact.Config.Credential` 与 `submit.Config.Credential` 等于 `lane.Credential` 的校验。
- `internal/desktop/live/cheapauth_follow_test.go`：补齐 `kit.ToastHost` 渲染、已移除统一 route 用例（`removed unbound route stays unavailable`），并在所有场景下显式校验「继任存在时 `compact_context` 以目标 lane provider 准入成功且无失败提示」以及「继任不存在时未准入并弹出包含 `chat_compact_start_failed` 与 `Select an available model before compacting this conversation.` 的失败 toast」。
- `internal/desktop/live/cheapauth_test.go`：在 `TestCheapSessionsFollowTheLaneOfTheirModel` 解绑 Codex lane 后的拒绝断言中，补齐对 `compact_context` 返回 `Select an available model before compacting this conversation.` 和 `submit_turn` 返回 `model provider is unavailable` 的文案校验。

## 验证命令与结果

环境：Linux，`unset GOROOT && export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。

1. **全仓构建与静态检查**
   - `go build ./... && go vet ./...`：通过（退出码 0，无告警）。
2. **定向单测（均通过，`-count=1 -p 1`）**
   - `go test -count=1 -p 1 ./internal/node/providers`：通过（0.436s），覆盖 `TestCheapRouterSuccessor` 全部 12 项子用例：
     - `routing_table_lane`
     - `own_lane_without_a_table_entry`
     - `own_lane_by_the_saved_platform`
     - `model_without_a_lane`
     - `unbound_lane_keeps_its_model_from_another_lane`
     - `disabled_table_lane_keeps_its_model_from_another_lane`
     - `lane_without_the_model`
     - `table_lane_without_the_model`
     - `unbound_route_still_serves`
     - `removed_unbound_route`
     - `lane_provider_has_no_successor`
     - `another_provider_with_the_model`
   - `go test -count=1 -p 1 ./internal/node/store`：通过（39.110s），覆盖 `TestMissingSessionProviderAtAdmission`、`TestCompactionProviderAtClaim`、`TestSessionProviderLifecycleAtAdmission`。
   - `go test -count=1 -p 1 ./internal/desktop/live -run 'TestCheap|TestConversation|TestComposer'`：通过（54.991s），覆盖：
     - `TestConversationFollowsTheLaneOfItsModel`（含 `a_shown_conversation_saves_its_lane`、`an_effort_the_lane_cannot_take_becomes_its_default`、`compaction_follows_during_reconciliation`、`a_session_reconciliation_moved_is_adopted`、`another_lane_never_stands_in`、`an_unbound_lane_keeps_its_conversation`、`removed_unbound_route_stays_unavailable`、`the_unbound_route_still_serves`）
     - `TestCheapSessionsFollowTheLaneOfTheirModel`
     - 其余 `TestCheap*`、`TestConversation*`、`TestComposer*` 回归用例。
3. **未跑范围**：按资源约束未运行全仓 `go test ./...` 与 `internal/desktop/shell`，也未连接真实 CheapRouter 账号或真实模型服务。

## 遗留风险与边界说明

- **用户手动禁用 lane provider 或解绑某条 lane**：按设计 lane provider 没有继任，也不会跨到其他恰好提供同名模型的 lane。对应会话的发送按钮会被禁用，手动触发 `/compact` 时由 Node 返回 `not_configured / provider_disabled` 并展示 `Select an available model before compacting this conversation.`，需用户重新启用该 lane 或在模型选择器中手动切到其他可用模型。
- **用户手动删除统一 provider（`remove_provider`）**：`CheapRouterSuccessor` 仅对停用（`Enabled == false`）的统一 provider 生效；若记录已被删除，旧会话不会自动迁移，压缩会报 `provider_removed` 与明确的不可用提示。
- **无路由表且无法推断 lane 的自定义模型**：若统一 provider 停用时既没有当前账号的 `modelLanes` 映射，模型 ID 和保存的 `Platform` 也无法被 `cheapauth.ModelLane` 归类到具体 lane，则不会猜测继任，需用户手动重选模型。
- **远端转发会话与子代理会话**：`session.Profile != nil` 与 `delegated(session)` 的会话不参与自动继任，继续受其原始 profile / 只读约束。
- **同步期并发 revision 冲突**：若 `followCheapRoutes` 写 `set_session_config` 时会话同时被其他操作修改而发生版本冲突，只会记录 warn 日志而跳过该会话，留待下次打开该会话或手动触发 `/compact` 时再由 `followCheapSession` 补齐迁移。
