# Agent 画图（生成 + 编辑）与内建 Skill 实现说明

> 对应方案：`docs/agent-notes/image-skill-plan.md`（第 3 节）
> 分支：`feat/agent-image-skill`

---

## 1. 改动清单

### 1.1 `internal/imagestudio` & `internal/cheapauth` & `internal/desktop/live`（共享编排 + 编辑能力）
- `internal/cheapauth/service.go`
  - `Restore()` 增加 `credentialLock` 同步，确保同进程多实例在登录/登出/Token 轮换后读取一致。
  - 新增 `ModelCatalog(ctx)`，供 Node 按需获取 CheapRouter 模型目录（含分组路由元数据）。
- `internal/imagestudio/types.go`
  - 新增 `ReferenceImage`、`DecodedImage`；`GenerateSpec` 扩展 `ResponseFormat`、`References`（`json:"-"` 不落盘至 `index.json`）及 `IsEdit()`。
  - `ImageError` 扩展 `TaskID` 与 `TaskEnded`，区分「服务端明确失败」与「已受理但结果未知（超时/断连）」。
- `internal/imagestudio/options.go`
  - 新增 `DefaultCheapRouterModel`（`gpt-image-2`）、`MaxReferenceBytes`（20 MiB）、`PickDefaultImageModel`、`MimeTypeForExt`、`EditsURL`（兼容 `origin`、`.../v1`、`.../images/generations`、`.../images/edits`）。
- `internal/imagestudio/store.go`
  - `IndexStore` 新增 `LoadDenials(now)` 与 `RecordDenials(model, groupIDs, now)`，在不覆盖创作工作台 `Prefs`/`Jobs` 的前提下安全读写 24h 分组拒绝缓存。
- `internal/imagestudio/client.go`
  - `NewClient()` 接入 `netproxy.Transport()`，新增 `WithTimings(pollInterval, taskTimeout)` 便于确定性测试。
  - 新增 `GenerateEdits(ctx, endpoint, apiKey, spec, extra)`：`POST /v1/images/edits` multipart（`image` 可多张、`prompt`、`model`、`size`、`n`、`quality`、`response_format` 及 `ExtraParams`），支持 `b64_json` / `data:` URL / 外链 `url`，复用 `sanitizeSecret`、50 MiB 下载上限（不携带 `Authorization` 头）、`ClassifyError`。
  - `GenerateCheapRouter` 支持编辑路径（`POST /v1/images/edits/async` + 轮询 `/v1/images/tasks/:id` + 自动降级 `POST /v1/images/edits` SSE/JSON），并在异步失败/超时/取消时透传 `TaskID`。
  - `GenerateCustom` 在 `spec.IsEdit()` 时转发至 `EditsURL` + `GenerateEdits`；当第三方接口返回 `404`/`405`/`501` 时给出明确的不支持 `/v1/images/edits` 错误说明。
  - 抽出 `DecodeOutput(ctx, output)`，供桌面 `SaveOutputs`（原子 `.part` → 重命名）与 Node 受限落盘共用。
- `internal/imagestudio/runner.go`
  - 抽出共享 `Runner`（含 `AuthProvider` 接口），统一封装 CheapRouter 候选分组试探、24h Denial 过滤/持久化、`RememberImageGroup` 回写、异步任务轮询与 SSE 降级、自定义接口执行及 `ctx` 取消语义，并在已拿到 `task_id` 且结果未知时禁止自动切换分组重试。
- `internal/desktop/live/imagestudio.go`
  - 移除重复的 `runCheapRouterImageJob`、`runCustomImageJob`、`pickDefaultImageModel`，改为委托 `imagestudio.Runner`；保留工作台 UI 的实时分组/任务进度展示、缩略图加载与 `~/Pictures/CheapRouter` 落盘行为。

### 1.2 `internal/node`（薄内置工具、路由、多图与编辑）
- `internal/node/agent/operations/media.json`
  - 向后兼容扩展 `generate_image`（`operation: "media.image"`，未改线协议枚举），新增可选入参：`channel`、`model`、`size`、`quality`、`count`（1..4）、`references`（1..8）。
- `internal/node/node.go`
  - `Node` 挂载 profile 绑定的 `nodeImageStudio`；`Options` 新增可选 `ImageStudio *ImageStudioOptions` 便于确定性测试与同进程服务注入。
- `internal/node/media_image_studio.go` & `internal/node/media_tools.go`
  - 实现 6 级路由优先级与无可用后端时不下发 `generate_image` 工具。
  - 在 `generate_image` 工具描述中动态拼接脱敏后的可用后端摘要（渠道、默认/可用模型 ID、自定义接口 Host，**不含任何密钥或凭证**）。
  - 支持 `references` 参考图编辑：兼容工作树相对路径（经 `core.OpenFiles(..., core.RestrictedFiles)` 受限读取）与当前会话附件 ID（校验 worktree 归属），检查 20 MiB 上限与 PNG/JPEG/WebP 魔数。
  - 产物写入 `<worktree>/assets/generated/`（多图自动追加 `-1`/`-2` 后缀，发请求前预占/冲突检测拒绝覆盖，未显式命名时按实际嗅探格式调整扩展名），所有图片均写入 Node SQLite Blob（`store.PutBlob`）并作为 `Attachments` 返回，JSON 结果不含 base64 原文。
  - 按方案 3.3 完成错误映射（复用现有 `protocol.ErrorCode` 与 `FaultDetails`，未改线协议枚举），已提交 `task_id` 后超时/断连返回 `ErrorCodeOutcomeUnknown` 并携带 `task_id` 与禁止盲目重试说明；严格透传 `ctx`，取消即中止轮询并返回 `ErrorCodeCancelled`。
- `internal/node/store/conversation_assets.go`
  - 新增 `generatedImageAssetFiles`（保留 `generatedImageAssetFile` 兼容封装），放宽至支持多图 `paths` 与多 `Attachments` 严格一一校验并登记到会话资产清单。

### 1.3 内建 Skill（不改 `plugins/` 子模块与 `BundledDefaults()`）
- `internal/node/agent/operations/skills/image-generation/SKILL.md`
  - 覆盖工具发现、渠道与默认模型路由、Prompt/尺寸/质量/张数/命名规范、`references` 参考图编辑说明、Markdown 相对路径嵌入展示、错误引导与防重复扣费规则，**不包含任何密钥或凭证路径**。
- `internal/node/agent/operations/builtin_skills.go` & `internal/node/agent/operations/resources.go`
  - 通过 `//go:embed skills` 嵌入，在 `operations.CaptureResources` 自动注入 `builtin:image-generation`；`Resources.Read` 直接读取内嵌 UTF-8 文本（不落盘、不走插件安装器），`Resources.Python` 返回 `(nil, nil)`。
  - 未触碰 `plugins/` 子模块，`BundledDefaults()` 仍保持 24 个随附包。

---

## 2. 路由矩阵与测试覆盖

所有测试均基于 `httptest.Server` 与 `t.TempDir()` 确定性 fixture，无外网与真实模型依赖。

| 测试包与用例 | 覆盖场景 |
| --- | --- |
| `internal/imagestudio/runner_test.go` | `GenerationsURL`/`EditsURL` 路径解析；CheapRouter 异步 `202` → `processing` → `completed` 轮询；外链 `url` 下载校验**不携带 `Authorization` 头**与原子 `.part` 落盘；`404 async unavailable` 自动降级 SSE 保活并记忆禁用；Grok 同步 JSON（忽略 `quality`、`response_format=b64_json`）；多分组 403/404 回退 + `IndexStore` 24h Denial 持久化与过期重试 + `RememberImageGroup`；Custom API `ExtraParams` 与密钥脱敏（`[redacted]`）；`ctx` 取消中止轮询；已提交 `task_id` 超时返回 `ErrTimeout`+`TaskID` 且不重试其他分组；`GenerateEdits` 单/多参考图 multipart（Custom `image` 与 CheapRouter `/v1/images/edits/async` `image[]`）与 Custom `405` 不支持 edits 报错 |
| `internal/node/media_image_studio_test.go` | 完整 6 级路由优先级矩阵（显式 `channel`/`model` → Media Settings 显式 Image → CheapRouter 登录态 → `image-api.json` → 对话模型原生能力 → 全无时不下发工具）；显式指定未配置渠道时诚实返回 `NotConfigured`、不静默切换其他付费通道；工具描述摘要脱敏校验（无 API Key 泄露）；完整错误码映射表；端到端 Node turn：多图（`count: 2`）落盘 + Blob + `ListConversationAssets` 登记 + JSON 无 base64；`references` 同时传入工作树文件与会话附件 ID 改图；Custom API 改图与 `405` 报错；路径穿越与多图重名冲突在 `Prepare` 拒绝；`ctx` 取消返回 `Cancelled` |
| `internal/node/media_tools_test.go` | 原有 30+ 回归用例全部通过（Media routing、Plan 模式拒绝、审批记忆、预占文件防重、视频 job 恢复、语音合成） |
| `internal/node/store/conversation_assets_test.go` | 单图与多图 `generatedImageAssetFiles` 校验（数量一致、越界路径拒绝、单路径多附件歧义拒绝） |
| `internal/node/agent/operations/resources_test.go` | `TestBuiltinImageGenerationSkill`（零插件下可见、Catalog/Read 正常、无密钥标记、越界/缺失拒绝、`Python` 返回 nil）及既有 `TestSkillLoad`/`TestBundledSkillPublication`/`TestModeMapping` |
| `internal/node/plugins` (`TestBundledPackageContracts`) | 验证 `len(BundledDefaults()) == 24` 与 24 个官方随附包契约未受影响 |

---

## 3. `edits` 接口细节与限制

1. **端点与 multipart 字段**：
   - **CheapRouter 通道**：对齐网关 `sub2api` 约定，优先提交 `POST /v1/images/edits/async`，若异步不可用自动降级 `POST /v1/images/edits`（OpenAI 系走 `stream=true` SSE 保活，Grok 走同步 JSON）；参考图字段名为 `image[]`（支持多张）。
   - **自定义 Image API 通道**：通过 `EditsURL(baseURL)` 解析 `POST .../images/edits`（支持用户配置 `origin`、`.../v1` 或直接填写了 `.../images/generations`），参考图字段名为 `image`（可重复多张）；文本字段包含 `model`、`prompt`、`n`，可选 `size`、`quality`（Grok 自动省略）、`response_format`（Grok 默认 `b64_json`，`gpt-image-*` 不主动附加以免报 400）及 `ExtraParams` 标量键值。
2. **参考图来源与校验**：
   - `references` 支持 1–8 项，每项可以是**会话附件 ID**（须归属当前 worktree）或**工作树相对路径**（经 `RestrictedFiles` 受限读取，拒绝路径穿越）。
   - 魔数白名单限定 PNG / JPEG / WebP，单张上限 **20 MiB**（防止 CheapRouter 网关静默截断）。
3. **响应兼容**：
   - 同时兼容 `b64_json`、`data:` URL 与外链 `url`（外链下载上限 50 MiB，不携带任何 `Authorization` 头）。
4. **限制**：
   - Sailry 原生 `media` 通道（`internal/node/agent/model/media.go`）目前仅支持文生图；若在 `media` 通道传入 `references`（或 `count > 1`），工具会诚实返回 `InvalidRequest` 并提示使用 `cheaprouter` 或 `custom` 通道。

---

## 4. 遗留项与已知边界

1. **第三方自定义接口不支持 edits 时的表现**：
   - 当自定义接口返回 HTTP `404` / `405` / `501` 时，`Client.GenerateCustom` 会归类为 `ErrBadRequest`（Node 映射为 `InvalidRequest`）并附带明确说明（`custom image API does not support image editing (/v1/images/edits, HTTP ...)`），不会静默切换到 CheapRouter 通道。
   - 若部分第三方网关返回自定义 400/500 文案或仅接受 `image[]` 而非 `image` 字段名，原始脱敏错误会直接返回给 Agent 供提示用户排查。
2. **创作工作台 UI 参考图编辑**：
   - 本次按任务范围已补齐 `internal/imagestudio` 的 `GenerateEdits` 与共享 `Runner` 底座，并在 Agent `generate_image` 全链路启用；桌面创作工作台页面（`internal/desktop/imagestudio`）的 UI 参考图选择器未在本次范围中改动。
3. **跨进程 OAuth 并发刷新**：
   - 同进程内已由 `cheapauth.credentialLock` 串行化；若同时运行桌面端与外部独立 `sailry-host` 指向同一 profile 目录，跨进程文件级刷新锁仍属单 profile 单归属约定（与方案第 5 节一致）。
4. **CheapRouter / Custom 画图用量账本**：
   - CheapRouter 与自定义 Image API 通道按张计费（无 token 用量字段），未写入 Sailry 的 token `read_usage` 表；原生 `media` 通道保持原有 `recordMediaUsage` 统计不变。

---

## 5. 需真机 / 真实网关验证的点

以下项目未在自动化离线测试中覆盖，需在真实环境按需验收：

1. **真实 CheapRouter 网关生成与编辑联调**：
   - 线上 `/v1/images/generations/async` 与 `/v1/images/edits/async`（含单张与多张 `image[]` 参考图）的真实异步轮询与 `404` → SSE 保活降级表现。
   - 真实账号多分组场景下 `gpt-image-2` 与 `grok-imagine` 的 403/404 自动回退、24h Denial 缓存与工作台/Agent 共享 `RememberImageGroup`。
2. **真实第三方 Custom Image API 兼容性**：
   - 验证常见 OpenAI 兼容图片网关对 `POST /v1/images/edits` multipart `image` 字段、`ExtraParams` 与外链 `url` 下载的兼容性，以及不支持 edits 的网关返回的实际错误文案。
3. **桌面端与移动端展示**：
   - 使用 `--data-dir .runtime/profiles/<fixed-name>` 在桌面端验证 Agent 加载 `builtin:image-generation` → 调用 `generate_image`（生图与改图、单图与多图）→ 审批交互 → 气泡 Markdown 相对路径图片渲染与会话资产栏预览。
   - 若需确认移动端（Flutter `apps/mobile`）对桌面端 Agent 生成的多图会话资产展示与下载，需在 iOS/Android 模拟器或真机上单独验证。
