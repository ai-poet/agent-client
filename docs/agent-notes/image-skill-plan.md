# Agent 通过 Skill 调用画图接口方案评估

> 调研基线：仓库 `github.com/ai-poet/cheaprouter-harness`，分支 `sync/upstream-029`；历史 Rust 基线 `b2d93c15b^`。本轮仅做调研与方案设计，不改动代码。

---

## 1. 结论先行

**推荐形态：Skill + 薄内置工具（复用并扩展 Node 侧已有的 `generate_image` / `media.image` 工具，配一份随附 Skill 负责场景编排与调用规范）。**

**一句话理由**：密钥隔离（0600 私有凭证、OAuth 刷新与动态分组取 Key）、CheapRouter 异步任务轮询与多分组容错回退、工作树受限写盘与会话资产入库都属于 Node 的安全与状态边界，纯 Skill + 命令行/HTTP 无法安全可靠地承担；而「何时画图、怎么组织提示词与参数、多渠道选择策略、结果展示与错误引导」放在 Skill 中更易演进、对上游侵入也最小。

---

## 2. 现状盘点（附代码证据）

### 2.1 Rust 老实现（`b2d93c15b^`）

旧 Rust 客户端通过内置 MCP Server `waku_js_repl` 暴露 `generate_image`（对外工具名 `waku_js_repl_generate_image`），同时服务内置 Agent 与接入的外部 CLI：

| 维度 | 历史实现细节 | 文件:行证据（`b2d93c15b^`） |
| --- | --- | --- |
| 工具挂载与注入 | 挂在 `waku_js_repl` MCP Server 上；关闭 Computer Use 时通过 `WAKU_REPL_TOOLS=generate_image` 启动 image-only REPL；系统提示词注入简短指引，要求将生成结果用绝对路径 Markdown 图片展示 | `src/js_repl.rs:262-282,457`；`crates/agent-bridge/src/config.rs:349-369,553-572,1285-1292` |
| 配置与鉴权来源 | **不通过环境变量传 Key**（防止命令行泄露），而是从引擎 `settings.json` 读取 `/config/provider_configs/anthropic/api_base` 和 `options/gateway_keys`：优先取 `models[model]`（画图工作台探测记住的专属分组 Key），否则依次回退 `openai` → `default`，**绝不回退 `anthropic`**（网关按 Key 所属分组平台路由，Anthropic 组会 404）；支持测试覆盖环境变量 `WAKU_IMAGE_GATEWAY_ORIGIN/KEY`；未登录或指向自定义端点时 `resolve_gateway()` 返回 `None`，工具直接不下发 | `src/js_repl_image.rs:9-12,27-46,83-130,267-307` |
| 入参与出参 | 入参：`prompt`（必填）、`model`、`size`（`1024x1024`/`1536x1024`/`1024x1536`/`auto`）、`quality`、`background`、`output_format`（`png`/`jpeg`/`webp`）、`n`（1-4）、`output_dir`。底层调 `sub2api::images::generate_blocking`（有异步任务接口则 `/async` + 轮询 `/v1/images/tasks/:id`，否则 `stream: true` SSE 保活）。出参为 MCP `content`（摘要文本 + `image` base64 块，供人看）及 `structuredContent.paths` | `src/js_repl.rs:284-407`；`src/js_repl_image.rs:50-74,199-216`；`crates/sub2api/src/images.rs:9-27,39-58,89-135` |
| 产物落盘与图库关系 | 默认落到 `<WAKU_SESSION_CWD>/generated-images/<YYYYMMDD-HHMMSS>-<i>.<ext>`（相对 `output_dir` 也按会话目录解析）。**不进入创作工作台图库（Pictures 文件夹 / gallery index）**，两者仅共享 `sub2api::images` 传输与记住的分组路由 | `src/js_repl_image.rs:142-167,210-243` |
| 权限与错误分类 | `ComputerUseWiring` 对 `waku_js_repl_generate_image` 自动预批准（Plan 模式和显式 deny 规则仍生效）；错误区分 `Unauthorized` / `NoImagePermission`（提示先在 Image Studio 试画以发现可用分组） / `WrongPlatform` / `ContentPolicy` / `OwnBalance` | `crates/agent-bridge/src/config.rs:361-369`；`src/js_repl_image.rs:171-196` |

### 2.2 Go 侧现有底座（存在「两套割裂子系统」）

调研发现，Go/MyGo 版**并非没有画图工具**，而是同时存在**上游 Sailry 原生媒体工具**与**Fork 自建的创作工作台（`imagestudio` + `cheapauth`）**两套互不打通的体系：

#### A. `internal/imagestudio`（创作工作台底层：已具备完整传输与自定义接口能力）
- **双通道客户端** (`internal/imagestudio/client.go:47-114`)：
  - `GenerateCheapRouter` (`client.go:47-75,116-247`)：先尝试 `POST /v1/images/generations/async` + 3s 间隔轮询 `GET /v1/images/tasks/:id`（最长 30 分钟，`options.go:23-25`）；若返回 `ErrAsyncUnavailable` 或 `ErrWrongPlatform` 则自动回退同步接口（Grok 走 JSON `b64_json`，OpenAI 系逐张走 `stream: true` SSE 保活，16 分钟超时）。
  - `GenerateCustom` (`client.go:79-114`)：调用自定义 OpenAI 兼容端点 `POST /v1/images/generations`（兼容 `origin` / `.../v1` / 完整路径，`options.go:114-129`），支持 `ExtraParams` 额外 JSON 字段与 `b64_json` / `url` / `data:` URL 三种返回格式（`client.go:429-495`）。
  - 缺口：当前仅支持文生图（`images/generations`），**尚未移植 Rust 版的参考图编辑**（`/v1/images/edits` multipart）；`GenerateSpec` 仅含 `Channel`/`Model`/`Prompt`/`Size`/`Quality`/`Count`（`types.go:61-68`），未包含 `Background`/`OutputFormat`/`References`。
- **模型选项与落盘** (`internal/imagestudio/options.go:34-99`、`client.go:344-426`)：内置按模型族的尺寸/质量表、魔数嗅探扩展名（png/jpg/webp/gif）、原子 `.part` → 重命名写盘、下载 URL 时**不携带 API Key** 且限制 50 MiB；所有错误经 `sanitizeSecret` 脱敏并归类为 16 种 `ErrorKind`（`types.go:96-186`，含 `ShouldTryNextGroup()` 判断是否切换下一分组）。
- **本地存储** (`internal/imagestudio/store.go:16-249`)：
  - `CustomStore`：`<profileDir>/image-api.json`（0600 权限，含 `BaseURL`/`APIKey`/`Model`/`ExtraParams`，`String()` 自动脱敏 Key，`types.go:22-34`）。
  - `IndexStore`：`<profileDir>/image-studio/index.json`（0600 权限，含 `Prefs`、`Denials` 24h 分组拒绝缓存、`Jobs` 最多 200 条历史）。

#### B. `internal/protocol` 与 `internal/node` 的 `media.image`（上游内置工具已存在，但未接通 CheapRouter / Image API）
- **枚举与策略**：`ToolOperationGenerateImage = "media.image"`（`internal/protocol/tool.go:275`）、`PluginActionGenerateImage = "media.image"`（`internal/protocol/plugin_action.go:42`）。注意：`sdk/plugins.md:1007-1010` 与 `plugins/plugin-builder/skills/plugin-builder/references/manifest.md:137` 明确说明 `media.image` 是**核心内置工具操作**，外部插件声明它虽然通过校验但**不再绑定插件工具**（历史上独立的 `media` 插件已在 `internal/node/plugins/catalog.go:42` 列入 `RetiredBundled()`）。
- **Node 侧已有完整 `generate_image` Handler**：
  - Schema 定义在 `internal/node/agent/operations/media.json:50-71`（入参仅 `prompt` 和可选 `file_name`，固定要求 `.png`）。
  - 装配入口在 `internal/node/workspace_tools.go:260` → `internal/node/media_tools.go:83-102`：设为 `Deferred: true`（首次请求隐藏，需经 `tool_search` 发现），`Approval: true`（需用户审批，Plan 模式拒绝，`internal/node/store/workspace_approvals.go:23-25`）。
  - 执行逻辑在 `internal/node/media_tools.go:525-594` → `internal/node/agent/model/media.go:344-397`：通过受限文件服务写入工作树 `<worktree>/assets/generated/<name>.png`（拒绝覆盖同名文件），把 PNG 写入 Node SQLite Blob（`store.PutBlob`）作为展示附件，并自动接入会话资产清单（`internal/node/store/conversation_assets.go:292-312`）与对话 UI 去重渲染（`internal/desktop/conversation/tools.go:315-321`）。
- **为什么当前对 CheapRouter / 自定义 Image API 用户“完全用不上”**：
  1. `routeMedia`（`internal/node/media_tools.go:51-66`）只查 `inv.Media[MediaKindImage]`（设置 → 媒体模型）和当前会话模型，且要求模型在 `ConversationProvider.Models` 中带有 `Generates: ["image" | "image_tool"]`（`internal/node/agent/model/media.go:86-98`）。
  2. `model.GenerateImage`（`internal/node/agent/model/media.go:344-397`）只支持同步 JSON `POST /images/generations`、Responses hosted `image_generation` 和 Gemini `generateContent`，**不支持** CheapRouter 异步任务、SSE 保活、多分组回退与拒绝缓存。
  3. `internal/node` 完全不读 `image-api.json`，也未接入 `cheapauth` 的画图路由逻辑。

#### C. 设置里的两处配置与持久化（彼此独立）
| 设置入口 | 界面文件 | 持久化位置 | Agent（Node）当前能否读取 |
| --- | --- | --- | --- |
| 设置 → **媒体模型（Media）** | `internal/desktop/settings/media.go:21-57`；模型能力推断见 `internal/desktop/settings/providers_media.go:12-39` | Node SQLite `personal_settings["media"]`（`internal/node/store/media.go:29-54`），绑定到 `ConversationProvider` + `Model` | **能**（在 turn 准入时通过 `CaptureMedia` 冻结到 `inv.Media`） |
| 设置 → **Image API（自定义画图接口）** | `internal/desktop/imagestudio/settings_view.go:14-109`；注册在 `internal/desktop/settings/settings.go:28,139-140` | `<profileDir>/image-api.json`（0600 文件，`internal/imagestudio/store.go:17,30-35`） | **当前未读**，但 `<profileDir>` 与 Node 的 `node.profile.Ownership.Path` 是**同一个目录**（`internal/desktop/live/runtime.go:259`、`imagestudio.go:40`、`internal/node/bootstrap.go:23-39`），Node 可直接访问 |

#### D. CheapRouter 登录后画图模型的识别与自动配置
- **识别逻辑**（`internal/cheapauth/images.go:68-178`）：`IsImageCatalogModel` 综合 `billing_mode == "image"`、`pricing_details.media_unit == "image"`、`effective_pricing.per_image_usd != nil` 及模型名前缀规则（`gpt-image-*`、`grok-imagine*`、`dall-e-*`、`imagen-*`、`flux-*`、`sdxl*` 等），排除视频/音频/向量模型。
- **分组路由与取 Key**（`internal/cheapauth/images.go:184-306`、`internal/cheapauth/service.go:1008-1082`）：
  - `ImageRouteCandidates` 按优先级排序候选分组：① `credentials.ImageGroups[model]`（上次画成功记住的分组）→ ② 平台对应的活跃车道分组（OpenAI→`codex`，Grok→`grok`）→ ③ 目录中的本平台分组（按倍率从低到高）→ ④ 复合分组 → ⑤ 账户默认 `GroupID`。
  - `ImageRouteKey(ctx, groupID)` 复用/自动签发对应分组的网关 API Key，并返回当前生效的业务网关 Origin（尊重 `gateway-origin.json` 节点切换）。
  - `RememberImageGroup(ctx, model, groupID)` 在成功出图后把分组 ID 持久化到 `<profileDir>/cheaprouter-account.json`（`internal/cheapauth/types.go:20,105`）。
- **关键断层**：登录同步后，`internal/desktop/live/cheapauth.go:462` 调用 `providers.CheapRouterProviders(gateway)` 写入 Node Provider，而 **`internal/node/providers/cheaprouter.go:167-173` 显式跳过了所有 `cheapauth.IsImageCatalogModel(item)` 和 `billing_mode != "token"` 的模型**；画图目录只同步给了 Desktop 创作工作台（`internal/desktop/live/imagestudio.go:142-172`），导致 Agent 侧完全感知不到登录后的画图能力。

### 2.3 Skill 机制调研

| 环节 | 现状实现 | 文件:行证据 |
| --- | --- | --- |
| 组织与分发 | `bundled.go:8-9` 用 `//go:embed plugins` 嵌入插件目录；`internal/node/plugins/catalog.go:29-37` 的 `BundledDefaults()` 列出 24 个默认随附插件（单测 `packages_test.go:276` 强断言长度为 24）；启动时由 `initializeUserPlugins` 安装/恢复（`internal/node/plugin_service.go:110-182`）。每个插件可在 `plugins/<pkg>/skills/<skill>/SKILL.md` 下提供最多 64 个 skill（单文件 ≤128 KiB，YAML frontmatter ≤16 KiB，须含 `name`/`description`）。此外支持用户通过 Git / `skills.sh` / Skillhub / 本地目录安装**独立 Skill 包**（`internal/node/plugins/catalog_skills.go:68-300`、`internal/node/agent_tools_skills.go:17-300`）。**注意：当前仓库根目录下没有独立的 `skills/` 嵌入目录；且 `plugins/` 遵循上游独立仓库与 `catalog.json` 校验规则（`plugins/AGENTS.md`）。** | `bundled.go:5-9`；`internal/node/plugins/catalog.go:29-37`；`internal/node/plugins/skills.go:14-120`；`internal/node/agent/operations/resources.go:261-382` |
| 发现与加载 | 回合准入时捕获本轮启用的插件包快照 → `operations.CaptureResources`（`resources.go:48-86`）→ `Resources.Catalog()`（`resources.go:132-146`）拼出 `Available skills (package:skill)` 列表 → 作为 `Key: "skills"` 注入模型上下文（`internal/node/agent/engine/inference.go:72`）。引擎自动向模型提供两个内置只读工具：`load_skill`（返回 `SKILL.md` 全文、`directory` 落盘路径、以及受管 `python` 解释器路径）和 `read_skill_resource`（读取同技能目录下 UTF-8 相对路径文本）。 | `internal/node/agent/engine/skills_tools.go:16-114`；`internal/node/agent/operations/resources.go:132-214` |
| 可携带资源与安全边界 | Skill 目录可携带参考文档（`references/*.md` 等，用 `read_skill_resource` 读取）和脚本（如 `scripts/*.py`，配合插件根 `requirements.txt` 由受管 `$SAILRY_PYTHON` 通过普通 `run_command` 工具执行）。**关键约束**：① `resources.go:145` 和 `skills_tools.go:73` 明文规定 **Skill 不授予任何工具/命令权限**；② Skill 包内容通过不可变内容摘要校验，无法动态注入凭据或运行时状态；③ `read_skill_resource` 禁止二进制和越界路径，脚本执行走 `run_command` 仍受沙箱与用户审批约束。 | `internal/node/agent/operations/resources.go:98-130,145,162-214`；`internal/node/agent/engine/skills_tools.go:73-109`；`plugins/plugin-builder/skills/plugin-builder/references/manifest.md:163` |

### 2.4 工具 vs Skill 的边界分析

如果采用**「纯 Skill（提示词 + 让 Agent 用 `run_command` 跑 curl/Python 直接请求画图 HTTP 接口）」**，会撞上 6 个工程硬障碍：

1. **密钥安全与 OAuth 生命周期无法满足**：
   - `cheaprouter-account.json`（含 OAuth `access_token` / `refresh_token`）和 `image-api.json`（含自定义 `api_key`）均为 0600 私有凭据文件；按仓库安全红线（`AGENTS.md` 数据保护 & `imagestudio/types.go:3-4`），**绝不能把 Key 写进 Skill / 提示词**，也不应引导 Agent 通过 shell 读取 `~/.sailry*/cheaprouter-account.json`（这会让明文 Token 直接进入工具输出、会话历史和模型上下文）。
   - CheapRouter 的画图 Key 并不是静态字符串：过期需要用登录端点刷新 OAuth Token、还要调 `/api/v1/keys` 复用或按分组签发 Key、并遵循 `gateway-origin.json` 节点切换；这些在进程外脚本里会与 Desktop 的 `credentialLock`（`internal/cheapauth/service.go:28-33`）产生 refresh token 轮换竞争，直接把用户踢下线。
2. **异步任务与分组回退复杂度过高**：CheapRouter 画图需要 `ImageRouteCandidates` 多分组试探 + 24h `Denial` 缓存 + `RememberImageGroup` 回写 + `/async` 提交轮询（长达 30 分钟）与 SSE 保活降级；把这套状态机塞进 Skill 脚本不仅与 `internal/imagestudio` 双份维护，还会在命令超时/中断时丢失状态。
3. **命令权限与沙箱摩擦极大**：若开启工作区沙箱，出网 HTTP 请求或读 profile 目录需要 `unrestricted` 命令权限，按 `internal/node/store/workspace_approvals.go:37-53` 每次都弹窗且不能“记住规则”；关闭 `commands` 插件或移动端/无 shell 场景下直接不可用。
4. **会话资产与对话内预览断链**：只有 Node 侧 `generate_image` 返回的 `engine.ToolResult` 携带 Blob `Attachments` 并满足 `generatedImageAssetFile`（`internal/node/store/conversation_assets.go:292-312`），才能自动进入会话资产清单、气泡预览与移动端渲染；shell 脚本写的文件只是普通工作树文件。
5. **模型上下文膨胀与泄露风险**：若脚本把原始 HTTP 报错打印到 stdout，缺少 `sanitizeSecret` 容易把 Bearer Key 打进对话日志；若打印 base64 则直接撑爆上下文。
6. **平台一致性差**：Windows/macOS/Linux 的 `curl`/Python 环境与代理配置（`netproxy.Transport()`）差异大，而 Go 侧 `imagestudio.Client` 已经一次封装到位。

反之，如果采用**「纯内置工具、没有 Skill」**：
- 工具描述（JSON Schema description）不宜塞入大段提示词工程、模型/尺寸选型建议、画图失败时的多渠道引导话术和回复展示规范，否则会污染所有会话的基础上下文，且调优提示词必须改 Go 代码/重编译。
- 现有 `generate_image` 默认 `Deferred: true`（需要 `tool_search` 加载），缺少 Skill 引导时模型容易忘记先检索工具或传错参数。

因此，**「薄内置工具（管执行、鉴权、路由、落盘、资产） + Skill（管触发判断、Prompt/参数策略、多渠道说明、回复呈现）」** 是权责最清晰的解。

---

## 3. 推荐方案

### 3.1 配置来源与路由优先级

统一在 Node 侧解析「可用画图通道」，让 **CheapRouter 登录后自动配置的画图接口**、**设置里的自定义 Image API**、以及 **Sailry 原生 Media models（Image）** 协同工作而非互相遮蔽。

**渠道与模型解析优先级（由高到低）**：

1. **调用方显式指定（本轮用户要求 → Agent 传参）**
   - 若工具入参显式传了 `channel`（`cheaprouter` / `custom` / `media`）或显式指定了某个渠道的 `model`（例如用户说“用自定义接口画”或“用 `grok-imagine` 画”），直接命中对应渠道；若该渠道未配置/未登录，**诚实报错，不静默切到别的付费通道**。
2. **设置里显式指定的“媒体模型 → Image”（Sailry 原生 `personal_settings["media"]`）**
   - 仅当用户在「设置 → 媒体模型」中**明确选择了某个 Image 模型**（而非默认的“跟随对话模型”）时生效，保持与上游 Sailry 原生行为 100% 兼容（`internal/node/media_tools.go:53-55`）。
3. **CheapRouter 登录后自动配置的网关画图接口（核心默认路径）**
   - 条件：已登录 CheapRouter（`<profileDir>/cheaprouter-account.json` 有效）且（目录中存在 `cheapauth.ImageCatalogModels` 或使用默认模型 `gpt-image-2`）。
   - 若创作工作台 `<profileDir>/image-studio/index.json` 的 `Prefs.Channel == "custom"` 且 `image-api.json` 已配置，可选尊重用户的创作工作台默认渠道偏好（需在方案评审时二选一确认：**固定 CheapRouter 优先** vs **跟随工作台 `Prefs.Channel`**；推荐**固定「CheapRouter 登录态 → 自定义 Image API」**为主，若工作台显式切到 `custom` 则作为默认 `channel` 提示，避免用户在工作台切过一次自定义后忘了切回导致 Agent 走错计费）。
4. **设置 → Image API 自定义画图接口（`<profileDir>/image-api.json`）**
   - 当未登录 CheapRouter（或 CheapRouter 账号无任何可用画图分组/模型）且 `CustomConfig.Configured() == true` 时自动作为默认画图通道。
5. **当前对话模型原生生图能力（上游兜底）**
   - 上述 2-4 均不可用时，回退到上游 `routeMedia` 原有逻辑：若当前会话模型本身支持 `image` / `image_tool`（如 Responses 托管生图或 Gemini 生图），仍提供 `generate_image` 工具。
6. **全部不可用**
   - 工具不在本轮下发（避免“给了工具却注定失败”，与旧 Rust `resolve_gateway().is_none()` 和上游 `mediaTools` 的 `if !offered { continue }` 原则一致）；Skill 引导 Agent 如实告知用户去「CheapRouter 账号登录」或「设置 → Image API / 媒体模型」配置。

> **关键工程点（避免 OAuth 并发刷新冲突）**：
> Node 与 Desktop 在桌面端处于同一进程内。`internal/cheapauth/service.go:33` 已用包级 `credentialLock` 保护同进程并发刷新，但仍建议将 Desktop 与 Node 指向同一个 `cheapauth.Service`（或在 Node 启动选项中注入 `ImageBackend` / 共享 `*cheapauth.Service`），同时兼容独立 `sailry-host` 进程按 `node.profile.Ownership.Path` 延迟构造自己的 `cheapauth.Service` + `imagestudio.CustomStore` + `imagestudio.IndexStore`。

### 3.2 Skill 的形态与落点

#### 放置位置与分发方式（需尊重 `plugins/` 仓库边界）
有两种承载方式，推荐 **方式 A**：

- **方式 A（推荐）：应用内建轻 Skill（Go `//go:embed`），不污染上游 `plugins/` 子模块**
  - 在应用仓库自有目录放置，例如 `internal/node/agent/operations/skills/image-generation/SKILL.md`（通过 `//go:embed` 编进二进制）。
  - 在 `operations.CaptureResources`（`internal/node/agent/operations/resources.go:48-86`）中将其作为核心内建 Skill（例如 key 为 `sailry:image-generation` 或 `builtin:image-generation`）注入本轮资源目录——**仅当本轮 `generate_image` 工具可用时，或始终注入并在内容中说明未配置时的引导话术**（推荐始终注入但控制精简在 ~2 KB，这样用户未配置时 Agent 也能准确指引去哪配置，而不是胡编外网链接）。
  - **优势**：完全不改动 `plugins/` 子模块和 `plugins/catalog.json`，不触发 `TestBundledPackageContracts` 中 `len(BundledDefaults()) == 24`（`internal/node/plugins/packages_test.go:276`）的破坏，后续同步上游 `sailry` 与 `sailry-plugins` 零冲突。
- **方式 B（备选）：作为独立插件包放入 `plugins/image-generation/`**
  - 只有在需要把该 Skill 上架官方/插件市场、允许用户单独卸载开关、且准备同步维护 `sailry-plugins` 独立仓库与 `catalog.json` 时才考虑；对当前 fork 来说维护成本过高。

#### Skill 暴露给 Agent 的内容（不需要额外脚本，纯提示词 + 可选参考文档）
- **无需携带 Python/Shell 脚本**（执行全交给薄工具，零外部运行时依赖、零密钥暴露）。
- `SKILL.md` 内容保持精炼（建议 80–150 行），覆盖 5 块核心知识：
  1. **能力发现与调用入口**：明确先通过 `tool_search`（`select:generate_image`）加载工具（若尚未在初始工具列表中），不要伪造 SVG 或外链图代替真实图片。
  2. **渠道与模型选择指引**：说明可用渠道（默认使用已登录的 CheapRouter 网关画图模型或设置中的自定义 Image API，用户明确点名某个模型或“用自定义接口”时再传 `channel`/`model`；不要猜测或编造未列出的模型名，可先读工具返回的当前可用渠道摘要或省略参数走默认）。
  3. **Prompt 与参数规范**：英文/中英提示词组织要点（主体、构图、光线、风格、约束）；`size`（方图 `1024x1024`、横图 `1536x1024`、竖图 `1024x1536`、高清 `2048x2048` 等）、`quality`（Grok 忽略 quality）、`count`（默认 1，最多 4）、`file_name`（有意义的英文文件名）。
  4. **结果交付方式**：拿到工具返回的相对路径 `path`（或 `paths`）后，在最终回复中用 Markdown 图片语法 `![简短说明](assets/generated/...)` 嵌入展示，并简述实际使用的模型/路径（复用 `internal/desktop/conversation/tools.go:315` 的设计：工具行折叠预览、由回复正文和会话资产栏呈现）。
  5. **异常处理与防重复扣费**：遇到 `unauthorized` / `no_image_permission` / `own_balance` / `content_policy` / `not_configured` 时如何给用户清晰指引；遇到 `outcome_unknown` 或超时不得静默重试以免重复扣费。

> **关于「让 Agent 知道当前有哪些可用画图渠道/模型」**：
> 不在 `SKILL.md` 里写死模型列表，而是把**脱敏后的当前可用画图摘要**（仅包含渠道名、默认模型、可用模型 ID 列表、自定义接口 Host——**绝不含任何 Key/Token/完整私有 URL 凭证**）动态拼入 `generate_image` 的工具描述或 `Deferred` 之外的轻量动态上下文中（与 `internal/node/agent/engine/inference.go:68-75` 的 `parts` 机制一致）。

### 3.3 薄工具设计（复用并扩展现有 `generate_image`）

**不新增第二套画图工具名**，直接复用并向后兼容扩展现有的内置 `generate_image`（`operation: "media.image"`），避免出现两套画图工具互相打架。

#### 1. 入参（向后兼容现有 `media.json:54-68`）
| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `prompt` | `string` | 是 | 画面描述（≤32000 字节，非空） |
| `file_name` | `string` | 否 | 纯文件名（禁止路径分隔符），单图时沿用原规则；多图（`count > 1`）时自动加 `-1`/`-2` 后缀，扩展名根据实际输出或统一转为 `.png`（建议延续现有 `.png` 约定或按嗅探扩展名白名单 `.png`/`.jpg`/`.webp`） |
| `channel` | `enum: "auto" \| "cheaprouter" \| "custom" \| "media"` | 否 | 默认 `"auto"`，按 3.1 优先级解析 |
| `model` | `string` | 否 | 可选覆盖模型 ID（留空则走对应渠道默认模型，如 CheapRouter 的 `gpt-image-2` 或 `image-api.json` 的 `Model`） |
| `size` | `string` | 否 | 如 `"1024x1024"`、`"1536x1024"`、`"1024x1536"`、`"2048x2048"`、`"auto"` |
| `quality` | `enum: "auto" \| "low" \| "medium" \| "high" \| "standard" \| "hd"` | 否 | 默认 `"auto"`；Grok 模型自动忽略 |
| `count` | `integer` (`1..4`) | 否 | 生成张数，默认 `1` |
| *(二期预留)* `references` | `[]string` | 否 | 工作树内相对图片路径或会话附件 ID，用于 `/v1/images/edits` 图生图/改图（待 `internal/imagestudio` 补齐 multipart edit 后启用） |

> 现有单测只传 `{prompt, file_name}`，上述新增字段全部可选，**100% 向后兼容** `internal/node/media_tools_test.go`。

#### 2. 出参（向后兼容现有结果结构与会话资产协议）
```json
{
  "path": "assets/generated/sunset-harbor.png",
  "paths": ["assets/generated/sunset-harbor.png"],
  "media_type": "image/png",
  "size": 184320,
  "channel": "cheaprouter",
  "provider": "CheapRouter",
  "model": "gpt-image-2",
  "revised_prompt": "A warm sunset over a quiet harbor...",
  "task_id": "imgtask_xxx"
}
```
- 单图时保留顶层 `path` / `media_type` / `size` 并在 `engine.ToolResult.Attachments` 附带 1 个 Blob 附件，完全命中 `generatedImageAssetFile`（`internal/node/store/conversation_assets.go:292-312`），无需破坏已有会话资产逻辑。
- 多图（`count > 1`）时返回 `paths` 数组及对应多个 `Attachments`，同步放宽 `generatedImageAssetFile` 支持多图登记。
- **绝不把 base64 字节塞回 JSON 文本**（沿用 `media_tools_test.go:490-492` 的原则，防止污染模型上下文）。

#### 3. 权限与审批策略
- 继续使用 `protocol.ToolOperationGenerateImage`（`"media.image"`），`ReadOnly: false`、`Concurrent: false`、`Approval: true`（`internal/node/media_tools.go:199-201`）。
- 审批走现有 `store.ConfigureWorkspaceApproval`（`internal/node/store/workspace_approvals.go:13-80`）：
  - **Plan 模式**：自动拒绝（`ErrorCodePermissionDenied`）。
  - **Ask / Project 模式**：首次调用弹出审批，展示 prompt、目标路径与渠道/模型；用户可选择「本次允许」或「本会话记住规则」（基于 `ApprovalBinding{Tool: "generate_image", Operation: "media.image"}`）。
  - **Full 模式**：按现有权限级别自动放行。
- 写盘继续走 `core.OpenFiles(inv.Worktree.Path, core.RestrictedFiles)`（`internal/node/media_tools.go:215,466`），拒绝路径穿越与覆盖已有文件。

#### 4. 异步任务、多分组回退与取消处理
- CheapRouter 通道直接复用 `internal/imagestudio.Client.GenerateCheapRouter`（异步提交 + 3s 轮询 + 自动降级 SSE/JSON）与 `cheapauth.Service.ImageRouteCandidates` / `ImageRouteKey` / `RememberImageGroup` + `imagestudio.IndexStore` 的 24h `Denial` 读写，**抽出一份共享的编排函数（放到 `internal/imagestudio`）供 Desktop 创作工作台和 Node `generate_image` 共同调用**，彻底消除逻辑重复。
- 若网关已受理异步任务（拿到 `task_id`）但在轮询中途因网络断开/本地超时未拿到结果：返回 `protocol.ErrorCodeOutcomeUnknown` 并在错误信息中带回 `task_id`（参考现有 `generate_video` 的 `videoJobFault` 模式，`internal/node/media_tools.go:507-519`），防止 Agent 重复提交扣费。
- 严格传递工具执行的 `ctx`，用户点击停止会话时立即中止 HTTP/轮询并返回 `ErrorCodeCancelled`。

#### 5. 失败与鉴权错误回传给 Agent 的映射
所有底层错误必须先经 `imagestudio.ClassifyError` / `sanitizeSecret` 脱敏，再映射为标准 `protocol.Fault`（供模型和 UI 清晰区分可恢复/不可恢复动作）：

| `imagestudio.ErrorKind` | `protocol.ErrorCode` | 回给 Agent 的可行动文案方向 |
| --- | --- | --- |
| 未配置任何画图通道 | `NotConfigured` | 提示引导用户登录 CheapRouter 或在「设置 → Image API / 媒体模型」配置 |
| `ErrUnauthorized`（登录失效 / Key 无效） | `AuthenticationFailed`（或现有 `model.Authentication` 映射） | 提示登录已过期或自定义 API Key 无效，需重新登录/检查密钥 |
| `ErrNoImagePermission` / `ErrWrongPlatform`（所有候选分组均拒绝） | `PermissionDenied` | 说明当前账户分组无画图权限或不支持 Images 接口，建议切换分组或配置自定义 Image API |
| `ErrOwnBalance` / `ErrUpstreamBalance` | `Unavailable` / `QuotaExceeded`（按现有 Fault 枚举） | 明确提示余额不足，建议充值或切换自定义接口 |
| `ErrContentPolicy` | `InvalidRequest` | 说明被内容安全策略拦截，建议调整提示词后重试 |
| `ErrBusy` / `ErrNetwork` / `ErrTimeout` | `Unavailable` / `OutcomeUnknown` | 区分“未提交可重试”与“已生成 task_id 不可盲目重试” |
| `ErrCancelled` | `Cancelled` | 用户主动取消 |

### 3.4 与创作工作台 / 图片库 / 会话资产的关系

| 维度 | 创作工作台（Image Studio） | Agent `generate_image` | 建议关系 |
| --- | --- | --- | --- |
| 主存储路径 | `~/Pictures/CheapRouter/<YYYY-MM>/<stem>-<i>.<ext>` | `<worktree>/assets/generated/<name>.png` + Node SQLite Blob | **各自保持主路径，不强行合并物理目录**：工作树相对路径是会话资产、Markdown 嵌入、跨端/Mobile 同步与 `RestrictedFiles` 安全边界的前提；用户 Pictures 文件夹面向人工手动创作 |
| 底层引擎与配置 | `internal/imagestudio` + `cheapauth` + `image-api.json` | 复用同一套 `internal/imagestudio` 编排器与配置源 | **100% 共享** |
| 分组记忆与拒绝缓存 | 读写 `cheaprouter-account.json`（`ImageGroups`）与 `image-studio/index.json`（`Denials`） | 读写同一份 `ImageGroups` 与 `Denials` | **共享路由记忆**（在工作台画通的分组 Agent 立即可用，反之亦然） |
| 创作工作台图库（`index.json` `Jobs`） | 记录工作台历史 | 默认**不写入**工作台图库，避免会话中的多次试画/草图污染用户全局图库；若后续有明确产品诉求，可在 `index.json` 中用来源标签（`source: "agent"`）或“保存到图库”按钮按需接入 | **默认解耦，保留扩展位** |

---

## 4. 改动面清单与粗略工作量

> 以下仅列出预计涉及的目录/包，不修改代码。

| 目录 / 包 | 预计改动内容 | 粗略工作量 |
| --- | --- | --- |
| `internal/imagestudio` | 抽出共享的高层编排器（例如 `Runner` / `Service`：封装 CheapRouter 分组试探 + Denial 读写 + RememberImageGroup + Custom 调用），供 Desktop 与 Node 共用；补充单测 | **中** |
| `internal/desktop/live/imagestudio.go` | 将 `runCheapRouterImageJob` / `runCustomImageJob` 中的纯路由与调用逻辑改为委托 `internal/imagestudio` 共享编排器，避免与 Node 重复维护；与 Node 共享同一 `*cheapauth.Service`（若桌面端直连本地 Node） | **小** |
| `internal/node/agent/operations` | ① 扩展 `media.json` 中 `generate_image` 的可选参数 Schema（`channel`/`model`/`size`/`quality`/`count`）；② 嵌入内建 Skill `skills/image-generation/SKILL.md`，在 `resources.go` 中支持内建 Skill 发现与 `load_skill` 读取 | **小–中** |
| `internal/node/media_tools.go` & `internal/node/node.go` | ① 扩展 `routeMedia` / `mediaTools`，接入 CheapRouter 登录态与 `image-api.json` 解析（按 3.1 优先级）；② 扩展 `generateMedia` 支持调用 `imagestudio` 编排器、多图落盘、错误码映射与用量/结果摘要；③ 将脱敏后的可用渠道/模型摘要注入工具描述或动态上下文 | **中** |
| `internal/node/store/conversation_assets.go` | 放宽 `generatedImageAssetFile` 支持多图（`count > 1`）登记到会话资产清单 | **小** |
| `internal/node/agent/engine`（可选微调） | 若需支持内建 Skill（非插件包来源）在 `skillTools` / `CaptureResources` 正常通过 `load_skill` 校验与 `directory` 处理，补少量兼容逻辑 | **小** |
| `internal/desktop/i18n` & `locales/`（按需） | 若新增错误提示或内建 Skill 展示名（中英本地化），按规范更新 `keys.txt` 与 `locales/` | **小** |
| `ARCHITECTURE.md`（按需） | 同步记录 Agent 画图工具与 `imagestudio` / 内建 Skill 的所有权关系 | **小** |

**总体工作量评估：中等（Medium）**——核心 HTTP 客户端、鉴权、分组路由、自定义 API 存储、内置 `generate_image` 工具壳子、审批与会话资产链条都**已经存在**，主要工作是「打通断层 + 抽取共享编排 + 补充内建 Skill 与完整测试」。

---

## 5. 风险与取舍

1. **维护成本：薄工具 + Skill vs 纯写死工具 vs 纯 Skill**
   - *薄工具 + Skill*：参数协议、密钥与状态机集中在 Go 侧一处维护（并与创作工作台复用），Prompt 策略、选模型建议和回复规范在 `SKILL.md` 独立演进；后续扩图生图（`edits`）只需给薄工具加 `references` 参数并在 `SKILL.md` 加一节说明。
   - *纯 Skill*：短期看似“不用改 Go 代码”，但实际要绕过密钥保护、OAuth 刷新锁、沙箱审批和会话资产协议，最终维护成本和出故障率远高于薄工具。
2. **密钥安全（红线）**
   - 严禁把任何 `api_key`、`access_token`、`refresh_token` 或带认证信息的 URL 写入 `SKILL.md`、系统提示词、工具 Schema 或工具返回文本。
   - 自定义接口的下载 URL 请求必须保持“不附带 Authorization 头”（`internal/imagestudio/client.go:343,385-389` 已满足）；所有 HTTP 报错文本必须经 `sanitizeSecret` 清洗后再返回给模型。
3. **OAuth Token 轮换与多进程/多实例竞争**
   - `cheapauth.Service` 的 `credentialLock` 是**进程内互斥锁**（`internal/cheapauth/service.go:33`）。在桌面端同进程内应复用同一 `Service` 实例；若同时运行外部 `sailry-host` 进程与 Desktop 进程访问同一 profile，刷新 Token 仍有文件级竞争风险（可在 `cheapauth.Store` 评估文件锁，或明确单 profile 单归属约定）。
4. **与上游同步的合并冲突控制**
   - 保持 `plugins/` 子模块不变、不改 `BundledDefaults()` 数量、不引入新的 `protocol.ToolOperation` 枚举，仅向后兼容扩展 `media.json` 与 `media_tools.go`，可以把未来同步上游 `sailry` 的冲突压到最低。
5. **离线与自定义接口差异**
   - 第三方 OpenAI 兼容图片接口差异较大（有的不认 `quality`/`size: "auto"`，有的只返回外链 `url`，有的需要自定义字段如 `aspect_ratio`）。`internal/imagestudio` 的 `ExtraParams` 已经承接了这部分差异，但需要注意：当用户切到 `custom` 通道且未传 `size`/`quality` 时，尽量保持最小载荷，避免第三方网关因未知字段报 400。
   - 离线状态下工具调用会触发网络错误，需返回清晰 `Unavailable` 错误而非长时间卡死；若既未登录又无自定义接口，工具干脆不下发。
6. **未覆盖范围（明确边界，避免蔓延）**
   - 当前 `internal/imagestudio/client.go` 尚未实现 `/v1/images/edits`（参考图编辑），建议**一期聚焦文生图**（与现有创作工作台对齐），二期再同时给工作台和 `generate_image` 补齐 `edits`。
   - 当前方案聚焦桌面端/Node 内置 Agent，不额外引入旧 Rust 式的外部 CLI MCP 注入。
7. **可测试性**
   - 薄工具 + 共享 `imagestudio` 编排器全链条可由 Go `httptest.Server` 确定性测试（覆盖同步 JSON、SSE 保活、异步提交+轮询、多分组 403/404 回退、Denial 过期、Custom API、`b64_json` 与 `url` 下载、密钥脱敏、取消与 `OutcomeUnknown`），不依赖真实模型或真实外网。

---

## 6. 验证方式

### 6.1 自动化单元测试与集成测试（确定性 Fixture，无外网依赖）
1. **编排与路由单测（`internal/imagestudio`）**：
   - CheapRouter 异步任务成功（`202` → `processing` → `completed`）与 `404 async unavailable` 自动降级 SSE 保活。
   - 多分组回退：分组 A 返回 403 `no_image_permission` / 404 `wrong_platform` → 自动记录 Denial 并切到分组 B 成功 → 回写 `RememberImageGroup`；24h 内再次调用自动跳过分组 A。
   - Custom API：兼容 `origin` / `.../v1` / 完整路径、`ExtraParams` 合并、`b64_json` 与 `url` 下载（校验下载不带 `Authorization` 头）。
   - 密钥脱敏：上游错误体或网络错误中包含 API Key 时，返回给调用方的错误字符串中绝不出现明文 Key。
2. **Node 工具与 Skill 集成测试（`internal/node/media_tools_test.go` & `operations` 测试）**：
   - **渠道优先级测试**：验证「显式 `channel`/`model` → MediaSettings 显式分配 → CheapRouter 登录态 → `image-api.json` → 对话模型原生能力 → 全无时不下发工具」的完整矩阵。
   - **内建 Skill 测试**：验证 `load_skill` 可加载 `image-generation` 的 `SKILL.md`，且 `plugins/packages_test.go` 原有 24 个随附包契约测试不受影响。
   - **审批与安全边界测试**：Plan 模式拒绝调用；Ask 模式生成 `media.image` 审批并支持记住规则；非法 `file_name`（路径穿越、已存在文件）在发请求前被拒绝；`ctx` 取消中止轮询。
   - **产物与会话资产测试**：单图与多图写入 `<worktree>/assets/generated/`、Blob 入库、`conversation_assets` 登记、结果 JSON 不含 base64 原始字节；保留现有 `media_tools_test.go` 全部用例回归通过。

### 6.2 端到端手动验收步骤（使用 `--data-dir .runtime/profiles/<fixed-name>`）
1. **场景 A（CheapRouter 登录零配置开箱即用）**：
   - 登录 CheapRouter 账号，不配置任何自定义 Image API 或媒体模型。
   - 新建对话让 Agent “画一张极简风格的海上日出插画，16:9 横图”。
   - 确认 Agent 加载画图 Skill → 经 `tool_search` 加载并调用 `generate_image`（带 `size: "1536x1024"`）→ 审批通过后生成图片并落盘至 `assets/generated/` → 气泡正文以 Markdown 图片正常渲染，会话资产栏可见，且在对话日志/工具详情中无任何 Key 泄露。
2. **场景 B（设置里的自定义 Image API）**：
   - 退出 CheapRouter 登录（或在对话中显式要求“用自定义画图接口”），在「设置 → Image API」填入自定义 Base URL / Key / Model。
   - 再次让 Agent 画图，确认请求发往自定义端点并正常落盘展示。
3. **场景 C（无任何可用配置）**：
   - 未登录且未配置自定义 Image API 与媒体模型，让 Agent 画图。
   - 确认工具未暴露，Agent 按 Skill 指引如实说明未配置并给出配置入口，不伪造图片或跑 shell 乱试。
4. **场景 D（创作工作台联动）**：
   - 在创作工作台先画成功一个需要特定分组的模型，随后在对话中让 Agent 使用同一模型画图，确认直接命中已记住分组无需重复试错。

### 6.3 需要真实环境 / 真机验证的部分
- **真实 CheapRouter 网关联调**（仅在显式要求真实联调时执行）：验证线上 `/v1/images/generations/async` 轮询与 SSE 保活在真实代理超时窗口下的表现，以及 Grok 与 `gpt-image-*` 模型的分组权限回退。
- **跨端渲染验证**：桌面端（macOS/Windows/Linux）验证 Markdown 相对路径图片渲染与大文件缩略图；若需确认移动端（Flutter `apps/mobile`）查看桌面端 Agent 生成的多图会话资产，需在对应 iOS/Android 真机或模拟器上单独验收。
