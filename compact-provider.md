# `/compact` 与会话的模型选择

**对同一会话“普通发送正常，手动压缩失败”的判断是 B：两条桌面路径没有使用同一份已保存的模型选择。** A（已入队回合缺少 provider 快照）也能单独复现，但不是 B 的直接后果。未读取用户的开发配置；单凭 `unavailable` 不能判定某次现场故障是哪一种。

## 如何区分

| 修复前的条件 | 观察结果 |
| --- | --- |
| 同一会话的 `Config.Provider` 指向不存在的 ID，另一个 provider 也提供同名模型 | 直接提交 `compact_context` 和 `submit_turn` 都在 `captureTurnSettings` 返回 `not_configured / provider_missing`，消息为 `model provider is unavailable`。没有压缩回合可供 claim。 |
| 桌面上已经选了新的 provider，`set_session_config` 还没完成 | 普通发送通过 `withComposerConfig` 等待保存，使用新 revision；手动压缩直接使用旧 revision。取决于命令顺序，Node 可能看到旧配置，也可能报版本冲突。 |
| 先以有效配置入队，再把 `agent_runs.provider` 改成 JSON `null` | claim 成功但 `inv.Provider == nil`。引擎回合失败，原文为 `captured model provider is unavailable`。SQL `NULL` 或损坏的 JSON 不同：原先会在 claim 时解码报错，尚未进入引擎。 |

## 修法与边界

- `CheapRouterLaneID` 由 lane 名决定，不随同一 lane 的组或密钥轮换变化。`applyCheapRoutes` 更新托管 provider，不代表用户选择把所有既有会话迁到新 lane。没有在登录或切组时批量改写会话。
- 桌面 `/compact` 和压缩按钮现在与普通发送一样，先完成当前会话的配置保存，再用返回的 revision 提交压缩。保存失败不提交。若只是凭据轮换，保留同一 provider、模型和其他会话设置；若当前选择失效且用户未另选，不搜索其他 lane。
- Node 在准入时保留 `not_configured` 及原有的 missing、removed、disabled 原因；手动压缩现在提示 `Select an available model before compacting this conversation.`。正常消息的文案不变，凭据、权限、版本冲突和无效模型等错误没有改写为缺少 provider。
- 仅对**已经入队且快照缺失或无法解码**的压缩回合，claim 复用准入时的 provider 解析并在同一事务修复快照。只接受会话当前仍选择的原 provider ID 和模型；转发会话仍用源 Node 的 profile。若已改选则结束旧回合，不执行其他 lane 的模型。有效快照即使遇到切 lane 也照旧使用，不重新抓取凭据或改写会话历史。

## 回归测试

| 新增测试 | 验证内容 |
| --- | --- |
| `store.TestMissingSessionProviderAtAdmission` | 比较同一失效选择上的两条真实命令，检查错误码、原因、压缩提示和普通发送原文。 |
| `store.TestCompactionProviderAtClaim` | JSON `null`、SQL `NULL`、非法 JSON 均恢复原 provider；切至同名模型的新 lane 后，缺失快照明确失败，完好快照不被替换。 |
| `engine.TestCompactionWithoutProviderCapture` | 实际启动引擎处理无快照、原 provider 也不存在的压缩回合。修复前确认原文为 `captured model provider is unavailable`，修复后检查终态及可操作的诊断。 |
| `live.TestComposerSendRouting` 新增的 `compact_after_model_save`、`compact_after_failed_model_save`、`compact_after_credential_rotation` | 桌面配置保存和压缩的命令顺序、精确 provider 和凭据、revision、focus，以及保存失败后不提交。 |

## 验证结果

Linux，`GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。`go build ./...` 和 `go vet ./...` 均退出 0。最终定向测试 `go test -p 1 ./internal/node/store ./internal/node/agent/engine ./internal/desktop/live` 三包全部通过：store 41.036 s，engine 54.541 s，live 300.557 s。已有的桌面命令测试还在测试窗口键入 `/compact`，检查 focus 和 Node 的错误副标题。

首次运行时，内存传输返回太快，两项桌面新用例未稳定捕获“保存中”。测试夹具现将保存回调延后到压缩动作注册后处理；未放宽命令顺序或失败断言。三个新增桌面用例连续运行三次均通过，随后完整的三个指定包通过。未运行全仓测试、真实账号或模型。
