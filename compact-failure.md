# `/compact` 失败排查

## 结论

**已复现「操作失败，请重试」如何吞掉原因；尚不能从这句话判定开发环境是哪一种故障。** 在中文桌面测试窗口直接输入 `/compact`，让测试 Node 对 `compact_context` 返回带消息的 `unavailable`。使用修复前的命令回调时，窗口只显示兜底文案，不显示 Node 的消息。这条路径没有调用摘要模型。

`compact_context` 先检查会话及 `ExpectedRevision`，捕获配置，成功才返回 `queued_turn`。命令被拒绝与已入队的压缩回合失败是两件事：前者原先走 `Runtime.report`，**不经过** `failureKey`；后者有回合记录。当前引擎的 `compactionSummaryFault` 还带 `model_request/invalid_response`，旧的 `failureKey` 会先命中模型响应无效，而不是总是落到兜底。只改这个 switch 修不好即时提示。

## 桌面映射

| 收到的信号 | 标题 | 在哪里看原因 |
| --- | --- | --- |
| `compact_context` 返回普通 fault，未创建回合 | `chat_compact_start_failed`「无法开始压缩」 | 提示副标题显示 `fault.Message`；标准日志输出 `compact_context: code=… message=…`。不记录命令载荷、focus 或凭据；疑似凭据或 URL 的诊断在日志中隐藏。 |
| 命令版本冲突、结果未确认、资源主机离线 | 保留 `session_changed`、`chat_action_unknown`、`resource_offline_action` | 同样附上 Node 消息。未确认不当作已失败，也不自动再发新请求。 |
| 回合 fault 以 `Compaction failed:` 开头，即使也带 `model_request/invalid_response` | `chat_compact_failed`「压缩失败」 | 点开手动压缩行看引擎原文；投影的错误块保留消息与 code。非压缩的 `invalid_response` 不改名。 |
| `compaction_failure` 记录为 `request/request_failed` 或 `reason=no_fitting_prefix` | `chat_compact_failed`；已暂停自动压缩保留 `chat_compact_failed_paused` | 优先显示记录的 `cause`；没有原文才退到 `compact_reason_request` 或 `compact_reason_budget`。手动回合缺少终态 fault 时也可从记录补回原因。 |
| 已完成、历史已加载、无新摘要 | `chat_compact_unneeded`「无需压缩」 | 不是模型失败；不显示错误。尚未加载摘要条目的回合不据此判为无需压缩。 |

`stage/reason` 属于压缩历史记录，不是 `FaultDetailsModelRequestPayload` 的字段。未增加协议变体；沿用已有的回合标题、可展开行和自动压缩错误块。新增命令提示键，已更新并生成全部十种语言的目录。

## 引擎证据

| 已运行的测试 | 关键断言 |
| --- | --- |
| `TestCompactionHandoff/failed_summaries_retain_usage/{malformed,empty,blank}`（加强已有测试） | 非法 JSON 和 `{}` 用 `RetryInvalidOutput`；空白用 `RetryEmptyOutput`。逐字比较第二次请求与「首个请求 + 两个换行 + 对应纠正文案」。连续失败只发两次，分别记账，可见历史不变；原因区分 `unexpected_output` 与 `empty_summary`。 |
| `TestCompactionHandoff/failed_summaries_retain_usage/truncated`（加强）及 `cap_follows_the_window`（已有） | 模型声明输出 64 时实际 `MaxOutputTokens` 为 64；即使返回可解析 JSON，若结束原因为 `MAX_OUTPUT_TOKENS`，也只发一次，记录 `summary/truncated_summary`，不替换历史。上下文 200,000 和 1,000,000、模型输出足够大时，请求上限分别为 8,000 和 24,000。 |
| `TestCompactionHandoff/failures_are_recorded/budget`（已有） | `Prepare` 给输出、系统指令和最长纠正请求预留预算，并保留前次摘要。最新不可拆分的用户输入超预算时，模型调用次数为零，记录 `budget/no_fitting_prefix`，原因准确到 `latest user message exceeds the summary context limit`。 |
| `TestCompactionHandoff/lenient_extraction_and_format_retry/empty_completion_retries_and_repeated_manual_request_is_a_no-op`（新增） | 首次无文本，第二次返回带前言、混合大小写围栏及别名的 JSON。断言恰好两次调用、渲染内容准确、摘要覆盖到短会话最后一条；无新材料再次手动压缩，调用数和记录都不变。已有 `TestConversationProjection/draft_phase_and_failed_diagnostic` 验证桌面区分「无需压缩」、未加载、失败。 |
| `TestParseStructuredAndLenientOutputs`（已有并扩充） | 接受独立或围栏中的摘要、JSON 前后说明、受支持的包装对象、大小写及已知别名（如 `goal`、`state`、`nextSteps`）；不是任意键和任意片段都能成为摘要。拒绝纯说明、仅 Markdown、句中的举例、数组、未结束的分析块和截断对象。 |

`SummaryOutputLimit` 取模型输出上限（未声明时用 8,000）与窗口派生上限的较小值；最终渲染的交接摘要上限又是实际输出预算的一半。**确实可能太小，但没有证据表明本次开发环境发生了截断。** 没有提高模型声明的上限，也没有把不完整的 JSON 当成功。`Select` 通常保留最近约 20% 的上下文（最多 20,000 tokens）；手动请求没有可压缩前缀时会再试一次全部可见历史。因此短会话本身不是跳过压缩的充分条件，确无可总结材料时才返回无需压缩。

**另外修复了两种错误成功**，它们并不能解释即时失败提示。用修改前的 `summary.go` 运行新增断言，两项都失败；修复后均通过：

- `TestParseRejectsHeaderOnlyAfterBounding`：裁剪掉唯一有内容的字段后，不再接受空标题作为摘要。
- `TestParseStructuredAndLenientOutputs/rejected_non-json_outputs/truncated_outer_object_with_complete_file_entry`：外层交接 JSON 在文件数组中截断时，不再把完整的内层文件条目提为交接摘要。

## 开发环境下一次取证

1. 在更新后的客户端操作一次。若立即显示「无法开始压缩」，记录副标题和同一时刻的 `compact_context` 日志。按实际代码检查版本冲突、配置或授权，不要先调输出预算。
2. 若已出现回合，再显示「压缩失败」，点开该行。记录同一回合的 `stage`、`reason`、`cause` 和可用的 HTTP 状态。`budget` 在调用模型前；`request` 查模型请求；`summary` 才查空白、非法 JSON 或输出上限。截断时再核对这次捕获的模型、上下文和输出配置。
3. 只需脱敏的诊断字段；不要发送密钥、完整请求、原始模型响应或对话历史。未知结果先检查原请求，不要另发一条来推断它是否成功。

## 验证记录

- Linux，Go 1.27.3，`CGO_ENABLED=0`。`go generate ./internal/desktop/i18n`、`go build ./...`、`go vet ./...` 均退出 0。
- 四个指定包最终全部通过：`internal/desktop/live`（269.563 s）、`internal/node/agent/engine`（71.084 s）、`internal/node/agent/compact`（缓存复用，前次 0.934 s）、`internal/desktop/i18n`（0.751 s）。首轮 `TestCatalogKeys/SourceKeys` 检出了两个未嵌入的原因键，补入 `keys.txt` 并重新生成后通过；中文回退测试直接校验译文，而不只是比较键。
- 修复前对照：用 Go overlay 临时换回基线 `98e2feb15` 的命令回调和解析器，不切分支。相同测试中，`TestComposerSendRouting/composer_parity_commands` 在中文窗口看到了「操作失败，请重试」而非 Node 消息；上述两项解析器断言也都检出了错误成功。
- 桌面新增 `TestCompactionFaultKeepsEngineMessage`、`TestCompactionResourceExplainsMissingRunFault`、`TestCompactLogMessage`，分别检查原文与标题、历史记录有无 cause 的行为、常见敏感信息不写入日志。扩展已有的 `TestComposerSendRouting/composer_parity_commands`，实际输入 `/compact`，检查拒绝后没有兜底文案。
- 未运行全仓测试、真实模型或用户的开发配置；脚本化响应只能验证上述路径，不能归因到某个真实供应商。
