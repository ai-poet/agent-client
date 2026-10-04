# DeepSeek Harness 参考

[English](deepseek-harness.md) | 中文

本文记录 DeepSeek Harness（`dsh`）的插件如何嵌入它的界面和 agent 逻辑，以及这些接缝在我们客户端里各自对应哪里。
往 Waku 移植 dsh 功能时使用。我们**移植** dsh 的功能，从不加载 dsh 插件（见 §1）。

路径约定：dsh 路径相对于客户端根目录下被 git 忽略的检出 `deepseek-harness-master/`；
Waku 路径相对于客户端根目录。两者在 2026-10-04 均已核对存在。

## 0. 它是什么、怎么读

- 上游：`deepseek-ai/deepseek-harness`，所有包版本 `0.2.1-alpha.1`，MIT（`LICENSE`）。
  TypeScript monorepo，约 319 个包位于 `packages/<group>/<name>`，应用位于 `apps/`
  （`cli` = `dsh` 可执行文件、`web`、`desktop` = Electron、`desktop-host`）。Node `^22.19.0 || >=24.0.0`。
- 基于 Cordis 的“一切皆插件”，Cordis 以源码形式 vendored 在 `vendor/cordis`
  （上游 `cordis` 4.0.0-rc.7，重新发布为 `@deepseek-ai/cordis`，本地改动列在 `vendor/README.md`）。
- 读英文 `.md` 文件，跳过 `.zh.md` 和 `.i18n.yaml`。
- 入门顺序：`docs/architecture.md`、`docs/cordis-primer.md`、`docs/user/develop/`（插件编写教程）、
  `docs/cookbook/extension-cookbook.md`，然后是对应的 `docs/subsystems/<name>.md` 和包的 `README.md`。
- 生成的文档比散文更权威：`docs/event-producer-consumer.md`（每个事件的生产者和消费者）、
  `docs/capability-seams.md`（每个 `ctx` 键）、`docs/tool-catalog.md`、`docs/persistence-catalog.md`。
  UI 槽位的权威列表是 `packages/extensions/cordis-client-runner/src/client/slot-catalog.ts`；
  `docs/subsystems/slots.md` 少了约 16 个键。
- 项目自称开发者预览版，会有破坏性变更。移植某个机制前先回源码核对，本文可能过时。

## 1. 只移植，不加载

- dsh 插件是运行在 dsh Node 进程**内部**的 ESM 模块。它拿到一个活的 Cordis `Context`，并与之交换活对象：
  `Agent` 句柄、waterfall 里的 `next()` 闭包、`AbortSignal`、`Symbol` 令牌、以函数为值的工具定义。
  没有进程外插件模式，也没有插件沙箱。Rust 客户端无法承载它；用 Rust 重写 Cordis 及其服务集等于重写 dsh。
- UI 部分是给 dsh 自家 Web 应用的 React 18 bundle，GPUI 渲染不了。
- dsh 插件已经在一个地方**原样**运行：我们现有的 `ProviderKind::DeepSeek` provider 以 sidecar 方式启动 `dsh web`
  （`crates/waku-core/src/deepseek_session.rs`、`crates/waku-core/src/deepseek_pool.rs`、
  `crates/waku-core/src/driver/deepseek.rs`），dsh 从自己的 home 加载用户自己的插件。本文讲的不是那个 provider。
  不要把“移植 dsh 功能”和“改 DeepSeek provider”混为一谈。
- vendored 引擎的 `claurst-plugins` crate（`crates/waku-agent/plugins`）处于休眠状态（没有任何地方调用
  `load_plugins`），不是做这件事的途径。

## 2. 插件模型（Cordis）

五个核心概念（`docs/cordis-primer.md`）：

1. 插件可以是带可选 `inject` / `Config` 的函数，带 `apply(ctx, config)` 的对象，或 `Service` 子类。
2. context 是服务仓库，服务占用 `ctx.<key>`（`ctx.tools`、`ctx.llm`、`ctx.sessions`…）。插件按键查找服务，从不直接 import 实现。
3. `inject = ['tools', …]` 让插件等到这些服务存在才启动。加载顺序由依赖表达，而不是启动序列。
   缺少服务时插件保持 pending，而不是崩溃。
4. 类型化事件，通过 TypeScript 声明合并来声明，每个事件有固定的分发模式。
5. 每次注册都是可逆的 effect（`ctx.effect()` / `ctx.on()` 返回 disposer）；卸载插件会撤销它注册的一切
   （fiber 状态 PENDING → LOADING → ACTIVE → FAILED/UNLOADING → DISPOSED，`docs/user/develop/framework/index.md`）。

| 模式 | 是否 await | 顺序 | 返回值 |
|---|---|---|---|
| `emit` | 否 | 注册顺序 | 无 |
| `waterfall` | 否 | 注册顺序，环绕式中间件 `(…args, next)` | 有 |
| `parallel` | 是 | 同时 | 无 |
| `serial` | 是 | 注册顺序 | 有 |
| `bail` | 否 | 直到有一个 bail | 有 |

waterfall 监听器必须调用 `next()` 才会往下传；不调用就直接返回即短路。`prepend: true` 让监听器排在普通监听器之前。

最小宿主插件（`docs/user/develop/basic/tool.md`）：

```ts
export const name = 'greet-tool'
export const inject = ['tools']
export function apply(ctx: Context) {
  ctx.tools.register(defineTool({
    name: 'greet', description: 'Greet someone by name.',
    parameters: { name: { type: 'string', required: true } },
    output: { schema: { type: 'string' }, render: (_a, v) => [{ type: 'text', text: v }] },
    async execute(args) { return `Hello, ${args.name}!` },
  }))
}
```

`Config` 是 Standard Schema（实践中用 Schemastery）；标了 `.volatile()` 的字段可以实时修改而不重新挂载
（`docs/user/develop/basic/config.md`）。

打包（`docs/user/develop/basic/publish.md`、`packages/boot/app-boot/README.md`）：

- **Bundle**：一个 npm 包，其 `package.json` 含 `"dsh": { "bundle": { "patch": "./cordis.patch.yml" } }`。
  patch 是 YAML 行：`- insert: [{ id, name, config, disabled, inject, group, isolate }]`；`name` 是模块标识符；
  `!!js` 表达式针对 `ctx` 求值，所以配置文件本身就是代码。后面的层按 `id` 定位行，并替换其**整个** `config`（不做深合并）。
- **Profile**：`$DSH_HOME/profiles/<name>/`（`$DSH_HOME` 默认 `~/.dsh`），包含 `package.json`
  （`dsh.profile.bundles`、pnpm 依赖）、自己的 `cordis.patch.yml`、`compatibility.json`、`pnpm-workspace.yaml`
  （`allowBuilds`）。自带模板：`web`、`headless`、`sdk`、`sdk-minimal`、`acp`。
- 层顺序：按列表顺序的各 bundle（`@deepseek-ai/dsh-base` 最先）→ profile patch → `$DSH_HOME/cordis.patch.yml` → 每个 `--patch` 文件。
- 安装：`dsh plugin --profile <p> add <spec>`（底层是 pnpm）或 `ctx.pluginManager.installBundle`。
  对 `@deepseek-ai/dsh*` 的 `peerDependencies` 与运行时版本不匹配的插件会被拒绝。
- Plugin Manager 报告 `applied`、`restart-required`、`overridden` 或 `failed`（`packages/boot/plugin-manager/README.md`）。
- 展示元数据无需激活代码即可读取：`locale/<lang>.json` 的 `meta.title` / `meta.description`，以及导出的 `./icon`
  （`packages/preset/agent-preset/skills/cordis-plugin-development/references/host-plugin.md`）。
- 模板：`packages/preset/agent-preset/skills/cordis-plugin-development/templates/`
  （`decoration/` = UI 插件，`mcp/` = 纯配置 bundle）。

## 3. 宿主侧逻辑接口

主要服务（`docs/capability-seams.md` 把约 90 个键分为 `core`、`seam`、`bundle`、`service`）：

| `ctx` 键 | 所属包 | 插件用它做什么 |
|---|---|---|
| `tools` | `packages/core/tools` | `register(defineTool(...))`、`guard(fn)`（最终拒绝） |
| `commands` | `packages/interaction/commands` | `register({name, description, input?, handler(inv)})`——不经过模型轮次直接执行 |
| `skills` | `packages/skill/skill` | `registerProvider(...)` / `register(...)` |
| `llm` | `packages/llm/llm` | `registerAdapter(providers, adapter)`；adapter 实现 `async *stream(options)` |
| `systemPrompt` | `packages/core/system-prompt` | `section({name, order, text})`、`context()`、`variable()` |
| `agents` | `packages/core/agent` | 活的 `Agent` 句柄：`followup`、`steer`、`inject`、`cancel` |
| `sessions` / `sessionProjections` | `packages/core/session`、`packages/session/session-projection` | 只追加日志；从日志折叠出的类型化投影 |
| `approval` | `packages/interaction/user-approval` | `request(...)` → `approval/request` waterfall |
| `userQuestions` | `packages/interaction/user-questions` | 结构化提问 → `user-questions/request` waterfall |
| `jobs` | `packages/jobs/jobs` | 后台任务；`job_*` 工具读取和停止它们 |
| `subagents` | `packages/subagent/subagent` | 具名 provider、可续接的子 agent |
| `goals` / `planMode` / `compaction` | 见 §6 | 功能服务 |

工具定义（`docs/cookbook/adding-a-tool.md`、`packages/core/tools/src/schema.ts`）：

- `parameters` 是 dsh 自己的 schema DSL，编译为 JSON Schema 子集（不是 zod）。也接受原始 JSON Schema 的
  `ToolDefinition`（MCP 工具就是这样进来的）。
- `execute(args, exec)` 返回一个按 `output.schema` 校验的规范 JSON 值；`output.render(args, value)` 把它转成给模型看的
  `ContentBlock[]`；`output.presentationMeta(args, value)` 存在结果上供 UI 卡片使用；抛异常即 `isError`。
  `exec` 带有 `signal`、`agent`、`deferContext()`、`concludeTurn()`。
- 工具本身没有权限字段。策略由 `tools/pre-execute` waterfall 决定，返回
  `{kind:'allow'} | {kind:'deny', reason} | {kind:'cancel'} | {kind:'ask', reason?, displayReason?}`；`ask` 交给 `ctx.approval`。

新行为放哪里（摘自 `docs/architecture.md`）：

| 目标 | dsh 机制 |
|---|---|
| 模型 provider | 在 `ctx.llm` 上注册 adapter |
| 面向模型的能力 | 在 `ctx.tools` 上注册 |
| 人工命令 | 在 `ctx.commands` 上注册 |
| 后台任务 | 在 `ctx.jobs` 上注册 |
| 拦截请求 / 工具 / 轮次 | `agent/*` 或 `tools/*` 事件；`agent/turn-stopping` |
| 给模型的上下文 | `agent.inject()`（进入下一个被接纳的请求） |
| 持久状态 | 扩展 `SessionEventMap`；从日志渲染和重放 |
| 同会话目标 | `ctx.goals` |
| 单个 agent 范围的注册 | 用该 agent 的 `agent.ctx` |

值得照搬的规则：**模型可见即已记录**——每个模型请求都必须能从会话日志重建。

三角色能力模式（`docs/user/develop/practice/index.md`）：可替换的能力由 **Service Definition**
（抽象服务，拥有键和请求/结果类型）、**Service Provider**（实现）和 **Consumer**（通常是工具）组成。
Provider 和 Consumer 只依赖 Definition。对应到 Rust：一个 trait、它的实现、以及持有 `Arc<dyn Trait>` 的工具。

## 4. 一个轮次，逐个接缝

来源：`docs/architecture.md`（Turn flow）、`docs/agent-lifecycle.md`、`docs/tool-execution-pipeline.md`。
持久会话事件用**粗体**标出。

1. 收件箱：`agent.followup/steer/inject/send` → `agent/inbox/inserted`。每条消息带类型化的 `source`
   （`packages/llm/llm/src/message.ts`）。`inject` 不会唤醒 agent。
2. 唤醒：`agent/status` → running、**`turn/start`**，每条消息一个 `agent/inbox/claimed`。
3. `system-prompt/assemble`（W）：改写 sections、contexts、tools、variables。
4. `agent/pre-step`（W）：`{kind:'reject'}` 或 `{kind:'enter', messages, startsRequestSeries?}`。
   主要注入点——压缩压力、计划模式、目标轮次、AGENTS.md、时间上下文、技能目录、hooks 都挂在这里。
5. **`step/start`**。
6. `agent/request`（W）：替换 provider / model / effort / max tokens（不能改消息）。
7. 对账：**`system/message`**、**`user/message`**、**`request/header`**、**`request/context`**；
   请求从日志推导并冻结。
8. `llm/stream`（W）：包装 adapter（重试、重放、路由）。
9. `agent/assistant-stream`（E）：只有实时帧，不持久化。
10. **`assistant/message`**，或 **`assistant/attempt`** + `agent/request-error`（W），后者可返回
    `{kind:'retry'}`（压缩、图片卸载、llm-retry）。
11. 每个工具调用：**`tool/call`** → `tools/pre-execute`（W，allow/deny/cancel/ask）→ guards →
    `tools/execute`（W，环绕调度）→ 工具本体（`fs/write-intent` / `fs/edit-intent` W，`fs/observed` E）→
    `tools/post-execute`（W，可替换内容、阻断、附加 `additionalContexts`）→ `tools/result`（E）→ **`tool/result`**。
    pre 按顺序执行，execute 并发（`maxParallelToolCalls`），post 按模型顺序。
12. `additionalContexts` 以 `user/message` 追加。
13. **`step/end`**；如果工具还欠一次请求或有新输入，回到第 4 步。
14. `agent/turn-stopping`（S）：监听器可以 `agent.steer()` 强制再走一步。
15. **`turn/end`**，`agent/status` → idle。每次持久追加都会同时发出 `session/event`。

按模式分类的事件（来自 `docs/event-producer-consumer.md`）：

- Waterfall：`agent/pre-step`、`agent/request`、`agent/request-error`、`llm/stream`、`tools/pre-execute`、
  `tools/execute`、`tools/post-execute`、`tools/ptc-dispatch-log`、`system-prompt/assemble`、`approval/request`、
  `user-questions/request`、`fs/write-intent`、`fs/edit-intent`、`compaction/summary-error`、
  `session-telemetry/record`、`connection/request`、`workspace/session-activity`。
- Serial：`agent/created`、`agent/turn-stopping`。
- Parallel：`session/flush`、`feedback/committed`、`workspace/session-stop`。
- Emit：`agent/status|error|disposed|assistant-stream|inbox/*`、`session/event|created|disposed`、
  `tools/result|change`、`commands/change`、`skills/change`、`subagent/start|end`、`goal/*`、`workflow/*`、
  `plugin-manager/*` 等。

## 5. UI 侧

### 5.1 客户端模块

- 包通过 `"dsh": { "client": { "platform": "web", "inject": [...], "immediately"?, "external"? } }`
  加上 `exports["./client"]` 浏览器 bundle 来启用。`web` 是唯一的平台。
- 宿主 `ctx.clientModules`（`packages/client/modules/src/index.ts`）扫描已启用的行，构建启动图，
  把 `window.__DSH_BOOT__` 注入 `index.html`，并在 `/plugins/??a/client.js,b/client.js&rev=` 提供 bundle。
- 每个 bundle 注册 `window.__ModuleLoader__.load({ id, factory(require) { … return { inject, apply(ctx) } } })`。
  bundle 可 `require` 的共享模块：`react`、`react-dom`、`@deepseek-ai/cordis`、`dsh-client-store`、
  `dsh-client-ui-slots`、`dsh-client-ui-primitives`、`dsh-client-ui-dockkit`（`packages/client/web/src/platform.ts`）。
  浏览器里跑着自己的一棵 Cordis 树（`packages/client/web/src/boot-client.ts`）；`ctx.uiRenderer` 挂载 `root` 槽位。
- UI 是可选的。纯 UI 的包仍需要一个空的宿主 `export function apply() {}` 才能得到 Loader 行。
  约 23% 的包有客户端部分；工具、LLM adapter、MCP、skills、hooks 从来没有。

模板（`packages/preset/agent-preset/skills/cordis-plugin-development/templates/decoration/client.js`）：

```js
window.__ModuleLoader__.load({ id: '@local/my-decoration', factory(require) {
  const React = require('react')
  return { inject: ['slots'], apply(ctx) {
    ctx.slots.inject('conversation.composer.dock', () => ctx.slots.register(
      { name: 'conversation.composer.dock', id: 'my-decoration', order: 5 }, Decoration))
  } }
} })
```

### 5.2 槽位与客户端服务

槽位有基数——**S**ingle（单个）、**L**ist（列表）、**K**eyed（按条目键）、**C**hain（链）——和作用域：
**R**oot、**S**ession 或 session-**M**aybe。组件只拿到 props，拿不到 `ctx`。标准 props（`docs/subsystems/slots.md`）：
所有地方都有 `useResource`、`useSessions`、`useSessionStatus`、`useWorkspaces`、`usePanelInfo`；
会话作用域有 `sessionId`、`useSession`、`useProjection`、`useConversation`、`useInput`、`inputActions`、
`useChat`、`useTrajectory`；`t` 来自注册时的 `locale`。

| 客户端服务 | 用途 | 来源 |
|---|---|---|
| `ctx.slots` | `register`、`inject`、`renderSlot` | `packages/client/ui-renderer/src/client/registry.ts` |
| `ctx.resources` / `useResource` | `dsh-resource://` 提供者（`file`、`plan`、`subagentchat`） | `packages/client/resources`、`docs/subsystems/client-resources.md` |
| `ctx.remote.<ns>` | 生成的到宿主服务的 RPC（Typert `@Remote`） | `packages/api/remotes/src/client/index.ts`、`docs/api-gateway.md` |
| `ctx.sidebarRightTabs` / `ctx.sidebarRight` | 右侧面板 tab 类型；打开 / 分屏 / 浮动 | `packages/client/ui-sidebar-right/src/client/index.ts` |
| `ctx.configForms` | 插件配置表单（`state`、`mutate(ops, rev)`） | `packages/client/ui-settings/src/client/config-form.ts` |
| `ctx.locale`、`ctx.theme`、`ctx.shortcuts` | 国际化、主题 token、快捷键 | `packages/client/{locale,ui-theme,shortcuts}` |
| `ctx.commandUi`、`ctx.inputTriggers` | 斜杠弹窗；`/` 和 `@` 来源 | `packages/client/{ui-commands,ui-input-trigger}` |
| `ctx.layout`、`ctx.uiSession` | 面板；接管输入框的待处理交互 | `packages/client/{ui-layout,ui-session}` |

不存在的：toast 或对话框服务（各包把 `Toast` / `Modal` 原语放进 `shell.overlay`）、命令面板、系统通知。

槽位目录（精简版，全部 92 个键见 `slot-catalog.ts`）：

| 区域 | 键（基数/作用域） | 主要占用者 |
|---|---|---|
| 外壳 | `root` S/R、`sidebar` S/R、`main` K/R（面板 id）、`rightbar` S/R、`shell.bottom` S/R、`shell.leading` S/R、`shell.overlay` L/R | ui-layout；plugin-manager 和 schedule 添加 `main` 面板；`shell.overlay` 里有许多 toast 和对话框 |
| 左侧栏 | `sidebar.panellist` L/R、`sidebar.footer.action` L/R、`sidebar.workspaces` S/R、`sidebar.workspaces.session.menu.item` L/R、`sidebar.workspaces.session.row.action` L/R、`sidebar.session.row.hover/.leading` L/R | ui-sidebar、ui-workspace、plugin-manager、schedule |
| 设置 | `settings.section` L/R、`settings.general.item` L/R、`settings.models.provider-card` K/R、`settings.plugins.tab` L/R、`settings.onboarding` L/R | settings-* 系列包、theme、locale、shortcuts |
| 插件管理 | `plugins.item` L/R、`plugins.bundle.config` K/R、`plugins.row.config` K/R（`pkg#rowId`）、`plugins.detail.*` L/R | settings-agent-loop / shell / subagent / web-search、voice-input |
| 会话头部 | `conversation.session.header.actions` L/S、`.utilities` L/S、`.corner` S/S、`.lineage` S/S | agent-preset、jobs、subagent、agent-team；open-in-app、schedule、session-log-export |
| 输入框 | `conversation.composer` C/S、`conversation.composer.bar` S/M、`conversation.composer.dock` L/S、`conversation.input.dock` L/S、`conversation.input.overlay` L/S、`conversation.input.{model,permission,plan}` S/S、`conversation.input.activity` S/S、`conversation.input.attachments` S/M | approval 和 user-questions 接管输入框（chain）；todo / 队列 / goal 的 dock；斜杠和 `@` 菜单；模型 / 权限 / 计划选择器；语音输入 |
| 对话记录 | `conversation.chat.node` K/S（按 chat-node 类型）、`conversation.chat.assistant-actions` L/S、`conversation.chat.turnTail` L/S、`conversation.approval.detail` S/S、`conversation.plan-review.actions` L/S | ui-chat（15 种节点）、goal、tool、user-questions、workflow-run；轮次末尾的 deliverables / plan / schedule |
| 工具 | `tool.call.toolview` K/S（工具线上名）、`tool.call.images` S/S、`tool.view.cordis` K/S | ui-tool、ui-deliverables、ui-skill、ui-cordis |
| 右侧栏 | `sidebar.right.pane.tab` K/S（+ `.title`）、`sidebar.right.tab.document` K/S（渲染器 id）、`sidebar.right.tab.guide.entry` K/S | deliverables、plan、schedule、browser、documentpreview（8 个渲染器）、files、terminal、subagent |

### 5.3 工具卡片

- `ToolCallTree`（`packages/client/ui-tool/src/client/tool/ToolCallTree.tsx`）把每个根调用和嵌套的 PTC 子调用都通过
  `renderSlot('tool.call.toolview', owner, { entryKey: toolName, fallback: <GenericToolCard/> })` 渲染。
- Props：`callId`、`toolName`、`useDisclosure`、`cwd`、`openFile`、`loadImage`，加上阶段联合类型——
  `preparing`（流式的部分参数，唯一的流式阶段）、`start`（`argsRaw`）、`result`（`content`、`isError`、`error`、
  `meta`、`subCalls`）。契约：`packages/client/ui-tool/src/client/contract/slots.ts`。
- 卡片通过纯函数卡片模型（`diff`、`read`、`search`、`web`、`image`）把宿主的 `presentationMeta` 作为 `meta` 读取。
  宿主的 `presentCall` / `presentResult` 从不进入 Web 客户端。
- 专用视图在 `packages/client/ui-tool/src/client/apply.ts`：bash、read、write/edit、grep/glob、
  web_search/web_fetch、todo_write、ask_user_question，以及给 goal、schedule、job、terminal、subagent、team、
  session-query 工具用的通用 DetailsRow。`present`（deliverables）、`skill` 和 `cordis_*` 各有自己的包。
  其他一切，包括 MCP 工具，回退到 `GenericToolCard.tsx`。
- 有代表性的业务卡片：`packages/client/ui-skill/src/client/index.ts`。

### 5.4 值得读的 fixture

`apps/web/tests/fixtures/plugins/`：`fixture-input-extension`（手写的 `client.js`，用于 `conversation.input.activity`
和插件管理配置页）、`fixture-layout-bottom`（`shell.bottom`）、`fixture-live-client`（动态增删、HMR、
插件管理详情槽位、会话菜单项）、`fixture-bundle`（多行 bundle 元数据，没有客户端部分）。

## 6. 功能插件目录

宿主包 → 客户端包 → 子系统文档。工具用 `code` 标出。

| 领域 | 宿主包 | 工具 / 命令 | 客户端 | 文档 |
|---|---|---|---|---|
| Agent 循环 | `core/agent`、`core/agent-loop`、`core/tools`、`core/system-prompt`、`core/session` | `run_code`（PTC 模式） | ui-tool、ui-settings-agent-loop | `subsystems/core.md`、`tools.md` |
| 预设 | `preset/agent-preset-registry`、`preset/agent-preset`、`preset/persona` | — | ui-agent-preset | `core.md` |
| 上下文 | `context/agent-instructions`（AGENTS.md / CLAUDE.md 链）、`context/time-context`、`context/session-reference`、`context/file-reference` | — | ui-reference | `workspace.md`、`session-reference.md` |
| 压缩 | `compaction/compaction`、`compaction-basic`、`compaction-tool-result-pruner`、`compaction-image-offload`、`command-compact`、`llm/token-meter` | `/compact` | ui-chat | `compaction.md`、`token-meter.md` |
| 目标 | `goal/goal`、`goal/goal-round-driver`、`goal/tool-goal`、`goal/command-goal` | `create_goal`、`get_goal`、`update_goal`、`/goal` | ui-goal | `goal.md` |
| 计划 | `plan/plan-mode` | `exit_plan_mode`、`/plan` | ui-plan | `plan.md` |
| Todo | `todo/tool-todo` | `todo_write` | ui-conversation（TodoPanel）、ui-tool | `todo.md` |
| 工作流 | `workflow/workflow`、`workflow/workflow-ptc`、`workflow/tool-workflow`、`workflow/tool-ralph` | `workflow`、`ralph` | ui-workflow-run | `workflow.md` |
| 定时 | `schedule/schedule`、`schedule/tool-schedule` | `schedule_create/list/update/delete` | ui-schedule | `schedule.md` |
| 后台任务 | `jobs/jobs`、`jobs/jobs-local`、`jobs/tool-jobs` | `job_output`、`job_list`、`job_kill` | ui-jobs | `jobs.md` |
| 子 agent | `subagent/subagent`、`subagent/tool-subagent`、`subagent/tool-subagent-control`，provider `subagent-{spawn,fork}-in-process`、`subagent-{acp,codex,claude-code,dsh-sdk}` | `subagent`、`subagent_fork`、`list_subagent_models`、`send_message`、`interrupt_agent`、`list_agents` | ui-subagent、ui-settings-subagent | `subagent.md` |
| Agent 团队 | `experimental/agent-team`、`experimental/tool-agent-team` | `spawn_teammate`、`wait_agent`、`team_task_*` | `experimental/client-ui-agent-team` | `agent-team.md` |
| 审批 | `interaction/user-approval`、`interaction/permission-presets`、`sandbox/sandbox-policy`、`experimental/auto-review` | `/permission` | ui-approval、ui-permission-presets | `approval.md`、`permission-presets.md` |
| 提问 | `interaction/user-questions`、`interaction/tool-ask-user` | `ask_user_question` | ui-user-questions | `user-questions.md` |
| 防护 | `guard/repeat-tool-reminder`、`guard/timeout-policy`、`fs/fs-observation-policy` | — | — | `tools.md`、`filesystem.md` |
| 交付物 | `deliverables/tool-present`、`deliverables/workspace-changes` | `present` | ui-deliverables | `deliverables.md` |
| 会话工具 | `session-query/session-query`、`session-query/tool-session-query`、`session-query/session-log-export`、`session/session-title` | `session_search`、`session_trace`、`session_event_*`、`/export` | session-log-export | `session-query.md`、`session-title.md` |
| Skills / MCP / hooks | `skill/skill`、`skill/tool-skill`、`mcp/mcp-client`、`mcp/mcp-resources`、`hooks/hooks-claude-code`、`hooks/hooks-codex` | `skill`、`mcp__<server>__<tool>`、`read_mcp_resource` | ui-skill | `skills.md`、`mcp.md` |
| 其他 | `lsp/tool-lsp`、`webhook/webhook`、`browser-use/browser-use`、`computer-use/computer-use`、`spill/spill`、`feedback/message-feedback`、`experimental/claude-code-mods`、`experimental/voice-input-bundle` | `lsp`、`/feedback` | ui-message-feedback、`experimental/client-ui-voice-input` | `lsp.md`、`webhook.md`、`spill.md`、`feedback.md`、`claude-code-mods.md`、`voice-input.md` |

最值得移植的功能的机制说明：

- **目标**（`packages/goal/`）：一个带版本号的目标，由持久的 `goal/change` 快照折叠而来
  （`active | paused | blocked | complete`）。轮次驱动器等 `agent/status` 空闲后先 flush，再用 source
  `{kind:'goal', goalId, revision, round}` `followup()` 一条 `<goal_round>` 提示，并在 `agent/pre-step` 里复核。
  只有被接纳的消息才计一轮；达到上限时记录 `round-limit` 阻塞。`create`/`edit`/`pause`/`resume`
  要求当前轮次里有一条人类来源的消息；`blocked` 需要 ≥ 3 轮。恢复或分叉后目标保持未启用，直到人类恢复它；
  取消一轮会暂停目标。
- **计划**（`packages/plan/plan-mode/src/index.ts`）：只记日志的 `plan/mode`；指导文本是 order 50 的提示 section，
  配置在 `packages/bundle/base/cordis.patch.yml`。一个 prepend 的 `agent/pre-step` 监听器只在该步被接受后才应用待定的切换。
  `exit_plan_mode` 始终注册（工具列表稳定 → 缓存稳定），在计划模式外调用会失败，并通过 `ctx.userQuestions` 呈现计划。
  计划模式只是指导；真正的限制由沙箱和审批执行。
- **Todo**（`packages/todo/tool-todo`）：`todo_write` 整体替换列表（`{content, status}`，无 id）；只记日志的
  `todo/write`，最后一次写入为准；投影在下一轮开始时清空；不会重新注入提示。
- **压缩**（`packages/compaction/compaction-basic/src`）：一个 serial 的 pre-step 检查通过 `ctx.tokenMeter`
  给上一个请求计价；触发阈值 `floor(min(W×0.8, W−O−65536))`；先裁剪，再总结最旧的一段平衡区间，
  原样保留最新的 `W−O` 的 16%。溢出恢复挂在 `agent/request-error` 上。整个事务在日志里有起止标记
  （`compaction/start` → `compaction/summary` + 带 `surfaceOp` 的替换 `user/message` → `compaction/end`）；
  没有配对结束的 start 就是一把锁。它绝不把工具调用和结果拆开，绝不遮盖 system 节点 0，并拒绝没有缩小的摘要。
  摘要请求逐字节重放已缓存的前缀以保持缓存命中。
- **子 agent / 团队**（`packages/subagent/subagent`、`packages/experimental/agent-team`）：具名 provider，
  启动前检查能力标志；工具随其 provider 的出现和消失而注册、注销；后台一次性运行变成 job；
  可续接的子 agent 通过 `parent.inject()` / `steer()` / `followup()` 回报。团队：lead 的日志是唯一事实来源；
  名册 `team/member`，邮箱 `team/message/queued|delivered` 并在恢复时重发，任务是比较并交换的 DAG `team/task`。
  **已移植**：移植来源是独立的 dsh-agent-teams 包（MIT），不是这个实验插件，而且只接入内置 Agent。
  它替换了 fork 原来的 Settings → Workflow，设计见 `docs/agent-teams.md`。
- **自动审查**（`packages/experimental/auto-review/src/index.ts`）：一个 `auto` 权限预设（完全访问沙箱 + `ask` 策略）
  加一个 prepend 的 `tools/pre-execute` 监听器，让当前模型返回严格 JSON `{risk, decision, reason?}`。
  拒绝会落到 `ask`，由人来决定；进程内子 agent 固定为 `never`。参数从不被改写。
- **工作流**（`packages/workflow/`）：`workflow` 工具接收 `{script, meta, args}`；`workflow-ptc`
  在新的沙箱化 Node 进程里运行这段 JS，提供 `agent()`、`parallel()`、`pipeline()`、`phase()`、`log()`；
  每个 `agent()` 都走 `ctx.subagents`；子 agent 失败时解析为 `null`。持久的 `tool-workflow/*` 记录驱动对话记录里的运行节点。
- **定时 / 后台任务**（`packages/schedule/schedule/src`、`packages/jobs/tool-jobs`）：任务存在宿主存储域里，
  不在会话日志里；规则有 `after`、`at`、`every`（≥ 60 s）、带 IANA 时区的 `daily`/`weekly`、五字段 `cron`。
  只为最早的目标设一个定时器；投递时向（可能是冷的）会话 `followup()` 一条 `[SCHEDULE REMINDER]`；
  周期任务只投递最近一次错过的执行；投递不是原子的。任务完成时，agent 忙就 `inject()`，空闲就唤醒，
  唤醒次数有上限，人类输入会重置。

## 7. dsh → Waku 映射

Waku 没有槽位或服务注册表。一个 dsh 槽位要么变成一个封闭枚举的新变体加上若干 `match` 分支，
要么变成上游渲染函数里的一行 `.child()` / `.children()`；功能本身放在 fork 自有的新文件里（`docs/FORK.md` 的 “Design rule”）。

### 7.1 界面

| dsh 接缝 | Waku 接缝 | 如何接入 |
|---|---|---|
| `sidebar.panellist`、`main` 面板 | `src/app/sidebar.rs` 的 `render_sidebar_search`、`render_sidebar_action_row`；`src/app/model_status.rs` 的 `main_page_open` / `close_main_pages` / `main_page_title` | 在新文件里写一行入口；加一个 `.child()`；两处行高倍数都要加；在 `Waku` 上加状态字段；在 `src/app/render.rs` 里加 `.when(open, …)`（范例：`src/app/image_studio.rs`） |
| `sidebar.workspaces.session.*` | `src/app/sidebar.rs` 的 `SidebarRow`、`sidebar_row` | 新变体或行操作 |
| `sidebar.right.pane.tab` | `src/app.rs` 的 `RightPanelSurface`；`src/app/right_panel.rs`；`src/app/surface_bar.rs` 的 `SurfaceKind` | 新变体；编译器会列出要补的分支（`label`、`icon_path`、`reusable_surface_index`、`render_right_panel`）；可选头部按钮和快捷键 |
| `shell.overlay`（对话框、toast） | `Option<XState>` + 在 `src/app/render.rs` 两个分支里的 `render_x`；`src/app/confirm_dialog.rs` 的 `request_confirm`；`src/app.rs` 的 `show_toast*` | 新建模态文件；toast 辅助函数 |
| 横幅、`shell.quota-notice` | `src/app/update_banner.rs`、`src/app/daemon_banner.rs`、`src/app/error_banner.rs` | `render_x_banner` + 在 `render.rs` 里加 `.children()` |
| `shell.bottom` | 没有（只有 `src/app/composer.rs` 里的 `render_workspace_footer`） | 需要新建 |
| `conversation.session.header.actions` | `src/app/sidebar.rs` 里的 `render_header` | 加一个 `.child(self.render_x_action(cx))`；用 `src/ui` 的 `icon_button` / `popover` |
| `conversation.chat.node`、`turnTail` | `src/app/transcript.rs` 的 `TranscriptRowKind`；`src/app/transcript_view.rs` 的 `transcript_row` | 新行类型或在行构建器里加分支 |
| `tool.call.toolview` | `src/app/transcript_view.rs` 的 `render_activity_item`（子 agent 先例：`src/app/subagent_row.rs`） | 没有按工具的注册表，`ActivityItem`（`crates/waku-protocol/src/model.rs`）也没有工具名字段：加一个 `#[serde(default)]` 字段，在 `crates/waku-core/src/driver/native.rs` / `driver/activity.rs` 里赋值，提前返回到新文件里的渲染函数，运行 `bun run protocol:generate` |
| `conversation.composer.dock`、`conversation.input.dock` | `src/app/render.rs`（输入框上方 / 下方）；`src/app/composer.rs` 的 `render_composer` 控件行 | 加一行 `.children()`；或在模型 / effort / 模式 / goal 芯片旁加控件 |
| `conversation.input.overlay`（斜杠、`@`） | `src/app/autocomplete.rs`；`crates/waku-client/src/composer_complete.rs` | 新的触发来源或行 |
| `conversation.composer` 链（审批、提问）、`conversation.approval.detail`、`plan-review.actions` | `src/app/permission_card.rs` 的 `render_permission_card`（计划分支：`is_plan_approval`）；`src/app/composer.rs` 的 `render_user_input` | 像计划审批那样在卡片里加分支 |
| `settings.section`、`settings.general.item`、`plugins.*` | `src/app.rs` 的 `SettingsPage`；`src/app/settings.rs` 的 `SETTINGS_PAGES` | 变体 + 一行（数组长度加一）+ 标题和分发分支 + `settings.x` / `settings.x_keywords` 键 + `src/app/tests.rs` |
| `ctx.shortcuts`、命令面板 | `src/app/command_palette.rs` 的 `PaletteAction`；`src/app/shortcuts.rs` | 新动作变体 |
| `ctx.locale` | `locales/app.yml`（英文，`en:` 行）、`locales/zh-CN.yml`、`locales/ja.yml`；`tr!` | 每个功能一个键命名空间 |
| `ctx.theme`、原语 | `src/theme.rs`；`src/ui/mod.rs`、`src/ui/menu.rs`、`src/ui/motion.rs` | 复用；图标放 `assets/icons/` + `src/assets.rs` |

### 7.2 逻辑

| dsh 接缝 | Waku 接缝 | 作用范围 |
|---|---|---|
| `ctx.tools.register` | `Tool` trait（`crates/waku-agent/tools/src/lib.rs`，vendored——不要改）；`crates/waku-agent-bridge/src/session.rs` 的 `builtin_tools` / `engine_tools`；范例 `crates/waku-agent-bridge/src/subagent.rs`；可开关列表 `crates/sub2api/src/agent_settings.rs` 的 `BUILTIN_TOOLS`；标题在 `native.rs` | 仅内置 agent；要对所有 provider 生效就改为提供一个 MCP 服务器 |
| `ctx.systemPrompt.section`、`agent.inject` | `crates/waku-agent-bridge/src/config.rs` 的 `session_rules` / `refresh_session_rules` | 内置 agent |
| `tools/pre-execute` 策略、`ctx.approval` | `crates/waku-agent-bridge/src/permission.rs`（`PermissionBridge`、`GuiPermissionHandler`） | 内置 agent |
| `ctx.userQuestions` | `session.rs` 的 `forward_questions` → `DriverEvent::UserInputRequested` | 内置 agent；其他 provider 各自映射 |
| `ctx.jobs`、子 agent 记录 | `BackgroundWorkEvent`（`crates/waku-protocol/src/model.rs`）、`crates/waku-agent-bridge/src/background.rs`、`src/app/background_work.rs` | 所有上报它的 provider |
| goal / plan / todo / 压缩 | goal：`crates/waku-agent-bridge/src/goal.rs`、`src/app/goal_dialog.rs`；plan：`InteractionMode`、`/plan`；todo：`crates/waku-protocol/src/todo.rs`、`src/app/todo_list.rs`；压缩：`DriverEvent::ContextCompaction` | 已经存在；移植改进点，不另起新系统 |
| `ctx.commands` | `crates/waku-core/src/composer_complete.rs` 的 `assemble_slash_commands`；`src/app/composer.rs` 的 `execute_local_composer_command`；零代码：`.cheaprouter/commands/*.md` | 应用层 = 所有 provider |
| 宿主服务 + `ctx.remote` | `Command`（`crates/waku-protocol/src/protocol.rs`）+ `handle_driver_command`（`crates/waku-core/src/daemon.rs`）；无状态的用 `WorkspaceOperation`（`crates/waku-protocol/src/workspace.rs`） | daemon |
| 会话事件 | `DriverEvent`（`model.rs`）+ `crates/waku-protocol/src/driver_wire.rs` | 旧客户端遇到未知类型会报错——优先用 `BackgroundWorkEvent` 或 `serde(default)` 字段 |
| 持久状态 | `AgentSession` / `AgentTurn` 的 `#[serde(default)]` 字段（`model.rs`，存在 `app.db`）；新表通过 `db/schema.ts` + `bun run db:generate`；fork 本地 JSON 通过 `crates/sub2api` 存在 `~/.cheaprouter/` | — |

始终写明移植落在哪个范围：**仅内置 agent**（bridge）还是**所有 provider**（daemon / 应用层）。

## 8. 移植步骤

1. 找到配对：宿主包（逻辑）和客户端包（界面）——见 §6。
2. 读它的 `README.md`、它的 `docs/subsystems/*.md` 页面和 `src/`。列出它用到的接缝（§4）、写入的持久事件，以及所有提示文本。
3. 决定 Waku 层级和范围（§7）：bridge、driver、daemon 还是应用层；仅内置 agent 还是所有 provider。
4. 遵守 `docs/FORK.md`：新功能放新文件；上游文件只加一到三行的挂接点并登记到 register；新文件列入
   “Files that are ours entirely”；在 `NOTICE.md` 加一行带日期的记录；协议改动只做增量（`serde(default)`、新的
   workspace operation）。除非记录为 departure，否则不改 vendored 引擎。
5. 遵守 `AGENTS.md`：`render` 能到达的路径上不做 I/O、不起进程；耗时工作放到 `cx.background_executor()`；键盘可操作。
6. 在三个 locale 文件里、同一个命名空间下添加 i18n 键。
7. 近似翻译时保留署名——见 §9。

## 9. 许可证

- 仓库是 MIT，“Copyright (c) 2026 DeepSeek”（`LICENSE`）。vendored 的 Cordis 及其兄弟包是 MIT，
  “Copyright (c) 2021-present Shigma”（`vendor/cordis/LICENSE`）。近似翻译代码或提示文本（目标轮次提示、
  压缩前言、`packages/bundle/base/cordis.patch.yml` 里的计划指导）时，在新文件头部和 `NOTICE.md` 里保留 MIT 声明。
  我们的 fork 是 GPL-3.0-only；MIT 代码可以连同声明一起纳入。
- 不要复制：`@deepseek-ai/libreoffice-kit*`（MPL-2.0）和 `@anthropic-ai/claude-agent-sdk`（非宽松许可）。见 `THIRD_PARTY_NOTICES.md`。
