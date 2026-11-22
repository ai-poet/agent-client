# Changelog

Release entries supply the corresponding GitHub Release notes. A version is
released only after its entry is here.

## [0.5.0](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.5.0) — 2026-11-22

修复 macOS 每次启动重复弹出欢迎引导的问题，并将内置 Agent 提示词自称统一为 CheapRouter Harness。

Fixes the welcome guide reopening on every macOS launch and updates the built-in Agent prompt identity to CheapRouter Harness.

### 修复 / Fixed

- 欢迎引导只出现一次：点击右上角关闭按钮不再漏记已读状态，首启迁移写入偏好不再覆盖引导状态，退出前同步落盘偏好，修复 macOS 每次启动重复弹引导的问题

  The welcome guide now appears once: clicking its close button records the dismissed state, first-launch migration no longer resets it when writing preferences, and pending preference edits flush synchronously on window close, fixing the macOS repeat-onboarding bug

- 内置 Agent 提示词的品牌自称改为 CheapRouter Harness，保留既有 `sailry://` 与 `sailry-image` 协议前缀不变

  Updated the built-in Agent prompt self-reference from Sailry to CheapRouter Harness while keeping existing `sailry://` and `sailry-image` wire protocol schemes intact

## [0.4.3](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.4.3) — 2026-11-22

按分组使用原生接口协议、充值收进账号弹层，并修复旧会话压缩失败。

Each bound group now uses its native API protocol, top-ups live in the account popover, and old conversations compact again.

### 新增 / Added

- 按分组使用原生接口：GPT / Codex / Grok 走 Responses，国模走 Chat Completions，Claude 走 Messages，Gemini 走原生流式；网关明确不支持时按声明回退

  Native APIs per bound group: GPT, Codex and Grok use Responses, domestic models use Chat Completions, Claude uses Messages and Gemini its native stream, with an explicit opt-out honoured

- 充值入口移到左下角账号弹层，展示余额与套餐；弹窗改为两层——金额与快捷档位、预计到账（含赠送）→ 支付方式 → 下单后进入二维码、倒计时与取消

  Top-up now starts in the account popover with balance and plan; the dialog is two layers: amount and quick amounts with the expected credit, then the payment method, then the QR code, countdown and cancel

- 自动检查到新版本时弹出升级窗口，显示版本与更新日志，「稍后」按版本记住

  A new release opens an upgrade window with the version and its notes, and "Later" is remembered per version

- Ask User 提问到达即展开选项，无需再点；没在输入时可直接用方向键与回车作答

  Ask User questions open their choices on arrival, and arrow keys with Enter answer them without a click when nothing is being typed

### 修复 / Fixed

- 旧会话压缩报「没有可用模型」：会话仍引用分 lane 之前的统一 provider，现按模型继任到承载它的 lane provider，不会跨 lane 顶替

  Compaction of conversations created before per-lane providers reported no available model: a selection on the retired route now follows the lane that carries its model, never another lane

- 压缩失败不再只显示「操作失败」，而是给出引擎原因（模型未按格式返回、摘要为空、被输出上限截断等）

  A failed compaction no longer reports only a generic failure; it names the engine's reason (invalid structured handoff, empty summary, truncated output)

- macOS 应用图标与 Windows 使用同一份品牌图标，旧品牌图标文件已移除

  The macOS app icon now matches Windows and shares one brand source; the old brand icon file is gone

- 同步上游 0.2.7 的核心修复：事件交付时校验权限、拨号重试与证书失败分类、Git 工作树截断防护、前台子任务释放、队列修订同步等 13 项

  Thirteen core fixes from upstream 0.2.7: per-event authorization at delivery, dial retry and certificate failure classification, Git worktree truncation guards, foreground child release, queue revision syncing

## [0.4.1](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.4.1) — 2026-11-22

修复从 0.3.8 自动更新到 0.4.0 时安装器一闪即退的问题。

Fixes the installer that exited immediately when 0.3.8 updated itself to 0.4.0.

### 修复 / Fixes

- Windows 安装器不再用 `taskkill /T` 结束旧进程：0.3.8 的更新是由旧客户端启动安装包，`/T` 会连安装包自己一起杀掉，导致更新点了没反应、客户端目录仍是旧版本

  The Windows installer no longer ends the previous build with `taskkill /T`: 0.3.8 starts the setup from the running client, so `/T` killed the setup itself, leaving the update with no effect and the old build in place

- Windows 安装器只在记录目录里确实还有本应用时才原地升级，避免遗留的安装记录把新装的位置带到别处

  The Windows installer continues in a recorded directory only when this application is still there, so a leftover install record cannot move a fresh install somewhere unexpected

## [0.4.0](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.4.0) — 2026-11-21

CheapRouter Desktop 从这一版起是 MyGo 原生客户端，旧的 Rust 客户端已移除，仓库只保留 Go 实现。

This release turns CheapRouter Desktop into a MyGo-native client. The legacy Rust client is gone and the repository keeps only the Go implementation.

### 主要改动 / Highlights

- 桌面端从 Rust 客户端迁移到 Go/MyGo，仍是原生渲染、不套 Electron：长对话不卡顿，没有独立显卡时走 CPU 渲染

  The desktop moves from the Rust client to Go/MyGo while keeping native rendering and no Electron shell: long conversations stay smooth, and CPU rendering is used without a GPU

- CheapRouter 接入：PKCE 回调登录、站点与 CDN 切换、按 lane（Claude / Codex / Gemini / 国模 / Grok）各自绑定分组，未绑定的 lane 登录后自动绑定第一个可用分组；分组显示在线率与福利/常驻徽章，余额与套餐同步；不登录也能正常使用客户端

  CheapRouter integration: PKCE loopback sign-in, site and CDN switching, per-lane group binding for Claude, Codex, Gemini, domestic models and Grok, with automatic first-group binding for unbound lanes, online rate and welfare badges, balance and plan sync. The client stays usable without an account

- 智能体工作台：原生 agent 循环、多文件修改、Git 与 worktree、内置终端、代码审查、插件，以及通过 iroh 连接远程主机

  Agent workstation: native agent loop, multi-file edits, Git and worktrees, built-in terminal, code review, plugins, and remote hosts over iroh

- 创作工作台：文生图与图生图，自动识别网关的画图模型，也可以在设置里配置自定义画图接口

  Image studio: text-to-image and image-to-image, automatic discovery of gateway image models, and configurable custom image endpoints

- 两跳更新：旧 Rust 客户端原地升级到本版，之后由 MyGo 更新器接管；清单使用 Ed25519 签名，替换过程为原子操作

  Two-hop updates: in-place upgrade from the previous Rust client, then MyGo's updater takes over; Ed25519-signed manifests and atomic replacement

- 无损迁移：首次启动读取旧数据目录，先备份再按稳定 ID 导入项目、会话与历史记录

  Lossless migration: the first launch reads the old data directory, backs it up, and imports projects, conversations and history by stable ID

- 品牌统一为 CheapRouter，关于页保留「基于 Sailry 开发」与 GitHub 链接

  Branding is CheapRouter throughout, with Sailry attribution kept on the About page

## [0.3.8]

- 修复图片输入的若干问题：本地 BMP、ICO、TIFF、PNM 等格式现在会先无损转换为 PNG 再发送，内置 Agent 能正确回传工具调用产生的图片，多工具同时返回带图结果时消息顺序不再错乱

  Fixed several image input issues: local BMP, ICO, TIFF and PNM files are now losslessly transcoded to PNG before sending, the built-in agent correctly passes back images produced by tool calls, and message ordering stays correct when several tools return image results at once

- 内置 Agent 新增 Gemini 4 Carbon 与 Gemini 4 Hydrogen，与 Argon 一样走 Gemini 原生接口

  The built-in agent now supports Gemini 4 Carbon and Gemini 4 Hydrogen, both served over Gemini's native API the same way as Argon

## [0.3.7]

- 内置 Agent 支持用中英文自然语言发起 Agent Teams：消息发送前不再按关键词预解析，由模型调用组队工具，当前会话担任队长；模型调用无论省略审批参数还是传入 automatic 都只能先建草案，同回合不能自行批准，须在后续用户回合明确批准或点击团队面板按钮后才会启动。斜杠命令及模板别名保留。团队成员的模型路由在草案和成员行中清晰展示。会话详情栏的后台子智能体可单项挂起或停止，长进程可单项停止；后台命令、子智能体与团队成员在成功、失败或停止后主动通知 Agent，主会话忙碌时排队、空闲时自动唤醒，回合收尾竞态下也不会丢失通知

  Built-in agents can start Agent Teams from natural-language requests in Chinese or English: messages are no longer pre-parsed for keywords, and the model calls the team tools itself, with the current session as captain. Model calls always stage a draft, even when approval is omitted or automatic is supplied; same-turn self-approval is rejected, and work starts only after explicit approval in a later user turn or from the Team panel. Slash commands and profile aliases remain available. Member model routes stay visible in drafts and roster rows. Session-detail rows can detach or stop individual background subagents and stop long processes. Background commands, subagents and team members notify the agent on success, failure or stop, queueing while it is busy and waking it when idle without losing notices at the turn-settlement boundary

- 轨迹面板支持导出当前会话的轨迹日志，便于留存记录与排查问题

  The trajectory panel can now export the current session's trajectory logs, making it easier to retain records and investigate issues

- 旧外部 CLI 会话与草稿现在会友好引导至内置 Agent：历史会话仍可阅读，新工作转由内置 Agent 处理，相关操作不再暴露内部错误

  Legacy external CLI sessions and drafts now guide users smoothly to the built-in agent: old transcripts remain readable, new work is handled by the built-in agent, and related actions no longer expose internal errors

- 旧版 Windows 默认改用不透明背景，避免 Acrylic 材质造成窗口漂移；新版 Windows 的 Mica 材质保持不变

  Older Windows versions now use an opaque background by default to prevent window drift caused by Acrylic, while Mica remains unchanged on newer Windows versions

## [0.3.6]

- 内置 Agent 移植 Sailry 执行内核：加入新的执行状态机、流式响应对应关系、工具并发、取消与中途转向、状态恢复和请求恢复；现有模型、工具、权限与历史数据保持兼容

  The built-in agent now uses Sailry's execution core, bringing its state machine, stream/tool pairing, bounded tool concurrency, cancellation, steering, state restoration and request recovery while preserving existing models, tools, permissions and history

- 自动压缩支持自定义百分比阈值，并完善压缩阶段与重试通知，让长任务的上下文处理和恢复过程更清楚

  Automatic compaction now accepts a custom percentage threshold, with clearer compaction-stage and retry notifications for long-running tasks

- 工具调用会保存完整原始输出，支持按页读取历史结果和命令的 stdout / stderr，无需重新执行命令；分页不会截断中文等多字节字符

  Tool calls now retain their complete original output, with paginated access to previous results and command stdout / stderr without re-running commands; paging preserves multibyte characters such as Chinese text

- 内置 Agent 成为唯一使用的 Agent，无需安装或配置外部 CLI；移除外部 CLI 的自动配置、会话接入、命令发现和旧用量统计页，输入框仅保留实际支持的斜杠命令与技能

  The built-in agent is now the only agent, with no external CLI installation or configuration required; external CLI auto-configuration, session integration, command discovery and the legacy usage page are removed, leaving only supported slash commands and skills in the composer

- 模型选择器直接打开内置 Agent 并按厂商分类，支持跨厂商搜索、键盘切换和自定义接口管理；选中模型后显示对应厂商图标，侧栏项目区不再被工具入口挤到下方

  The model picker opens the built-in agent directly with vendor grouping, cross-vendor search, keyboard navigation and custom endpoint management; the selected model shows its vendor icon, and sidebar tools no longer push the project section down

- 底部源站切换现在会联动模型目录、余额、用量、订阅、公告和充值等业务请求，旧源站的迟到响应不会覆盖新数据；登录与令牌刷新仍使用认证域名

  Switching the gateway origin now updates model catalogs, balances, usage, subscriptions, announcements and payments together, while late responses from the previous origin are discarded; sign-in and token refresh remain on the authentication origin

## [0.3.5]

- 云账户模型分组新增「福利」与「常驻」双轨分类：福利线路带「限时 · 波动」标识，切换前会提示确认，任务进行中也会给出状态提醒

  Cloud account model groups are now split into welfare and standard categories: welfare routes carry a "Limited time · Variable" badge, switching to one asks for confirmation, and a busy task produces a status warning

- 新增守护进程重连诊断：可从设置导出近期连接错误与重连历史，报告包含时间、系统错误、版本与进程信息，不包含鉴权令牌、API 密钥或消息正文；日志支持自动轮转

  Added daemon reconnect diagnostics: export recent connection errors and reconnect history from Settings, including timestamps, OS errors, versions and process details while excluding auth tokens, API keys and message bodies; logs now rotate automatically

- 提升 MCP 生命周期与 OAuth 稳定性：修复删除会话、停用服务及关闭时的进程清理与取消处理，避免 OAuth 刷新阻塞取消操作

  Improved MCP lifecycle and OAuth stability: fixed process cleanup and cancellation when deleting sessions, retiring services or shutting down, and prevented OAuth refreshes from blocking cancellation

## [0.3.4]

- 内置 Agent 的轨迹面板新增诊断报告：在本地汇总失败请求、重试、慢响应和缓存读取异常，按严重程度展示每条线索，并可切换详细视图，便于定位运行问题

  The built-in agent's trajectory panel now includes diagnostics: failed requests, retries, slow responses and cache-read anomalies are summarized locally, with severity for each finding and a toggleable detailed view to make runtime issues easier to investigate

- 优化主页、侧栏与输入框的窄宽度布局，限制内容宽度并支持必要的换行，避免文本与控件溢出

  Improved narrow layouts across the home page, sidebar and composer, constraining content width and wrapping where needed to prevent text and controls from overflowing

- 修复 Windows 内置浏览器的 DirectComposition 视觉对象创建与显隐处理，提升 WebView2 显示稳定性

  Fixed DirectComposition visual creation and visibility handling for the built-in browser on Windows, improving WebView2 rendering reliability

## [0.3.3]

- Review 面板支持全文搜索：可以查找折叠文件和尚未展开的上下文，命中后自动展开并定位；文件可标记为已读并折叠，内容再次变化时会自动恢复为未读

  Review now supports full-text search across collapsed files and hidden context, expanding and revealing each match automatically. Files can be marked as viewed and collapsed, and changed content automatically becomes unread again

- 子 Agent 的展示更清晰：连续子任务可以分组展开，显示成员数量、运行与失败状态；每个成员可展示角色、模型和结果预览，已有的流式记录与详情入口保持不变

  Subagent activity is clearer: consecutive child tasks can be grouped and expanded, with member counts and running or failed states; each member can show its role, model and a result preview while preserving existing streaming records and detail views

- 内置 Agent 的重试与中断恢复更可靠：保留中断前已经输出的内容，避免重复或覆盖；重试保持请求上下文稳定，并改进后台进程停止与失败恢复

  Built-in agent retries and interruption recovery are more reliable: output shown before an interruption is preserved instead of duplicated or overwritten, retries keep request context stable, and background-process stopping and failure recovery are improved

## [0.3.2]

- 新任务页支持自定义壁纸：可在「设置 → 外观」选择本地图片，调整焦点与缩放，应用抖动、ASCII、半色调或扫描线效果；切换壁纸时平滑交叉淡化，界面表面可随壁纸取色。窗口、侧栏与输入框采用更统一的半透明材质，macOS 支持整窗模糊

  Custom wallpapers for the new-task page: choose a local image under Settings → Appearance, adjust its focal point and zoom, and apply Dither, ASCII, Halftone or Scanlines effects; wallpaper changes crossfade smoothly, and surfaces can take their tint from the artwork. The window, sidebar and composer now share a more consistent translucent treatment, with whole-window vibrancy on macOS

- 窗口最大化按钮现在可在最大化与还原之间切换，并修复 Windows 最大化后无法恢复窗口大小的问题

  The window maximize button now toggles between maximized and restored states, fixing Windows windows that could not return to their previous size after being maximized

## [0.3.1]

- 界面更清爽、更有层次：菜单、命令面板和状态胶囊采用半透明毛玻璃，退出时模糊与内容一起淡出；标题栏低频面板入口收进「三个点」菜单；Agent 工作行改为安静的点阵活动提示，时间、token 和状态文字不再整行闪烁

  A cleaner, more layered interface: menus, the command palette and the status capsule use translucent frosted glass, with the blur fading out alongside the content; lower-frequency panel entries move into the title bar's "..." menu; and the Agent working row uses a quiet cell activity indicator instead of shimmering its timer, token count and status text

- 新增界面动态效果偏好：可跟随系统或减少动态效果，也可让动画在窗口进入后台时暂停；菜单、Toast、侧栏重排和长消息折叠等交互更连贯

  New interface motion preferences: follow the system setting or reduce motion, and optionally pause animations while the window is in the background; menus, toasts, sidebar reordering and long-message folding now move more coherently

- 外部链接打开更安全：应用和终端只允许 HTTP、HTTPS 与 mailto 链接，阻止其他协议被外部调用；HTML/HTM 文件优先交给默认浏览器，Windows 本地路径也能正确识别

  Safer external links: the app and terminal only allow HTTP, HTTPS and mailto links, preventing other protocols from being invoked externally; HTML/HTM files open in the default browser first, and Windows local paths are recognized correctly

- 修复 Windows 内置浏览器与 GPUI 的 DirectComposition 接口兼容性，WebView2 的显示、裁剪和可见性使用正确的原生视觉对象；GPUI 源码现已随项目提供，源码构建不再依赖电脑上的外置目录

  Fixed DirectComposition compatibility between the built-in browser and GPUI on Windows: WebView2 now uses the correct native visual interfaces for display, clipping and visibility; GPUI sources are also bundled with the project, so building from source no longer depends on an external directory on the computer

- 提升 Agent 后端与开发工具的稳定性：包含非 UTF-8 字节的输出使用替换字符读取，不再中断；进程退出时避免管道长期阻塞；开发脚本兼容不支持 `.nothrow()` 的旧版 Bun

  Improved Agent backend and development-tool stability: output containing non-UTF-8 bytes is decoded with replacement characters instead of failing, process exits no longer wait indefinitely on blocked pipes, and the development script works with older Bun versions that do not support `.nothrow()`

## [0.3.0]

- 内置 Agent 支持 Gemini 4 Argon：走 Gemini 原生接口，可选低、中、高三档思考，Gemini 和 Antigravity 分组都能用；设置 → 云账户里有单独的 Gemini 分组，默认选第一个并自动准备好密钥；一次并行调用多个工具也能正常执行

  The built-in agent can use Gemini 4 Argon over Gemini's own API, with low, medium and high thinking, through Gemini and Antigravity groups alike; Settings → Cloud Account has a Gemini group of its own that starts on the first one with a key set up for you; and several tools called at once run as they should

- 新增会话详情栏：对话右侧显示运行环境（电脑、文件夹、分支）、上下文用量、未提交的改动和后台任务，可以直接停止后台任务、打开 Review 或提交；打开浏览器、终端等工具时会被覆盖，窗口较窄时自动让出位置，标题栏按钮可显示或隐藏。任务执行中，待办清单作为对话的最后一行显示

  A new session details rail beside the conversation shows the environment (computer, folder, branch), context usage, uncommitted changes and background tasks, and lets you stop a background task, open Review or commit; tools such as the browser or terminal cover it, it steps aside on narrow windows, and a title-bar button shows or hides it. While a task runs, its to-do list is the last row of the conversation

- 需要你处理的任务一眼可见：等待授权、确认计划、回答问题或团队成员请求的任务，在侧栏显示提醒图标并排在前面，Dock 图标显示数量；⇧⌘A（Windows 为 Ctrl+Shift+A）跳到下一个

  Tasks waiting on you stand out: one holding a permission, a finished plan, a question or a team member's request gets an alert icon in the sidebar and sorts to the top, the Dock icon shows how many, and ⇧⌘A (Ctrl+Shift+A on Windows) jumps to the next one

- 侧边提问：按 ⌘;（Windows 为 Ctrl+;）、点标题栏按钮或输入 /btw，就能针对当前任务另外提问；回答显示在右侧，不写进主对话，也不打断正在进行的任务

  Side chat: press ⌘; (Ctrl+; on Windows), use the title-bar button or type /btw to ask about the current task; the answer appears on the right, never enters the main conversation and does not interrupt the running task

- Review 面板支持行内评论：在改动的任意一行点「+」（或用方向键加 Enter）写评论，⌘↩（Windows 为 Ctrl+Enter）一次全部发给 Agent，任务进行中会自动排队

  Line comments in Review: click "+" on any changed line (or use the arrow keys and Enter) to comment, and ⌘↩ (Ctrl+Enter on Windows) sends them all to the agent at once, queued while a task is running

- 内置 Agent 新增轨迹面板：用时间线和明细列出每次模型请求（首字延迟、用时、用量、重试原因）、工具调用、压缩和回退，点工具行上的「检查」可直接定位；记录可在设置 → Agent 中关闭

  A trajectory panel for the built-in agent lays out every model request (time to first token, duration, usage, retry causes), tool call, compaction and rewind on a timeline and in a ledger, and "Inspect" on a tool row jumps straight to it; recording can be switched off in Settings → Agent

- 出错时更好排查：错误信息末尾附上请求 ID；错误提示按原因给出对应操作，例如压缩上下文后继续、打开模型提供方或账户设置；重新发送时显示「正在重新发送」，子 Agent 的重试也会显示；额度已用完的错误不再白白重试

  Failures are easier to act on: the message ends with the request ID; the banner offers the step that fits the cause, such as compacting the context and continuing or opening model provider or account settings; a resend says so, and sub-agent retries show too; and an exhausted quota is no longer retried

- 画图使用聊天输入框上方选择的节点（主站或 CDN），不必再单独选择

  The image studio draws through the node picked above the composer (main site or CDN), with no separate choice to make

- Codex 的模型列表去掉已停用的 GPT-5.5 和 GPT-5.4，只保留 GPT-6.1-Sol、GPT-6-Astra、GPT-6-Sol、GPT-5.6-Sol 和 GPT-5.6-Terra

  The Codex model list drops the retired GPT-5.5 and GPT-5.4, leaving GPT-6.1-Sol, GPT-6-Astra, GPT-6-Sol, GPT-5.6-Sol and GPT-5.6-Terra

## [0.2.16]

- 上下文圆环现在显示真实用量：DeepSeek、GLM、Kimi 等模型以及经网关转换格式的请求之前读不到用量，圆环一直接近 0；现在每一步模型调用和每次压缩后都会更新。自动压缩也改为按圆环显示的上下文窗口触发，1M 窗口的模型不再在 20 万左右就被压缩

  The context ring now shows real usage: DeepSeek, GLM, Kimi and requests the gateway converts between API formats used to report none, leaving the ring near zero; it now updates after every model call and every compaction. Automatic compaction also follows the window the ring shows, so a 1M-window model is no longer compacted at around 200k

- 内置 Agent 更稳、更安全：后台启动的开发服务器不再两分钟后被结束；命令或网页里的中文不再让会话卡在「工作中」；任务进行中每一步都会保存，应用意外退出后能接着做；同一文件的多处修改不再互相覆盖；修改文件前必须读过最新内容，不会再覆盖你刚改的地方；超长输出完整保存到文件，对话只保留首尾；后台命令结束时会主动告诉 Agent；反复执行同样的操作时会提醒它换个思路

  The built-in agent is steadier and safer: a dev server started in the background is no longer stopped after two minutes; Chinese text in a command or a page no longer leaves a session stuck working; every step of a task is saved as it goes, so it can carry on after the app quits unexpectedly; several edits to one file no longer overwrite each other; a file is only changed once its latest content has been read, so your own recent edits are never overwritten; very long output is saved in full to a file with only its start and end kept in the conversation; the agent is told when a background command finishes; and it is nudged to change course when it keeps repeating the same step

- 权限更细：Bash 和 PowerShell 的「始终允许 / 始终拒绝」只针对当前这一条命令，不再一次放行或封禁所有命令

  Finer permissions: "Always allow" and "Always deny" on Bash and PowerShell now cover that one command, instead of every command

- 出错时说得更清楚：限流、网络、上下文超长、密钥失效、服务故障会显示中文说明和处理建议；停止任务不再弹出英文提示；失败提示和权限卡上的按钮可以用键盘操作；在非 Git 文件夹里改了文件时，会说明为什么无法查看或撤销改动；MCP 工具最长等待 10 分钟，卡住时不再拖住整个任务

  Clearer when something goes wrong: rate limits, network failures, an overflowing context, a rejected key and service outages are explained with what to do next; stopping a task no longer shows an English notice; the failure banner and permission card buttons work from the keyboard; outside a Git repository the app explains why changes can't be reviewed or undone; and an MCP tool that hangs is abandoned after ten minutes instead of holding up the whole task

## [0.2.15]

- 新增「云端 API 节点」：登录后可在主站和 CDN 加速节点（中国及亚太地区可尝试）之间切换，后台配置的其他节点也会自动出现；聊天输入框上方的节点标签和侧边栏账户菜单都能一键切换，并显示测速延迟。只有模型请求走所选节点，登录、账户和支付始终使用主站；所选节点暂时连不上时会自动切到可用节点，恢复后再切回来

  Adds cloud API nodes: once signed in you can switch between the main site and a CDN node (worth trying from mainland China and the rest of Asia-Pacific), and any other node the service lists appears on its own; the node chip above the composer and the sidebar account menu switch in one click and show the measured latency. Only model requests use the chosen node — signing in, your account and payments always use the main site — and when the chosen node stops answering the app moves to one that works, then back once it recovers

- 内置 Agent 长任务更稳定：网络中断、限流、服务繁忙或回复中途断开时会自动等待后重试（工作行显示「重新连接中」和倒计时），断开前已显示的半截回复会被撤回再重新生成，不再出现重复内容；超过 10 分钟的长回复不再被掐断；对话超出上下文窗口时会先自动压缩再继续；余额不足、额度用完等无法靠重试解决的错误会立即提示，不再白白等待

  Makes long built-in agent tasks far more reliable: network drops, rate limits, busy servers and replies cut off mid-stream are now retried automatically after a short wait (the working row shows "Reconnecting" with a countdown), the half reply shown before a drop is taken back before it is generated again, so nothing appears twice; replies longer than ten minutes are no longer cut off; a conversation that outgrows the context window is compacted and carries on; and errors no retry can fix, such as an empty balance or a spent quota, are reported at once instead of after a wait

- 内置 Agent 的任务失败后，如果已经完成过一些步骤，错误提示旁会多一个「继续」按钮，保留已完成的工作从中断处接着做；目标模式遇到网络等临时故障会在稍后自动继续，团队成员遇到临时故障也会一直重试直到恢复

  When a built-in agent task fails after getting some work done, the error banner now offers "Continue", which picks up where it stopped and keeps the finished work; goal mode resumes on its own after a temporary failure such as a network outage, and team members keep retrying until the failure clears

## [0.2.13]

- 修复内置 Agent 使用 GPT 模型时提示词缓存经常失效、费用偏高的问题：同一个任务的请求现在都会带上会话标识，始终交给同一个上游账号处理并复用缓存；长期记忆在一个任务里只读取一次，任务进行中新写入的记忆不会再让之前的缓存全部作废，下一个任务会读到最新的记忆；待办清单的进度提醒和目标模式的进度不再写进系统提示词，每完成一项待办不会再让整段对话的缓存失效（Claude 和国产模型同样受益）；GPT 的系统提示词也不再重复发送，每次请求少约 2000 个 token

  Fixes the built-in agent's prompt cache often missing on GPT models, which made long tasks cost more: every request of a task now carries the conversation's identity, so the task stays on one upstream account and reuses its cache; long-term memory is read once per task, so a memory saved mid-task no longer throws the cache away — the next task picks the new memory up; the to-do reminder and goal progress no longer live in the system prompt, so finishing a to-do no longer invalidates the cache for the whole conversation (on Claude and the Chinese models too); and GPT no longer receives the system prompt twice, saving about 2,000 tokens per request

- 修复内置 Agent 的子 Agent 出错或被停止后，说「继续」会把它做过的事从头再做一遍的问题：子 Agent 没做完时，主 Agent 现在能看到它已经做了什么——改过哪些文件、调用过哪些工具、哪一步失败了、最后写到哪里——并从中断处接着做；按停止时已经完成的子 Agent，它的结果也不会再丢失

  Fixes "continue" redoing everything a built-in agent's sub-agent had done after it failed or was stopped: when a sub-agent does not finish, the main agent now sees what it got done — the files it changed, the tools it called, the step that failed and the last thing it wrote — and carries on from there; a sub-agent that had already finished when you pressed Stop no longer loses its report either

- 修复画图页的模型列表有时只剩 gpt-image-2 的问题：模型目录还在加载时会显示「正在加载模型」，加载失败时显示原因并可点击重试，失败后也会很快自动重试，不再把默认模型当作全部可用模型；列表没加载好之前不能提交，避免用错模型

  Fixes the image studio's model list sometimes showing only gpt-image-2: while the model catalog loads the picker says so, a failed load shows why with a retry and is retried automatically within seconds, instead of passing the default model off as the whole list; drawing waits for the list, so a picture is never drawn with a model you did not pick

- 修复内置 Agent 使用 GPT 模型时，读取、搜索等工具调用报「Invalid input: missing field」而全部执行失败的问题：工具参数一次性整段返回时也能正确读到了

  Fixes the built-in agent's tool calls on GPT models all failing with "Invalid input: missing field": a call whose arguments arrive in one piece rather than streamed is now read correctly

## [0.2.12]

- 内置 Agent 新增长期记忆：它会把值得长期保留的信息（你的身份和偏好、你希望的工作方式、项目进展、资料在哪里）各存成一条记忆，按工作区和全局分开保存，下次会话自动带上，不用每次重新交代。你也可以直接说「记住……」或「忘掉……」。默认每聊 12 条消息以及会话结束时自动提炼新记忆；项目记忆会同步到 Claude Code 的项目记忆里，同一目录下的 Claude Code 也能看到，Claude Code 自己写的记忆不会被改动。批量删除记忆一定会先问你，完全访问模式下也一样。在「设置 → Agent → 记忆」里可以开关这些功能、调整占用的上下文大小，并查看、编辑、置顶或删除每一条记忆

  The built-in agent now has long-term memory. It saves what is worth keeping — who you are and what you prefer, how you want the work done, where a project stands, where things live — as one memory each, per workspace and globally, and brings them into later sessions so you do not have to repeat yourself. You can also just say "remember…" or "forget…". By default it summarizes new memories every 12 messages and when a session ends, and project memories are synced into Claude Code's project memory so Claude Code in the same folder sees them too, without touching the memories Claude Code wrote itself. Deleting memories in bulk always asks you first, even in full access. Settings → Agent → Memory turns each of these on or off, sets how much context the memories may take, and lets you read, edit, pin or delete every memory

- 内置 Agent 联网搜索改为通过云账号的网关搜索（与 Codex 使用同一搜索服务），搜到的内容会附上来源链接，也可以限定网站或只看最近几天的结果；以前在没有另行配置搜索服务时只能查到很少的内容。使用自己的接口或未登录时仍用原来的搜索方式

  The built-in agent's web search now goes through the cloud account's gateway — the same search Codex uses — and lists its sources, with optional site and recency limits. It used to find very little unless a search service had been set up separately. With your own endpoint or when signed out, it keeps the old search

- 左侧任务栏改版：顶部可以切换「按项目 / 时间线」（默认按项目），并一键展开或收起全部分组；按项目查看时每个项目先显示 5 个任务，没有项目的任务单独列在「任务」里。任务改为单行显示，失败、未读、运行中的状态显示在标题左侧，等待你确认时右侧显示「等待确认」，悬停可以直接置顶或归档，项目和分支移到标题的悬停提示里；运行中的任务排在最前，项目前有展开箭头，收起的分组里有任务在运行或有未读回复时也会提示，任务开始运行时它所在的项目会自动展开。新增跨项目的「已置顶」分区；删除任务改为归档，可以在归档视图里取消归档或永久删除；在别的任务上完成的回复会显示未读圆点，也可以右键「标记为未读」

  The task sidebar is redesigned: switch between By project (the default) and Timeline at the top, and expand or collapse every group at once. By project shows five tasks per project at a time, with tasks that belong to no project listed under Tasks. Tasks are single-line rows: failed, unread and running states sit left of the title, a "Needs approval" tag shows on the right while a task waits on you, hovering offers pin and archive, and the project and branch move into the title's tooltip. Running tasks sort first, and projects carry a disclosure chevron, a folded group still shows when a task inside is running or has an unread reply, and a project opens by itself when one of its tasks starts running. A Pinned section gathers tasks across projects; removing a task now archives it, and the archive view can restore or permanently delete it. A reply that finishes in another task shows an unread dot, and any task can be marked unread from its menu

- 修复 Windows 上安装电脑操作驱动时偶尔报"下载成功但文件不在"的问题：下载改放在应用自己的目录里，每次安装单独一个文件夹，文件下载后消失会自动重新下载，最多三次；仍然失败时，报错会写明下载收到了什么、文件夹里还剩什么

  Fixes installing the Computer Use driver on Windows occasionally failing with "reported success but nothing was there": downloads now land in a folder of their own inside the app's directory, and a file that vanishes after downloading is fetched again, up to three times. If it still fails, the message says what the download received and what was left in the folder

- 修复内置 Agent 使用 GPT 等模型时，两行工具或思考状态之间偶尔出现一大段空白、本该归在一起的操作被拆成两组的问题

  Fixes a large blank gap that sometimes appeared between two tool or thinking rows when the built-in agent ran GPT and other models, splitting work that belongs together into separate groups

- 修复删除任务或回退对话后，新建任务欢迎页的活动概览跟着减少的问题：活动记录现在单独保存，只增不减；分叉出的任务也不再把原任务的消息重复计入。修复前已经删除的任务无法找回

  The activity overview on the new-task welcome screen no longer shrinks when a task is deleted or a conversation is rewound: activity is now kept on its own record that only grows, and a forked task no longer counts the messages it copied from the original a second time. Tasks deleted before this fix cannot be recovered

## [0.2.11]

- 内置 Agent 新增智能体团队，取代原来的「设置 → 工作流」：在任务里发送 `/agent-teams <目标>`（或用 `/agent-teams-<模板名> <目标>` 从团队模板开始），当前会话就成为队长，拟定成员（每人可以用不同的模型和推理强度）和带依赖关系的任务；右侧「团队」面板显示草案，可以确认启动、返回对话修改或放弃。启动后成员自动领取可以开始的任务、相互发消息协作，每个任务完成前都要通过检查，不合格时会自动安排修复和复审；成员需要权限时在队长的任务里弹出确认。面板显示进度、成员和任务关系图，点成员可以查看它的实时记录，停止团队需要再点一次确认；应用重启后再打开这个任务，团队会从停下的地方接着做。成员上限、默认模型、执行提示词和团队模板在「设置 → Agent → 团队」里调整

  The built-in agent can now run a team, replacing Settings → Workflow: send `/agent-teams <goal>` in a task (or `/agent-teams-<template> <goal>` to start from a team template) and the task becomes the captain, drafting members — each on its own model and reasoning effort — and tasks with their dependencies. The Team panel on the right shows the draft to approve, take back to the chat for changes, or discard. Once started, members pick up tasks as they become ready and message each other; every task passes a check before it counts as done, with a fix and a second review arranged when it falls short, and a member's permission request appears in the captain's task. The panel shows progress, members and the task graph, opens a member's live record on click, and stops the team on a second click; after a restart, opening the task again picks the team up where it left off. Member limit, default model, working instructions and team templates live in Settings → Agent → Teams

- 计划模式下 Agent 规划完成后，计划会在右侧「计划」面板完整显示，输入框上方只留一行「批准并执行 / 继续规划」：可以选中计划里的文字引用到意见里再退回，也可以翻看每一版计划，与上一版逐行对比；顶部栏的「计划」按钮随时打开或收起面板，有计划等你审批时会亮起提示。内置 Agent 会把完整计划交给你审批，不再只给一句摘要，并且在写计划之前就把问题问清楚，交上来的计划里不再夹带要你确认的问题

  In plan mode, a finished plan opens in full in a Plan panel on the right, and the card above the composer keeps one row with Approve and Keep planning: select part of the plan to quote it in your notes and send it back, or step through every version and compare each with the one before, line by line. The Plan button in the header shows or hides the panel at any time and lights up while a plan waits for you. The built-in agent hands over its whole plan for approval instead of a one-line summary, and settles its questions before writing the plan, so the plan no longer asks you anything

- 内置 Agent 在计划模式下不再频繁拒绝查看类命令：带引号的搜索、`2>&1` 和 `>/dev/null`、`cd 目录 && …`、`sed -n`、`awk`、`git branch`、`git -C` 等只读命令都能直接运行；写文件、安装、构建和运行测试仍要等计划批准后再做。同时修复在权限确认里对命令选过「总是允许」后，计划模式会放行所有命令和文件修改的问题

  Plan mode on the built-in agent stops refusing commands that only read: quoted searches, `2>&1` and `>/dev/null`, `cd dir && …`, `sed -n`, `awk`, `git branch`, `git -C` and the like now run, while writing files, installing, building and running tests still wait until the plan is approved. Also fixes plan mode letting every command and file edit through once commands had been set to "Always allow" in a permission prompt

- 输入框的推理强度改为 Claude Code 风格的 Effort 滑杆卡片：拖动后松手吸附到最近的档位，也可以用方向键、Home、End 调节；档位名按强度着色，档位越高轨道上的火焰越旺，系统开启「减少动态效果」时火焰静止；服务等级 / 接口格式和上下文窗口仍在同一张卡片里。同时修复换了档却不生效的几种情况：Claude Code 会在你发下一条消息时以新强度接着原会话继续，不打断正在进行的回合；Kimi 只改强度也会立即应用；自定义接口声明的 off 档真正关闭思考，不再在 DeepSeek、GLM 上开启高强度思考

  The composer's reasoning control is now a Claude Code–style Effort slider card: drag it and it snaps to the nearest level on release, or use the arrow keys, Home and End; the level's name is colored by intensity and the flame along the track grows with it, holding still when the system asks for reduced motion; service tier / API and context window stay in the same card. Also fixes efforts that were picked but never applied: Claude Code picks up a new effort with your next message and carries on the same conversation without interrupting a running turn, changing only Kimi's effort now applies it, and an `off` level declared on your own endpoint really turns thinking off instead of turning high thinking on for DeepSeek and GLM

- 文件面板支持右键菜单：打开、用默认应用打开、在文件管理器中显示、在消息中引用、复制路径或相对路径、新建文件或文件夹、就地重命名、移到回收站、刷新；也可以用键盘操作（方向键、Home/End、回车、F2 重命名、Delete 删除、Shift+F10 打开菜单），面板顶部新增新建与刷新按钮。远程工作区同样可以新建、重命名和删除。文件面板还能预览常见格式：图片（PNG、JPEG、GIF、WebP、BMP、ICO、SVG、TIFF，可在适应面板和原始大小之间切换）、表格（Excel、WPS/OpenDocument 表格、CSV、TSV，按工作表分页显示前 1000 行，带列标和行号）、Word 和 PowerPoint 的文字内容；PDF、压缩包、音视频等显示文件大小，并可一键用默认应用打开

  The Files panel has a context menu: open, open with the default app, reveal in the file manager, mention in a message, copy the path or the relative path, new file or folder, rename in place, move to the trash and refresh; rows also work from the keyboard (arrows, Home/End, Enter, F2 to rename, Delete, Shift+F10 for the menu), and the panel header gains new-file, new-folder and refresh buttons. Creating, renaming and deleting work on remote workspaces too. The Files panel also previews common formats: pictures (PNG, JPEG, GIF, WebP, BMP, ICO, SVG, TIFF, fitted to the panel or at their own size), spreadsheets (Excel, OpenDocument, CSV, TSV — each sheet's first 1,000 rows on its own tab, with column letters and row numbers), and the text of Word and PowerPoint files; a PDF, an archive, audio or video shows its size and opens with the default app in one click

- 内置 Agent 画图更好用：不开启电脑操控也能画图；Agent 会把画好的图片直接显示在回复里，回复里引用本机图片路径的图片现在能正常显示；画图失败时的提示更直接；画图那一行显示画的内容，JavaScript 调用的那一行会显示执行的代码

  Drawing with the built-in agent works better: it can draw without Computer Use switched on, shows the pictures it made right in its reply, and images in a reply that point at a file on this computer now load; a failed drawing says plainly why, a drawing's row shows what was asked for, and JavaScript calls show their code

- 内置 Agent 可以操控应用内置浏览器（Windows）：打开网页、读取页面、点击、输入、选择、按键、执行脚本、等待内容出现、查看控制台和截图，适合对本地开发服务或测试站点做自动化测试。浏览器会在当前任务的右侧面板中打开；点击、输入等操作和执行命令一样按权限设置询问

  The built-in agent can drive the app's in-app browser (Windows): open pages, read them, click, type, choose options, press keys, run scripts, wait for content, read the console and take screenshots — for automated testing of a local dev server or a staging site. The browser opens in the current task's right panel; clicking, typing and the like ask for permission the way running a command does

- 管理员在后台填写的内容会按界面语言显示：分组名称和说明、公告、充值页的套餐、活动与帮助文字有翻译时显示译文；翻译保存在本机，下次启动直接可用。只影响显示，路由和分组的判断仍按原文

  Text written by the site's administrators — group names and descriptions, announcements, and the plans, promotions and help text on the top-up page — shows in the interface language when a translation exists, and translations are kept on this machine for the next launch. Display only: routing and group choice still go by the original text

- 内置 Agent 使用 GPT、Grok、DeepSeek 等模型时，工具用得更规范：读文件、搜索和编辑用专门的工具而不是 `cat`、`grep`，编辑时不再因为带上行号而失败；提交代码时不会用 `git add -A` 把 `.env` 之类的文件一起提交，提交钩子失败后也不会去改写上一次提交；只在值得并行或独立处理时才启动子 Agent，子 Agent 结束后的汇报也更完整

  With GPT, Grok, DeepSeek and other models, the built-in agent uses its tools the way Claude Code does: it reads, searches and edits with the dedicated tools rather than `cat` and `grep`, no longer fails edits by copying line numbers into them, never sweeps files such as `.env` into a commit with `git add -A` or rewrites the previous commit after a failed hook, and starts sub-agents only for work worth running in parallel or on its own, with fuller reports when they finish

- 修复在 HTTP 代理后面（例如设置了 `HTTPS_PROXY` 环境变量）使用时，云账号登录、登录码兑换及其它服务请求失败并提示「could not parse response body: HTTP/1.1 200 OK …」的问题：代理对 CONNECT 的应答曾被当成真正的响应头

  Fix cloud sign-in, login-code exchange and every other service request failing with "could not parse response body: HTTP/1.1 200 OK …" behind an HTTP proxy (for example with `HTTPS_PROXY` set): the proxy's reply to CONNECT was taken for the real response headers

## [0.2.10]

- 新建任务的欢迎页新增活动概览：会话数、消息数、活跃天数、高峰时段、最常用模型和最长连续天数，可在全部 / 30 天 / 7 天之间切换；下方是近半年每天的消息热力图（悬停显示当天条数），「模型」页按消息占比给模型排行。数据来自本机的任务历史

  The new-task welcome screen gains an activity overview: sessions, messages, active days, peak hour, favorite model and longest streak over All / 30 days / 7 days, a heat map of messages per day for the last six months (hover for a day's count), and a Models tab ranking models by their share of messages. It is counted from this machine's task history

- 任务运行时，「工作中」那一行会说明此刻在做什么：思考中、输出中、运行工具中、等待模型或等待你的操作；使用内置 Agent 时还会显示本轮已输出的 token 数

  While a task runs, the working line says what is happening right now — thinking, writing, running tools, waiting for the model or waiting for you — and, with the built-in agent, how many tokens the turn has produced

- 模型选择器里每个模型下多了一句简短说明，介绍这个模型适合做什么

  Each model in the model picker carries a one-line description of what it is for

- 登录云账号时可以选择哪些 CLI 交给云账号配置：检测到 Claude Code、Codex 等已经在用你自己的账号、API Key 或其他中转时会先询问；保留的 CLI 配置文件保持原样，继续用你自己的登录，内置 Agent 始终使用云账号。之后可在「设置 → CheapRouter 账号」里逐个切换

  At sign-in you choose which CLIs the cloud account configures: when Claude Code, Codex or another CLI already runs on your own account, API key or relay, CheapRouter asks first, and a CLI you keep stays untouched on your own sign-in; the built-in agent always uses the cloud account. Switch each CLI later in Settings → CheapRouter Account

- 输入框下方的上下文用量环在智能体还没上报窗口大小时，也会按公开模型列表里的上下文窗口显示百分比；切换模型后立即按新模型的窗口计算，不再沿用上一个模型的数值

  The context ring below the composer shows a percentage even before the agent reports its window, using the model's context window from a public model list, and switching models re-measures against the new model at once instead of keeping the previous model's figure

## [0.2.9]

- 侧边栏「画图」下方新增「模型运行状态」页，内容与网页版一致：监测分组、运行正常、响应变慢、异常分组四项统计，每个分组一张卡片，显示状态、指纹检测结果、最近 24 次检测、最近结果、首字延迟和 24 小时 / 7 天可用率；「查看详情」可看历史趋势和最近事件。页面打开时每 30 秒自动刷新

  A Model status page under Images in the sidebar, matching the web console: four summary figures (monitored, healthy, degraded, down) and a card per group with its status, fingerprint checks, last 24 probes, latest result, first-token latency and 24-hour / 7-day availability; View details shows the history and recent events. The page refreshes every 30 seconds while open

- 充值时与网页同步充值活动：快捷金额里会出现活动门槛并标出赠送金额，输入金额时提示「再充多少可享哪个活动」，付款前显示活动赠送和实际到账；支付中的订单和支付成功提示也会写明赠送了多少。后台配置的帮助文字和图片（如客服二维码，可点开放大）以及每日到账上限也会显示在充值页

  Top-ups follow the web page's promotions: promotion thresholds join the quick amounts with the bonus each earns, a hint says how much more to top up for which promotion, and the bonus and total credited show before paying, on the pending order and in the success message. The administrator's help text and picture (such as a support QR code, which opens full size) and the daily credit cap appear on the top-up sheet too

- 名称里带「国模」的分组和套餐（如新上架的「国模中级套餐」）归到「国模」一栏，不再出现在 Codex 下——以前还没购买或尚未列出模型的这类分组会被当成 Codex 的，选中后 GPT 请求会发错分组

  Groups and plans named for the Chinese models (such as the new 国模中级套餐) are filed under the Chinese models instead of Codex. Before, such a group that was not yet bought or listed no models read as a Codex group, and picking it sent GPT requests to the wrong group

## [0.2.8]

- 自己的接口直接出现在模型选择器里：在「设置 → 模型接口」添加接口和模型后，内置 Agent 的模型选择器里会多出以这个接口命名的一栏，不用再去「设置 → Agent」绑定。Anthropic Messages、Responses、Chat Completions 三种格式的接口都算，模型按接口声明的格式发送，不会因为名字被改走别的格式——以前只有绑定到 Chat Completions 的接口才列得出模型。模型接口页的每个模型都有「测试」按钮，页面会显示请求实际发往的地址，以及这个接口是否已出现在选择器里；选择器里这些接口那一栏的末尾有「管理模型…」

  Your own endpoints show up in the model picker: add an endpoint and its models in Settings → Model providers and the built-in agent's picker gains a section named after it — no binding in Settings → Agent needed, whichever of the three API formats it speaks, and each model goes out in the endpoint's format whatever its name. Before, only an endpoint bound to Chat Completions listed its models. Each model on Model providers has a Test button, the page shows the exact URL requests go to and whether the endpoint is in the picker, and the picker's endpoint sections end in "Manage models…"

- 地址末尾自带版本号的接口（如智谱 `…/api/paas/v4`、火山方舟 `…/api/v3`）不再被多拼一个 `/v1` 而返回 404；请求返回 404 时，错误信息会写明模型、接口类型和完整的请求地址，不再只有 "Model not found: unknown"

  Endpoints whose address already ends in a version (Zhipu's `…/api/paas/v4`, Volcengine Ark's `…/api/v3`) no longer get an extra `/v1` and a 404; a 404 now names the model, the API and the full request URL instead of just "Model not found: unknown"

- 新增 Claude Sonnet 5.5 和 GPT-6.1 Sol：Claude Code 和 Codex 的模型列表里都能选到，网关提供 Claude Sonnet 5.5 时内置 Agent 默认使用它

  Added Claude Sonnet 5.5 and GPT-6.1 Sol: both are in the Claude Code and Codex model lists, and the built-in agent starts on Claude Sonnet 5.5 when the gateway serves it

- 内置 Agent 一次派出的多个子智能体现在同时运行——用 GPT、Grok 和国产模型时它们以前是一个接一个跑的；某个子智能体等你批准操作时，其他子智能体也不会再被卡住

  Sub-agents the built-in agent starts together now run at the same time — with GPT, Grok and the Chinese models they used to run one after another — and one waiting for your approval no longer holds up the others

- 设置 → 内置 Agent 里的 MCP 服务器移到了端点下方，不用再翻过长长的工具列表才能找到；以 `/sse` 结尾的服务器地址现在按 SSE 方式连接，以前这类服务器连不上

  Settings → Agent now shows MCP servers right under the endpoints instead of below the long tool list; a server address ending in `/sse` now connects over SSE, where before such servers failed to connect

## [0.2.7]

- 子智能体看得见了：内置 Agent 和 Claude Code 派出的子智能体在对话里显示为一行摘要（智能体类型、任务，运行中有动画），点击即在右侧面板实时查看它的每一步——写了什么、调用了哪些工具，工具行可展开看参数和输出，运行中可随时停止。内置 Agent 的子智能体现在和主会话用同一个网关、同一套 MCP 工具和设置；后台子智能体只出现在启动它的会话里，也不会再拖住当前这一轮的结束

  Sub-agents you can follow: a sub-agent started by the built-in agent or Claude Code shows as one line in the conversation (its kind and its task, animated while it runs); click it to watch every step live in the side panel — what it wrote and the tools it called, each tool row expandable to its input and output — and stop it at any time. The built-in agent's sub-agents now use the same gateway, MCP tools and settings as the conversation that started them; a background sub-agent appears only in its own conversation and no longer holds up the end of the turn

- 内置 Agent 现在知道网关上每个模型的上下文窗口（自定义端点里填写的窗口也算），用量环显示百分比，自动压缩按模型真实的窗口触发

  The built-in agent now knows the context window of every model the gateway serves (and the ones you enter for your own endpoints): the usage ring shows a percentage, and auto-compaction fires at the model's real window

- 新用户默认使用内置 Agent 和它的默认模型，不再默认选中需要另外安装的 Codex CLI；之前装过但从未选过的，也会自动切换过去

  New users start on the built-in agent and its default model instead of the Codex CLI, which needs a separate install; if you installed before and never picked one, you're moved over too

## [0.2.6]

- 登录更稳：浏览器登录后跳不回客户端（比如打不开 127.0.0.1）时，登录页会显示一个一次性登录码，把它粘贴到客户端的登录窗口即可完成登录，也可以粘贴地址栏里的整条链接。登录窗口在点击登录时弹出，可以重新打开浏览器、复制登录链接到其他设备上登录，或取消登录；关掉窗口不会中断登录，点左下角可以重新打开。登录码 10 分钟内有效、只能用一次，离开发起登录的这台客户端就无法使用

  Sturdier sign-in: when the browser can't get back to the app after you sign in (for example, it can't open 127.0.0.1), the sign-in page shows a one-time code — paste it into the app's sign-in window to finish, or paste the whole link from the address bar. The window opens when you click Sign in and lets you open the browser again, copy the sign-in link to finish on another device, or cancel; closing it doesn't stop the sign-in, and the footer brings it back. A code is valid for 10 minutes, works once, and only in the app that started the sign-in

- 修复退出登录后左下角又显示回账号、重启后又自动登录的问题。退出登录现在立即生效，并会在服务端注销这次登录；退出后马上换个账号登录，也不会混入上一个账号的数据

  Fixed signing out not sticking: the footer no longer shows the account again a moment later, and a restart no longer signs you back in. Signing out now takes effect at once and also ends the session on the service; signing straight in as someone else no longer picks up the previous account's data

## [0.2.5]

- 国模有了自己的分组：设置 → 云账号和账户菜单新增「国模」一栏，在国模订阅和国模按量付费分组之间手动选择；选订阅时额度用完自动改走按量付费，额度恢复后切回。国模分组不再出现在 Codex 的分组里，此前把 Codex 切到国模分组后 GPT 请求报错的问题随之解决——更新后 Codex 会自动换回 Codex 分组，原来选的国模分组移到「国模」一栏。模型选择里的「按量付费」一项随之去掉，走哪个分组由「国模」一栏决定，模型副标题会写明；选过那一项的对话自动沿用，无需重选

  Chinese models get a group of their own: Settings → Cloud Account and the account menu gain a "Chinese models" lane where you pick their subscription or pay-as-you-go group; with the subscription picked they move to pay-as-you-go once its limit is used up, and back once it resets. Their groups no longer appear among Codex's, which fixes GPT failing after Codex was pointed at one — after the update Codex moves back to a Codex group on its own, and the group it held becomes the Chinese models' pick. The model picker's "Pay as you go" entry is gone with it: the lane decides, and each model's subtitle names the group it goes through. Conversations on that entry carry on without being picked again

- 刚登录就用国模发消息，不再报「unknown model」要点重试：发送前会先为这个模型准备好所在分组的密钥（账户里已有就直接复用），短暂提示「正在准备模型路由…」后自动发出

  A Chinese model used right after signing in no longer answers "unknown model" until retried: the message waits a moment ("Getting this model's route ready…") while the key for its group is found — or made, when the account has none there — and then goes out on its own

- 内置 Agent 能看图了：粘贴、拖入或选择的图片会随消息一起发给模型，所有模型都一样（此前模型只收到图片的文件路径，GPT 更是一张图都看不到）；不支持图片的模型会由接口直接报错说明

  The built-in agent can see pictures: images you paste, drop or pick go to the model with your message, whatever the model (before, it only got the file's path, and GPT never saw a picture at all). A model that cannot read images says so in the API's error

## [0.2.3]

- 国模可以按量付费：订阅和按量付费分组都提供的模型（DeepSeek、智谱 GLM、Kimi 等），在内置 Agent 的模型选择里多出一项「按量付费」，选它就按余额计费。订阅那一项仍优先使用订阅额度，额度用完或订阅过期后自动改走按量付费，下一轮对话即生效，额度恢复后再切回订阅

  Chinese models can be paid by use: a model both a subscription and a pay-as-you-go group serve (DeepSeek, Zhipu GLM, Kimi and the like) gets a second "Pay as you go" entry in the built-in agent's model picker, billed to your balance. The subscription entry still spends the subscription first, and once its limit is used up or it lapses it moves to pay-as-you-go on its own from the next turn, returning to the subscription when the limit resets

- 内置 Agent 重新显示模型的思考过程：Claude、GPT 以及 DeepSeek、GLM、Kimi 的推理内容都会出现在对话里

  The built-in agent shows the model's reasoning again: Claude's, GPT's and DeepSeek's, GLM's and Kimi's thinking all appear in the conversation

- 没有手动选过推理强度的对话，现在按模型默认的强度发送，与菜单上显示的一致——此前 Claude 不会思考，GPT 固定用「中」；GPT-6 系列也能收到所选的推理强度；对话中途切换模型不再沿用上一个模型的推理强度

  A conversation that never picked a reasoning effort now sends the model's default, the one the menu shows — before, Claude did not think at all and GPT always ran at medium. The GPT-6 family receives the chosen effort too, and switching models mid-conversation no longer carries the previous model's effort over

## [0.2.2]

- 设置 → 套餐：浏览在售套餐，直接在应用内购买或续费。价格、原价、有效期、每日/每周/每月额度、适用的 CLI 和模型一目了然，已持有的套餐标出到期时间和用量。用支付宝或微信扫码付款，付款后套餐自动开通——订阅分组、密钥和模型路由随即就绪；如果套餐对应的 Claude Code 或 Codex 还在走别的分组，一键切换过去

  Settings → Plans: browse the plans on sale and buy or renew one in the app. Price, original price, term, daily/weekly/monthly caps and the CLI and models each plan serves are shown at a glance, and the plans you hold are marked with their expiry and spend. Pay by scanning an Alipay or WeChat code; once paid, the plan activates on its own — its subscription group, key and model routing are ready at once, and if the plan's Claude Code or Codex is still routed through another group, one click moves it over

- 因余额不足、套餐额度用完或订阅过期而失败的对话，错误横幅直接给出「充值」「升级套餐」或「续费套餐」按钮。账户菜单新增「购买套餐」，订阅卡片可一键续费，还没有订阅时会提示去看套餐，充值窗口也能直达套餐页

  A turn that failed because the balance ran out, a plan's limit is spent or the subscription lapsed gets a Top up, Upgrade plan or Renew plan button right in its error banner. The account menu gains "Buy a plan", subscription cards renew in one click, an account without a subscription is pointed at the plans, and the top-up sheet links to them too

- 支付更稳：登录凭证快过期时支付窗口会先续期，不再因「无效的 token」失败；微信直连在电脑上显示二维码，不再报「缺少支付链接」；支付宝直连的二维码是完整地址，手机可以正常扫码

  Payments are sturdier: the pay sheet renews a session that is about to expire instead of failing with "invalid token"; WeChat direct shows its QR code on a PC instead of reporting a missing payment link; and the Alipay direct QR code carries the full address, so phones can scan it

- 内置 Agent 的模型选择器按厂商分栏：Anthropic、OpenAI、Google、xAI、DeepSeek、智谱 GLM、Kimi、MiniMax、通义千问各占一栏，带标识和模型数，自定义端点的模型单列在「自定义」下；Tab / Shift+Tab 在厂商之间切换

  The built-in agent's model picker files models by vendor: Anthropic, OpenAI, Google, xAI, DeepSeek, Zhipu GLM, Kimi, MiniMax and Qwen each get a column with their mark and model count, and your own endpoint's models sit under "Custom"; Tab / Shift+Tab steps through the vendors

## [0.2.1]

- 内置 Agent 在 Claude 路由上启用提示缓存：它像 Claude Code 一样标记请求，因此在直连 Anthropic 的分组（如 Claude Max）上，每一步都从缓存读取此前的对话，不必再次全价付费

  The built-in agent is prompt-cached on Claude routes: it marks its requests the way Claude Code does, so on a group that goes straight to Anthropic (such as Claude Max) each step reads the conversation so far from the cache instead of paying for all of it again

- 为新版 Claude 模型（Opus 4.6 及以后、Sonnet 4.6 与 5、Fable 5）选择的推理强度，现在按 Claude Code 的方式送达，并会显示在网关的用量记录里；选择推理强度后 Opus 5.5 不再报错

  The reasoning effort chosen for a current Claude model (Opus 4.6 and later, Sonnet 4.6 and 5, Fable 5) now reaches it the way Claude Code sends it, and shows in the gateway's usage log; Opus 5.5 no longer fails when an effort is chosen

- 画图，位于侧栏「搜索」下方：描述一幅画来生成，或添加图片（拖入、粘贴或选择）来编辑。可选模型、尺寸（附价格）、质量和张数，预估费用和余额就在「生成」按钮旁。图片保存到「图片」文件夹并进入图库，每张都能预览、用作参考图、再画一次、发送到任务或在磁盘上显示。耗时长的图片作为网关任务运行，关闭窗口也不中断；没有画图权限的分组会被自动跳过，能画的分组会被记住，Agent 的 `generate_image` 也会使用它

  Images, under Search in the sidebar: describe a picture and draw it, or add pictures — drop, paste or pick them — to edit them. Pick the model, size (with its price), quality and how many; the estimate and your balance sit beside the Draw button. Pictures go to your Pictures folder and into a gallery where each one can be previewed, used as a reference, drawn again, sent to a task or revealed on disk. Long pictures run as gateway tasks that survive closing the window, and a group without image access is skipped automatically — the one that draws is remembered, and the agents' `generate_image` uses it too

- Agent 的工作过程实时可读：浏览代码的操作归并成一行（「已探索 · 2 次搜索、3 个文件」），命令归并成另一行，每一步都说明正在做或已完成什么，失败的命令标红，悬停可看输出末尾

  The agent's work reads as it happens: reading around the code gathers into one line ("Explored · 2 searches, 3 files"), commands into another, each step says what it is doing or did, and a failed command is marked in red with the end of its output on hover

- 思考过程只占一行：模型思考时显示最新的一句，结束后显示思考了多久，不再是一个自己展开又收起的框

  Thinking stays one line — the newest sentence while the model thinks, how long it thought once it is done — instead of a box that opened and closed on its own

- 为引导运行中任务而发送的消息会立即显示，每条消息前后的工作各自折叠，并显示各自的用时

  Messages sent to steer a running task show at once, and the work before and after each one folds on its own, with its own time

- 「工作时长」不再计入等待你批准或回答的时间

  "Worked for" no longer counts time spent waiting for your approval or answer

- 失败的对话会在输入框上方以横幅说明——不再有编造的回复——并提供完整错误、复制和一键重试（重试不会删除任何文件）

  A failed turn says so in a banner above the composer — no more made-up replies — with the full error, copy and one-click retry (which never deletes files)

- 可在某一轮的卡片上撤销该轮的文件改动：之后又被改过的文件保持不动并列出，文件夹里的其他内容不受影响。点击卡片上的文件可打开它的差异

  Undo one turn's file changes from its card: files changed again since are left alone and listed, and nothing else in the folder is touched. Click a file on the card to open its diff

- 对话记录右上角新增状态胶囊：显示目标、当前步骤、计划进度、后台任务和最近的改动

  A status capsule at the top right of the transcript: the goal, the current step, progress through the plan, background work and the latest changes

- 多个待批准请求排队并显示计数，按数字键即可选择答复，Esc 不再中止整个任务，「拒绝并说明」可以告诉 Agent 该怎么做。待批准的计划以排版后的文本显示

  Several approval requests queue with a counter, a digit key picks an answer, Escape no longer stops the whole task, and "Deny and explain" tells the agent what to do instead. Plans to approve read as formatted text

- 快捷键：⇧⌘M 切换访问模式，⇧⌘P 开关计划模式，⌘T 切换推理强度（Windows 上用 Ctrl）

  Shortcuts: ⇧⌘M cycles the access mode, ⇧⌘P toggles plan mode, ⌘T cycles reasoning effort (Ctrl on Windows)

- 应用在后台时，任务需要你批准或回答会发出通知，侧栏会显示有多少请求在等待

  A notification when a task needs your approval or answer while the app is in the background, and the sidebar counts how many requests wait


## [0.2.0](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.2.0) — 2026-11-17

This is the first stable release of Sailry's Go runtime, shared Client and Link
layers, and rebuilt Desktop and Mobile controllers.

### Changes

- Organize local and directly connected hosts through the initiating
  controller's unified resource directory. Keep projects, files, history and
  execution on their owning Node.
- Persist remote project, worktree and knowledge directory summaries for offline
  display and cold-start recovery. Preserve drafts and selection across reconnects;
  clear restricted projections after access revocation or connection removal.
- Keep credential renewal on the controller and share only execution credentials.
  Add explicit same-account login recovery for waiting tasks without replaying
  completed tools or restarting stopped work.
- Synchronize personal configuration and plugin KV with exact source identities,
  durable write groups and revision checks. Preserve captured credentials and
  package versions for admitted tasks.
- Select the highest compatible plugin release on the execution Node. Add
  official version history, per-version release notes, compatible version
  switching and version pinning. Exclude withdrawn and unsupported releases
  from installation and ordinary update prompts.
- Preserve official plugin update sources and immutable published version
  mappings. Retain installed settings and enabled choices, and reject conflicting
  replacements without overwriting concurrent work.
- Load document images through their captured host and worktree file routes on
  Desktop and Mobile, with confined relative paths, bitmap and SVG support,
  offline caches and dependency refresh.
- Support custom API-key channel headers. Bundle model metadata for offline
  capability and reasoning controls, refresh missing datasets when opening model
  settings, and maintain the bundled snapshot through scheduled repository updates.
- Refine host navigation and platform icons, conversation tabs, split panes,
  queued messages, provider editors and Mobile model controls.
- Fix Windows startup compatibility checks and background Git console windows;
  enlarge the Windows application icon without changing the macOS icon.

### Compatibility and distribution

Desktop, Host and Mobile use Link protocol 2; upgrade the communicating
components together. Plugin manifests and the Sailry extension remain version 1.
Existing Turso storage is validated in place; unsupported storage generations
are rejected without replacing the database. Archived databases and preview
`.sqlite` files are not automatically selected or converted.

Desktop packages are available for macOS and Windows, Host archives for macOS
and Linux, and Mobile builds for Android and iOS. iOS distribution uses
TestFlight. Linux Desktop is not included. Signed in-app Desktop update
installation is currently available on macOS; Windows provides installer and
portable packages.

## [0.2.0-alpha.2](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.2.0-alpha.2) — 2026-11-17

### Changes

- Use the initiating controller's unified directory for local and directly
  connected hosts. Keep entry and target identities separate through Desktop,
  Mobile, resource operations, subscriptions and request recovery.
- Keep projects and knowledge on their selected storage Node, with private or
  shared visibility independent of location. Aggregate knowledge retrieval and
  preserve source identity, citations and partial failures across hosts.
- Keep personal configuration authoritative on the controller and cache exact
  source IDs, revisions, settings and package digests separately for each user
  on execution hosts.
- Share execution credentials from the controller without copying renewal
  secrets. Refresh after one quarter of the actual token lifetime and resume an
  expired model step in its original turn after the same account is renewed.
- Commit personal plugin KV and local dispatch or session effects atomically,
  then reconcile only the personal writes through a durable outbox and source
  revision checks. Preserve conflicts and never replay scheduling or other effects.
- Add execution-host selection to independent reminders and scheduled tasks.
  Keep edits, controls, queue jobs and history on the resource's original host.
- Bind database files, SSH key files and SSH agents to their actual Node so
  cached configuration cannot resolve a path or credential on another machine.
- Group the official WorkBuddy package's regional channel choices under one
  brand while preserving their route and account identities.
- Classify paired mobile shells and controller Nodes from authenticated peer
  metadata, preserving stored history and direct trust while retiring identity
  merge, sponsorship and imported-directory access paths.
- Load saved Mobile hosts before refreshing remote directories, so an offline
  host does not block startup or access to other hosts.
- Preserve captured API keys for remote conversations after a channel changes
  its current key, while continuing to enforce revocation and expiry.
- Keep captured model authentication available after a plugin update or an
  unrelated settings change without bypassing disabled or removed packages.

### Compatibility

Desktop, Host and Mobile use Link protocol 2 and must be upgraded together.
The plugin manifest and extension remain version 1; packages using the new
multi-host methods require Sailry `0.2.0-alpha.2`. Data migration from
`0.2.0-alpha.1` is not supported.

## [0.2.0-alpha.1](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.2.0-alpha.1) — 2026-11-16

This prerelease is the first distribution of the Go rewrite. It includes Desktop
for macOS and Windows, Host for Linux and macOS, and Mobile for Android and iOS.
iOS builds are distributed through TestFlight. Linux Desktop is not included.

### Data compatibility

The release uses the existing `~/.sailry` profile location. On first launch,
recognized Rust databases, plugin packages and attachments are preserved under
`storage/rust-archive/` before the new SQLite storage is initialized. Rust sessions
and settings are not imported into the Go application. Node identity, controller
preferences, themes and project files remain in place.

### Changes

- Replace the Rust execution backend with the Go Node, shared Client and Link layers.
- Add desktop conversation tabs, split groups, independent project terminals and active/archived project session lists.
- Improve conversation scrolling, tool progress, image previews and toast visibility above dialogs.
- Restore OAuth callbacks, automatic model discovery and immediate model-picker refresh after channel changes.
- Fix hosted image generation compatibility and keep generated images in final outputs.
- Preserve Claude tool continuation with stable channel session identity and scoped thinking replay; retain canonical text and tool history when switching models.
- Read complete instruction files within actual model context capacity.
- Add an offline mobile demonstration host through pairing code `888888`, including an interactive simulated terminal.
- Refine mobile host selection, connection recovery, theme controls and terminal presentation.
- Expand desktop theme rendering and update the bundled themes.
- Set the desktop minimum window size to 1024 by 768.

### Distribution and updates

Desktop update catalogs and macOS update archives are signed and verified against
the bundled publisher keys. This alpha is available through the experience update
channel; stable-channel clients do not select prereleases. Windows installers and
portable archives are available from GitHub Releases; in-app installation is
currently supported on macOS only. Remote Host updates use the versioned Host
archives and installer checksums published with the release.

## [0.1.5](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.5) — 2026-11-08

- Replace Goose execution with Sailry's Agent loop and Rig provider integration
- Share MCP connections while isolating session cancellation, approvals and failures
- Keep device pairing reliable during concurrent storage writes
- Improve tool execution, Git pagination, file search and failure reporting
- Add pinned sessions and terminals, project and branch menus, and model scores
- Refine model controls, connector status, keyboard shortcuts and desktop updates
- Preserve mobile model features and refresh views when returning to the app
- Simplify browser element references and correct Windows browser layering and tab contrast
- Publish macOS and Windows Desktop, macOS and Linux Host, Android and iOS packages

Linux Desktop is not supported in this release.

## [0.1.4](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.4) — 2026-11-05

- Relicense Sailry under Apache-2.0 ([baacb30](https://github.com/ai-poet/cheaprouter-harness/commit/baacb301f030fc26b45a91acdae7b8396a509eb1))
- Keep the install progress ring circular ([612bf4b](https://github.com/ai-poet/cheaprouter-harness/commit/612bf4b33d012155cc52d44192309625a5085125))
- Name provider picker collection types ([8773406](https://github.com/ai-poet/cheaprouter-harness/commit/87734064fb70fc9999e08ad67829cc6395cc2430))
- Compile macOS tests once and shard their runs ([96111c9](https://github.com/ai-poet/cheaprouter-harness/commit/96111c96dc1d9dd58bf138921e80d3d16b3a8496))
- Include runtime parsers in shared test archives ([6dc39b5](https://github.com/ai-poet/cheaprouter-harness/commit/6dc39b54bd9eca0f6d1e13abeaf8c2a1c8e07492))
- Keep automatic session read acknowledgements silent ([ad24107](https://github.com/ai-poet/cheaprouter-harness/commit/ad241078e25b856cebdf4e322a63f79c1b56f9ae))
- Retry conflicting automatic read acknowledgements ([2ad8e3b](https://github.com/ai-poet/cheaprouter-harness/commit/2ad8e3b4143f2414867594de30bf26af9aac6416))
- Require shared test shards for product readiness ([bc2cb20](https://github.com/ai-poet/cheaprouter-harness/commit/bc2cb20afca6161311da974a244e7e8d0cd0b7b4))
- Bound file previews with incremental size accounting ([fc69e5c](https://github.com/ai-poet/cheaprouter-harness/commit/fc69e5c18a184585151066f4eabefd345a428623))
- Settle cancelled delegated waiters before releasing ownership ([7891be6](https://github.com/ai-poet/cheaprouter-harness/commit/7891be645b2a7009ceaeb78525622c4301d65570))
- Reuse verified Checks results for unchanged product inputs ([3431eaf](https://github.com/ai-poet/cheaprouter-harness/commit/3431eaf77b0aea6bcda6edea41e362d08e016f06))
- Allow interrupted conversations to reuse attachments ([cafd54d](https://github.com/ai-poet/cheaprouter-harness/commit/cafd54dfde3421ac0374d78ac77df2779174be3f))
- Prepare Sailry 0.1.4 ([45b1702](https://github.com/ai-poet/cheaprouter-harness/commit/45b17024543a2caafba8651cab950f79fbd6df81))

[Full changelog](https://github.com/ai-poet/cheaprouter-harness/compare/v0.1.3...v0.1.4)

## [0.1.3](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.3) — 2026-11-04

- Release browser resources when closing sidebar ([96b975d](https://github.com/ai-poet/cheaprouter-harness/commit/96b975dcc6ef77441f97228c26f4fe6f7f70684f))
- Publish recent conversation history before statistics ([65fe5b4](https://github.com/ai-poet/cheaprouter-harness/commit/65fe5b45f6ac4305af90ae8c6e29961556a197b3))
- Report controller presence from active subscriptions ([07d7b40](https://github.com/ai-poet/cheaprouter-harness/commit/07d7b4076d29d55ede10ca236e8de95e17c70f9a))
- Keep steering responsive and merge compatible followups ([7a7e0bf](https://github.com/ai-poet/cheaprouter-harness/commit/7a7e0bf805cb37e1b6c72fc65ce6fddfdbd779e8))
- Decide Link peer compatibility by protocol number ([e306c3b](https://github.com/ai-poet/cheaprouter-harness/commit/e306c3bfa1e5826783d80fee3b3090e4096cbaaf))
- Time run startup stages and connect MCP servers concurrently ([2045f7a](https://github.com/ai-poet/cheaprouter-harness/commit/2045f7a4bdf53de032b7f2f7f172998021706ebb))
- Measure model latency as time to first token ([6a03774](https://github.com/ai-poet/cheaprouter-harness/commit/6a037747452fbeeae5b2b975f28b1a8eea2b77a7))
- Update statistics plugin minimum version ([98d452f](https://github.com/ai-poet/cheaprouter-harness/commit/98d452fa0cd2d2a8f62c7e9e437c76e4f10c4e30))
- Return edited regions and leaner tool results to the model ([4a866c7](https://github.com/ai-poet/cheaprouter-harness/commit/4a866c7c708b6144da33d05da458cb020c868e4b))
- Update plugins for files 0.1.22 ([e1c4d1c](https://github.com/ai-poet/cheaprouter-harness/commit/e1c4d1cc4f6a78d52d330f86337b88f3c8c19399))
- Qualify guidance commands in request persistence ([651a534](https://github.com/ai-poet/cheaprouter-harness/commit/651a534e66a2efe9824a7b67a4d9db71aa9d5b4f))
- Cut code index per-query reconcile overhead without skipping validation ([efdd8ad](https://github.com/ai-poet/cheaprouter-harness/commit/efdd8adc229c5d5bcc452cadc827338cc069339a))
- Fix guidance merge and stop edge cases from review ([42ef95f](https://github.com/ai-poet/cheaprouter-harness/commit/42ef95f66f64dbcbf04d59929ea80022999e81b4))
- Fix foreground delegation, guidance and latency review findings ([5cf7884](https://github.com/ai-poet/cheaprouter-harness/commit/5cf78840d48b2e09883e9e03944a14d97befc8ba))
- Fix remaining guidance, latency, web and host update review findings ([90a1d4a](https://github.com/ai-poet/cheaprouter-harness/commit/90a1d4a82689a7f2a862327b3290fba2d6c67c4e))
- Compare recovered attempt snapshot against the recorded attempt ([b881167](https://github.com/ai-poet/cheaprouter-harness/commit/b8811677299c72130a02e9266d2bed3aef3fa56f))
- Keep explicitly stopped input as the stopped turn instead of requeueing it ([1d7b58d](https://github.com/ai-poet/cheaprouter-harness/commit/1d7b58d514c6fbd27e30b5606e77805f11113b90))
- Align node runtime tests with bounded output, summary and statistics contracts ([c3be636](https://github.com/ai-poet/cheaprouter-harness/commit/c3be6364689613722ee107a19dafb3dfdf06f028))
- Show user-stopped tool rows in their normal style ([9cc9ea0](https://github.com/ai-poet/cheaprouter-harness/commit/9cc9ea0fa57ea05f68dfd0975af137f23cc8d70e))
- Make running input a single send action without immediate guidance ([cc623d6](https://github.com/ai-poet/cheaprouter-harness/commit/cc623d6190fd4795b2d2f49c48f84a2cb252effb))
- Show turns in execution order and keep stopped input in place ([7ff3b0f](https://github.com/ai-poet/cheaprouter-harness/commit/7ff3b0fdaa99d3e0300bb2b903d0677ece4a8ce8))
- Move long foreground commands to the background when guidance is pending ([6b24626](https://github.com/ai-poet/cheaprouter-harness/commit/6b2462618730e13714a4fc3b3a78eddc65cd6950))
- Show live output and hold completion for promoted commands ([a4e825a](https://github.com/ai-poet/cheaprouter-harness/commit/a4e825a482460d1e6dd6441539739104a7a540c6))
- Expect stopped input in history rather than a projected copy ([f101e27](https://github.com/ai-poet/cheaprouter-harness/commit/f101e276b6ba9c3446644938c160eed7304143c4))
- Consolidate protocol and content behavior matrices ([1661545](https://github.com/ai-poet/cheaprouter-harness/commit/16615459cf91440bc7ad16bd80431062a4e306ca))
- Consolidate Client and Flutter behavior scenarios ([b3d4e48](https://github.com/ai-poet/cheaprouter-harness/commit/b3d4e4863d02e5a855e87e9a4c07785fb458c5be))
- Consolidate Desktop behavior scenarios ([f8eed73](https://github.com/ai-poet/cheaprouter-harness/commit/f8eed730050a6fcb2f7b9a66652b4edf03c7038a))
- Bound guidance admission stack on Tokio workers ([e86db0e](https://github.com/ai-poet/cheaprouter-harness/commit/e86db0e84d467be89cbbcb4ae7d93fa91cc32014))
- Consolidate Node pure behavior matrices ([fabf390](https://github.com/ai-poet/cheaprouter-harness/commit/fabf3903daac98747e113cab756878aea5b5b0c9))
- Consolidate Store behavior scenarios ([419e9d9](https://github.com/ai-poet/cheaprouter-harness/commit/419e9d907ce20696994eb58595940c4996f876cd))
- Consolidate Link relay rollback scenarios ([76c3882](https://github.com/ai-poet/cheaprouter-harness/commit/76c3882d8c09334a1291a6cbf7ad13cdfe4c9d2f))
- Consolidate Agent integration behavior scenarios ([eae0ef3](https://github.com/ai-poet/cheaprouter-harness/commit/eae0ef313c4d0f410eb39d7aa0dfea8b73d70692))
- Check model progress before the installation registry exists ([a33f30b](https://github.com/ai-poet/cheaprouter-harness/commit/a33f30b10bafa4033486d6d3902c1f8052ac24d6))
- Label consolidated scenario cases and check media revision steps ([d32d15e](https://github.com/ai-poet/cheaprouter-harness/commit/d32d15e245c7345be6037fc8166d4d30328b07d2))
- Restore Tab to queue input after the current turn ([30b8162](https://github.com/ai-poet/cheaprouter-harness/commit/30b8162fab76484c3e2ae1391b72f209b2daf211))
- Update plugins for code context revision conflicts ([065d98b](https://github.com/ai-poet/cheaprouter-harness/commit/065d98bac9981238c58a93684f0197ea0846f254))
- Mark joining queue input with the loader and keep background notices in app ([c89ef14](https://github.com/ai-poet/cheaprouter-harness/commit/c89ef14288b447cd2f506d9c07d0dc2e4569018d))
- Show session title as an ordinary tool row and label TTFT directly ([259c444](https://github.com/ai-poet/cheaprouter-harness/commit/259c44472316d55be2bb4569f7bbe16b2f6a3403))
- Refresh matched code index files before returning search results ([947a586](https://github.com/ai-poet/cheaprouter-harness/commit/947a5868fcb595ea0c81003f9b51133cc9a0bf44))
- Drop the package column from the composer command menu ([27f4e2f](https://github.com/ai-poet/cheaprouter-harness/commit/27f4e2f59955cba382f31089191a2dcbb7e9d68e))
- Add a session order setting for created or updated time ([202c7ee](https://github.com/ai-poet/cheaprouter-harness/commit/202c7ee29a68376ea45797d7066faaf4342ac5f7))
- Dim sidebar session ages below their titles ([0c96944](https://github.com/ai-poet/cheaprouter-harness/commit/0c969448455f802c23d6a1e02c361a2b2b09029e))
- Prepare v0.1.3 release versions and plugin catalog ([d676a4f](https://github.com/ai-poet/cheaprouter-harness/commit/d676a4fdf5d194df47e97442b5d647443f55f6d1))
- Watch only delegated sessions waiting on approval ([ce67155](https://github.com/ai-poet/cheaprouter-harness/commit/ce67155b802754c7c58ed06cca26383f6f70b969))
- Resolve strict Rust lint findings for release ([5cd7db8](https://github.com/ai-poet/cheaprouter-harness/commit/5cd7db8bd0c17b933f4f6fe7d8e2ab384af09ad9))
- Keep Host and queue presentation lint clean ([6433b9c](https://github.com/ai-poet/cheaprouter-harness/commit/6433b9cb0c2fdba8547bbeda022044c78b9bc5c9))
- Group queue row presentation options ([3f2654a](https://github.com/ai-poet/cheaprouter-harness/commit/3f2654adcb1d7e2cce53703e6b67c6f457d4db39))
- Remove redundant copies in document fixture cases ([7355b06](https://github.com/ai-poet/cheaprouter-harness/commit/7355b0665fc864b4ba77ed0e5c95bdf068a83883))
- Align release fixtures with current behavior ([a0647d3](https://github.com/ai-poet/cheaprouter-harness/commit/a0647d338eaf283713da2fa122c489a571a9d7d8))

[Full changelog](https://github.com/ai-poet/cheaprouter-harness/compare/v0.1.2...v0.1.3)

## [0.1.2](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.2) — 2026-11-03

- Fix image alignment and HTTP browsing for 0.1.2 ([134cae2](https://github.com/ai-poet/cheaprouter-harness/commit/134cae25a82a64650ca3220c8a19792eed512c88))

[Full changelog](https://github.com/ai-poet/cheaprouter-harness/compare/v0.1.1...v0.1.2)

## [0.1.1](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.1) — 2026-11-03

Upgrade Desktop, Mobile and Host together. Terminal input uses a new batch format
and is not compatible with version 0.1.0.

- Batch terminal input across local and remote controllers and limit pointer backlogs
- Restore notification history without replaying old alerts
- Use Enter to guide an active task and Tab to queue input
- Keep original tool history without separate summary requests for older tool calls
- Show recent session activity and the latest lines of command output
- Use fixed Mobile download filenames and recover Desktop packages after symbol upload failures

## [0.1.0](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.0) — 2026-11-02

- Update statistics to 0.1.24 with first-step bars, speed fallback and grouped tool names ([7128834](https://github.com/ai-poet/cheaprouter-harness/commit/71288348498a3cc298fada59418f3e8088a27261))
- Update files to 0.1.18 without the written result notice ([fd4ae2d](https://github.com/ai-poet/cheaprouter-harness/commit/fd4ae2d49e37af5cacd39744534a656bdd62b95d))
- Reselect tool details when a hidden conversation pane is shown again ([9a92c0d](https://github.com/ai-poet/cheaprouter-harness/commit/9a92c0dbd2ce2d1797ce18069d539ecdddc8a2ef))
- Show file writes and edits as line-numbered checkpoint diffs ([adc14db](https://github.com/ai-poet/cheaprouter-harness/commit/adc14db6c777a18acbb7433776446348e3c5246b))
- Show background results that start automatic turns ([1988263](https://github.com/ai-poet/cheaprouter-harness/commit/1988263319856b7b6856f7e56b9cf5ae154fa300))
- Tidy background command output and show elapsed time ([e88e0a8](https://github.com/ai-poet/cheaprouter-harness/commit/e88e0a82d9c4e49afeb7619a968e5fd2694e719b))
- Skip conflict toasts for automatic session read acknowledgements ([335c0c9](https://github.com/ai-poet/cheaprouter-harness/commit/335c0c9be9b7c04af77477c0ff0b4a3a7dbcb11e))
- Fix visible history search and current Mobile provider contracts ([725fd04](https://github.com/ai-poet/cheaprouter-harness/commit/725fd04996112d4b7cc3a2c77dd53d2a59e2211c))
- Update delegation to 0.1.29 for the subagent delete icon ([8ab3501](https://github.com/ai-poet/cheaprouter-harness/commit/8ab3501ca23426f9ea98061c1bacb155c819aa73))
- Stabilize terminal exit and registered worktree fixtures ([0a68667](https://github.com/ai-poet/cheaprouter-harness/commit/0a68667a059fb14bf034294f5fce7d382ed18f4f))
- Speed up the conversation activity popover and fix header popover focus ([3e3ad9c](https://github.com/ai-poet/cheaprouter-harness/commit/3e3ad9c59b2df43101915d1ee3f69d51281e77fb))
- Keep conversations live through visibility changes and lazy turn recovery ([fb715bf](https://github.com/ai-poet/cheaprouter-harness/commit/fb715bfb783856caae11f77b16ce10a17903ba04))
- Render visuals and HTML in the document editor ([cf950f0](https://github.com/ai-poet/cheaprouter-harness/commit/cf950f05aa6c25da2e80e5185177239af8de56e5))
- Add Markdown formatting toolbar, link editing and responsive files toolbar ([9b0cbd7](https://github.com/ai-poet/cheaprouter-harness/commit/9b0cbd7356291a5ab65fa2817448e940b5fa6afb))
- Reuse session statistics for queue commands and report missing search scopes neutrally ([44632bf](https://github.com/ai-poet/cheaprouter-harness/commit/44632bfb2f4327395c2ca08283626753a0f64216))
- Unify editor formatting and reuse document visuals ([889c75a](https://github.com/ai-poet/cheaprouter-harness/commit/889c75a28fe040fff901307d29676bc5394824eb))
- Bound conversation reads and defer derived presentation work ([53681be](https://github.com/ai-poet/cheaprouter-harness/commit/53681bedbf6e4daf9305373c0e5a7f6ac6261778))
- Align Claude OAuth requests and refresh handling with OMP ([8e4b338](https://github.com/ai-poet/cheaprouter-harness/commit/8e4b33852dcec53ee0cb33058d6edf7fab804ec4))
- Pin GDK to the Sailry fork with tested provider fixes ([bb6fe98](https://github.com/ai-poet/cheaprouter-harness/commit/bb6fe98dc1415f12eebfbf64882fc07db788b09d))
- Keep request rejections visible without reporting private details ([b9e9b32](https://github.com/ai-poet/cheaprouter-harness/commit/b9e9b32927a979a33f2e4e97f1602af6af9cd72d))
- Compare captured execution settings when joining guidance ([8c9e452](https://github.com/ai-poet/cheaprouter-harness/commit/8c9e452b7003e33baf9075b0fad7cc0bf8e81026))
- Destroy the conversation side panel when it is closed ([e774afd](https://github.com/ai-poet/cheaprouter-harness/commit/e774afd4618c03915c83d21580534d742bc26a19))
- Report filesystem access failures as partial search results ([cddc964](https://github.com/ai-poet/cheaprouter-harness/commit/cddc9643b8a207d33d55e10e7bd9045fc5e08d16))
- Classify expected tool and transport failures as warnings ([7799086](https://github.com/ai-poet/cheaprouter-harness/commit/7799086a5f8be9f8a6a74ed7ecab4122b9f5c49d))
- Preserve and upload symbols for local Desktop distribution builds ([b26b414](https://github.com/ai-poet/cheaprouter-harness/commit/b26b414e80c6fbb75b1ca0d304f002b998ccc5dd))
- Scope code search issues and rank exact definitions first ([efe8f47](https://github.com/ai-poet/cheaprouter-harness/commit/efe8f47ce957ede7ce5e4714b3211e5f3e5ad00f))
- Update plugins for code 0.1.11 ([8bd0efe](https://github.com/ai-poet/cheaprouter-harness/commit/8bd0efe9b4dbafd43cb0a543ad70f746d157a551))
- Defer guidance intent and coalesce session statistics reads ([87b8b8a](https://github.com/ai-poet/cheaprouter-harness/commit/87b8b8a5158d174cdea4ae855196416ef9fd7adf))
- Adopt upstream Responses reasoning with provider-scoped replay ([42a8360](https://github.com/ai-poet/cheaprouter-harness/commit/42a836077bb91443b0f52bf0bb19c8c80edeef8c))
- Add initial plugin tools and the turn message read action ([5a9f28e](https://github.com/ai-poet/cheaprouter-harness/commit/5a9f28e7239aaf187efa49c0135db62767844ead))
- Update plugins for memory 0.1.15 ([2aa88b4](https://github.com/ai-poet/cheaprouter-harness/commit/2aa88b4828a8242a99abcb1a99a68223517a1e1e))
- Render background results in the shared user bubble ([349408e](https://github.com/ai-poet/cheaprouter-harness/commit/349408e314a62a3615de214418c87a4fae08cc19))
- Record review and implementation agent model preferences ([004f9ea](https://github.com/ai-poet/cheaprouter-harness/commit/004f9eaf20b38cd89dbbcd00506e10f02e73f7cd))
- Keep tool details by expansion state instead of viewport position ([0e4f7bf](https://github.com/ai-poet/cheaprouter-harness/commit/0e4f7bf39e60de033b8304c47802b6afd7624721))
- Project the saved snapshot diff for every file mutation ([9c40d79](https://github.com/ai-poet/cheaprouter-harness/commit/9c40d791a468b5f5bae874fab33e3211170d059c))
- Keep the declared preview for notebook edits ([fe07534](https://github.com/ai-poet/cheaprouter-harness/commit/fe075343026c006bd4b86fb56023bca35ac26cc0))
- Persist plugin turn parts once per conversation ([26e99d5](https://github.com/ai-poet/cheaprouter-harness/commit/26e99d5e02806a5eddade9804f67282b8f1ebceb))
- Update plugins for memory 0.1.16 ([5e20cdd](https://github.com/ai-poet/cheaprouter-harness/commit/5e20cddcb51af99ddcb354cdc636b24ea44e0b66))
- Fix memory writes, repository reconnects, and context deduplication ([0a4eee7](https://github.com/ai-poet/cheaprouter-harness/commit/0a4eee75ae257ce802942adfcc9086134cda762f))
- Keep visible conversation details active during terminal focus ([b6ce18b](https://github.com/ai-poet/cheaprouter-harness/commit/b6ce18b522061a735711f607269166124ea0b039))
- Center the code copy button in its header ([8442603](https://github.com/ai-poet/cheaprouter-harness/commit/84426036c8ed61c20e018f13a0727a102aa0555f))
- Update commands plugin to omit startup-only details ([52b5e8a](https://github.com/ai-poet/cheaprouter-harness/commit/52b5e8aae48f3066739d7a87a43455e3a17ed8f2))
- Serve the full Kit icon catalog for plugin icons ([a3d28d1](https://github.com/ai-poet/cheaprouter-harness/commit/a3d28d171db0b22cfa1d1fefb1a56df767d8594a))
- Show connector account loading and retry states ([e88752c](https://github.com/ai-poet/cheaprouter-harness/commit/e88752c8895d2c69d9487b22e460a2c47aa93e5f))
- Show focus rings only for keyboard focus ([65d52f4](https://github.com/ai-poet/cheaprouter-harness/commit/65d52f45d80389e6c9e7076b0784d9576b560e36))
- Fix empty image placeholder, image clicks and math-paragraph inline code ([bc2c28a](https://github.com/ai-poet/cheaprouter-harness/commit/bc2c28aa7cbe8ce1cf18e17b1d52a673e839063f))
- Preview document images on click only when read-only ([7f6eb75](https://github.com/ai-poet/cheaprouter-harness/commit/7f6eb754d966226c186e242898637590dfac0cfb))
- Hide agent-only parts from conversation search and clean result snippets ([9e7394b](https://github.com/ai-poet/cheaprouter-harness/commit/9e7394b2364261b066b94f424b198daa583a24ee))
- Polish file change review, search rows and activity spacing ([61237c0](https://github.com/ai-poet/cheaprouter-harness/commit/61237c00b89081b10dd7020f23f2b4326f4d4a74))
- Stop recording notifications for terminal exits ([b33d66c](https://github.com/ai-poet/cheaprouter-harness/commit/b33d66c3f9775cea69d8007c717be7ebbbc31cb3))
- Update plugins for terminal exits, statistics tooltips and memory deletion ([00b4e1a](https://github.com/ai-poet/cheaprouter-harness/commit/00b4e1aa89db4d12be1961fbe7931e0a92cf0fa7))
- Animate shell sidebar and details panes in both directions ([21a252f](https://github.com/ai-poet/cheaprouter-harness/commit/21a252fff8a06e967d8b2cc546e078160971a096))
- Skip inaccessible Git subdirectories silently ([d9e1b07](https://github.com/ai-poet/cheaprouter-harness/commit/d9e1b072b3b8a0ace4679562d2d7d74f77ae1fd0))
- Acknowledge guidance queued with different settings ([c67d01c](https://github.com/ai-poet/cheaprouter-harness/commit/c67d01c3111f9b345d9fb79a609b431c34d2d2a5))
- Keep conversation image previews enabled ([b347385](https://github.com/ai-poet/cheaprouter-harness/commit/b347385b2f906e5d690a2cc5d86179f058c497eb))
- Make background command follow-up opt in ([aa780a7](https://github.com/ai-poet/cheaprouter-harness/commit/aa780a7f2726e53f02ead971c754c2b3cdf1907a))
- Update commands plugin for optional background follow-up ([2c0a341](https://github.com/ai-poet/cheaprouter-harness/commit/2c0a341041867c5dbee744419b5011eaff4c113b))
- Cover terminal redraws during sidebar resizing ([b60a924](https://github.com/ai-poet/cheaprouter-harness/commit/b60a924d7c7fd9be8c3c0edb2932b530120f5437))
- Skip inaccessible tracked directories in Git status ([9076e03](https://github.com/ai-poet/cheaprouter-harness/commit/9076e03032ed2c9601f6596ef566c4d6ad5a0f7d))
- Repair strict lint and current history fixtures ([8b26edf](https://github.com/ai-poet/cheaprouter-harness/commit/8b26edfb46127a54a1ed6848d76ce9891886392e))
- Pin restored plugin catalog lineage ([5890688](https://github.com/ai-poet/cheaprouter-harness/commit/589068889bf577687e192f2b2b2374728dc74c91))
- Prepare 0.1.0-alpha.8 ([841b4f4](https://github.com/ai-poet/cheaprouter-harness/commit/841b4f4c3f654cd927d66a8c90693397810c46d2))
- Match Dart paging fixture to on-demand history ([bc0dd35](https://github.com/ai-poet/cheaprouter-harness/commit/bc0dd353621d95708fb7dc446e3e41944e4e730f))
- Remove timing races from restore and reminder fixtures ([a11fe9c](https://github.com/ai-poet/cheaprouter-harness/commit/a11fe9ceea19be189d87711ee13723fa45d6d6cc))
- Remove the synthetic model retry prompt ([1ce14a3](https://github.com/ai-poet/cheaprouter-harness/commit/1ce14a3e27080fac854e2fe18136c84315630a1d))
- Expose hook timeout test outcomes ([7ad7ec5](https://github.com/ai-poet/cheaprouter-harness/commit/7ad7ec57e13d9540f029484a3b622de929b4e328))
- Render mobile transcript steps as desktop tool rows ([c7339d3](https://github.com/ai-poet/cheaprouter-harness/commit/c7339d3e78d256c36f1e284963922889b717d354))
- Add the mobile composer activity strip ([4d2c4fe](https://github.com/ai-poet/cheaprouter-harness/commit/4d2c4fec112fa1ed76fa013a61d97433d7906c40))
- Redesign the mobile conversation list ([24104e2](https://github.com/ai-poet/cheaprouter-harness/commit/24104e29a2ad3305b17f120f2b770ef55b2f9e0c))
- Use the singular step label for one step ([718c5d7](https://github.com/ai-poet/cheaprouter-harness/commit/718c5d7b8a261fa5e5cb88a336522818268ea2f2))
- Handle missing requested tool paths without masking other failures ([75bc9f5](https://github.com/ai-poet/cheaprouter-harness/commit/75bc9f5b24514141ebd0c816d2212334e4ffe58d))
- Await derived usage in mobile contract fixtures ([a6ca46b](https://github.com/ai-poet/cheaprouter-harness/commit/a6ca46b83ca105ada2f9b57adb9d76547fed1db4))
- Open mobile conversations without stalling the route transition ([7c3c796](https://github.com/ai-poet/cheaprouter-harness/commit/7c3c796cecd918db00c73a4281069dd26c05231e))
- Align product versions with the Node status capability ([20b423a](https://github.com/ai-poet/cheaprouter-harness/commit/20b423add425f44c338be0a02aef7de752c2be5c))
- Remove roles and memory settings from mobile ([3717ec9](https://github.com/ai-poet/cheaprouter-harness/commit/3717ec926d7d60a0f0ba88e2ce0f01d0b7160a45))
- Add Node-owned status protocol and Client feed ([fb711a0](https://github.com/ai-poet/cheaprouter-harness/commit/fb711a053e11cec422387f6500a49c6bb2086bbc))
- Compute scoped plugin status on the execution Node ([1556017](https://github.com/ai-poet/cheaprouter-harness/commit/15560178ff54059dbe6b3ab9a20373179203675b))
- Pin Git and Goals packages with Node-owned status ([7bdd23f](https://github.com/ai-poet/cheaprouter-harness/commit/7bdd23f6c3fbb4f8c31ba4b0961718d870f15989))
- Render Node status in Desktop contributions ([26e4371](https://github.com/ai-poet/cheaprouter-harness/commit/26e4371265fa20e193c589a73286a7c2d1f6c0a7))
- Preserve status intents and uncertain action recovery ([ef6cdbb](https://github.com/ai-poet/cheaprouter-harness/commit/ef6cdbbb9eb168809a83d82bbdf20486671eefb2))
- Show Node-owned status in the Mobile controller ([ea6a09c](https://github.com/ai-poet/cheaprouter-harness/commit/ea6a09cccd21751b8abed451b66fcf16580a13fe))
- Retain uncertain status retries and refresh moved scopes ([76a8235](https://github.com/ai-poet/cheaprouter-harness/commit/76a82356b1e57166ae3c11d9f8ef1874b011b216))
- Show an optional scope label in the mobile project bar ([8b35210](https://github.com/ai-poet/cheaprouter-harness/commit/8b352105431208b99626a64bec6e34fa7e9a63e7))
- Redesign the mobile usage page around the desktop statistics data ([0f80c6f](https://github.com/ai-poet/cheaprouter-harness/commit/0f80c6faaa9ad31928582d71ebcf85fd4dfc2b27))
- Drop status retries when their captured contribution disappears ([b5283bd](https://github.com/ai-poet/cheaprouter-harness/commit/b5283bd02835d55947585bfc62cd0434f867a7a4))
- Brace the status availability guard ([356be2e](https://github.com/ai-poet/cheaprouter-harness/commit/356be2e8d5c5cb47113cce8043f3c1b58aa4660b))
- Recover status subscriptions after watcher failure ([ea8b395](https://github.com/ai-poet/cheaprouter-harness/commit/ea8b395d9b3ddb1aa96c42e69b52b82a978cdd38))
- Preserve status read cancellation through command logging ([c906023](https://github.com/ai-poet/cheaprouter-harness/commit/c906023e4d5c83a21f18e026cb5b74db8eb86c19))
- Use the builtin test attribute for status helpers ([ad2037a](https://github.com/ai-poet/cheaprouter-harness/commit/ad2037ab5a7f6e7ead8165aded4eb562a9f1bbb4))
- Add mobile host metric history and formatting ([9695cc9](https://github.com/ai-poet/cheaprouter-harness/commit/9695cc985d9a81b389d127dd00e82207f1516652))
- Redesign the mobile hosts page around host cards and charts ([79f586f](https://github.com/ai-poet/cheaprouter-harness/commit/79f586f70d162b5e8e0073e94ae9db194efed683))
- Add mobile Git and file presentation helpers ([27ee961](https://github.com/ai-poet/cheaprouter-harness/commit/27ee961aeb7c2bf8546643ae2e543680e882fde6))
- Redesign the mobile resources pages ([b374df0](https://github.com/ai-poet/cheaprouter-harness/commit/b374df036bbf4573bc2ed9a8020df72ce82f21d4))
- Load network presets from GitHub and probe relay latency ([c91fdb3](https://github.com/ai-poet/cheaprouter-harness/commit/c91fdb3e922dc5ea69960b685405d77dd3b8bebb))
- Load pairing relay choices from the Node catalog ([aa5c4f8](https://github.com/ai-poet/cheaprouter-harness/commit/aa5c4f86f7d12652b82c2d77bfc1413bda6bf178))
- Use controller identity in wrong-target discovery test ([488e330](https://github.com/ai-poet/cheaprouter-harness/commit/488e33078c1d6c791a38b84c611d02518e2c3447))
- Localize network presets for the Desktop Chinese locale ([106ac49](https://github.com/ai-poet/cheaprouter-harness/commit/106ac492ca6fd433ff64126287592781623b311f))
- Share the cancellation fixture client through Arc ([4687a46](https://github.com/ai-poet/cheaprouter-harness/commit/4687a464d54ba0a3f13ad85b4b9b72ef2d1ffec2))
- Initialize ring in standalone network test fixtures ([561b34b](https://github.com/ai-poet/cheaprouter-harness/commit/561b34b0fa935bb9ed7cfb214eea5af87207b0f6))
- Name the default relay as Public network ([255328c](https://github.com/ai-poet/cheaprouter-harness/commit/255328c341bd51616e0e3c0da867fc0c3574ad82))
- Reconnect reads after paired route refreshes ([c52c5db](https://github.com/ai-poet/cheaprouter-harness/commit/c52c5dbbfd21c3d3af90ff12400c657ffaf11025))
- Allow Mobile to forget saved hosts offline ([5f1b0ff](https://github.com/ai-poet/cheaprouter-harness/commit/5f1b0ff2cc32a7064872b9838d302c5bfca9a084))
- Serialize Mobile pairing and host removal ([256b152](https://github.com/ai-poet/cheaprouter-harness/commit/256b1525596a46d8956ba4dcc6cc6a4d27995c4b))
- Regenerate Mobile host removal bindings ([7fa14ad](https://github.com/ai-poet/cheaprouter-harness/commit/7fa14ad8f7b4e6115dc75c766cb7c0aacda1e9cd))
- Fill the pinned host tab header to its declared extent ([6f4dc52](https://github.com/ai-poet/cheaprouter-harness/commit/6f4dc52acc30aa8a5e0e3296bfa188bfa58e84ff))
- Restart stopped Mobile watches on host selection ([fa21d3e](https://github.com/ai-poet/cheaprouter-harness/commit/fa21d3e08e6ed5e655301782f8b25c4eb25c1ed3))
- Tidy mobile tool details, Git headers, terminal and file viewer ([798a47d](https://github.com/ai-poet/cheaprouter-harness/commit/798a47d1969596a3237f63e5bdf80ab339ef68bd))
- Remove unused mobile command status copy ([89e605b](https://github.com/ai-poet/cheaprouter-harness/commit/89e605ba78a9ed78e360d8fd9ab760658d8b657d))
- Keep forgotten-host assertion independent of peer debugging ([1a43eab](https://github.com/ai-poet/cheaprouter-harness/commit/1a43eab5cfb42392746ee16ef8dfb2f917aa43df))
- Use glass capsules for mobile conversation chips and attachments ([10df47f](https://github.com/ai-poet/cheaprouter-harness/commit/10df47fc0d719eee2d953a3535ff42cc884e6ac8))
- Reduce mobile task filters to status tabs with counts ([e360a75](https://github.com/ai-poet/cheaprouter-harness/commit/e360a7512d0559448c134527ea0d839f1bf1b4f2))
- Remove Mobile tool detail disclosure ([cce8f30](https://github.com/ai-poet/cheaprouter-harness/commit/cce8f30b3bf00092af5a8dae04502bcd22899a95))
- Render connection routes with Kit tags ([b4d1d72](https://github.com/ai-poet/cheaprouter-harness/commit/b4d1d724e6160adcf037b2c3c7779849d3a79773))
- Check Mobile tool argument content contracts ([ee6a3c3](https://github.com/ai-poet/cheaprouter-harness/commit/ee6a3c3f7cbda7123e108f4a9b2e135fe6bd7a15))
- Scope mobile tool detail reads to active surfaces ([2714e0f](https://github.com/ai-poet/cheaprouter-harness/commit/2714e0f173f9d2cf45428575b6240ab0d64759c5))
- Serve runtime catalogs through Sailry website ([0f11588](https://github.com/ai-poet/cheaprouter-harness/commit/0f11588f2b7fefa6b3f821f262d578a09165276b))
- Deploy website after formal release catalog publication ([fc3b76b](https://github.com/ai-poet/cheaprouter-harness/commit/fc3b76b11b86b649c5516a19024b5b2b22a7ef43))
- Poll mobile host metrics inside the Node rate window ([6fed736](https://github.com/ai-poet/cheaprouter-harness/commit/6fed73669fc2611808d6fcd1750f20e9365adfb8))
- Show mobile turn changes only with file changes; flatten approval card ([325f37a](https://github.com/ai-poet/cheaprouter-harness/commit/325f37a178250c3f9218a06e51b15741b6108cf8))
- Merge mobile stop and send into one composer action ([ffb49b5](https://github.com/ai-poet/cheaprouter-harness/commit/ffb49b5bb865f389a85acda70b4e52cd37638cb8))
- Add the Desktop chats group to mobile project pickers ([b82c11b](https://github.com/ai-poet/cheaprouter-harness/commit/b82c11bde832f1315d1a5c38b75f8d8d3ab6189d))
- Rebuild the mobile model picker after the Desktop layout ([d729fcc](https://github.com/ai-poet/cheaprouter-harness/commit/d729fccd2082699f2c273790a40316046b16d308))
- Use rounded plain tabs for mobile task filters ([df803a8](https://github.com/ai-poet/cheaprouter-harness/commit/df803a83d286dd4bf99ce2a4c3e7a461cd51c06e))
- Collapse mobile multi-file diffs into rows; swipe to close terminals ([ef59ff0](https://github.com/ai-poet/cheaprouter-harness/commit/ef59ff01f5d49c59b3623bb7e5efdfc1811ccd0b))
- Drop the settings shortcut from the mobile host sheet ([58ed189](https://github.com/ai-poet/cheaprouter-harness/commit/58ed1897204af15d7c96d65986ba26c23ebb34af))
- Keep mobile host charts continuous after missed reads ([cef403b](https://github.com/ai-poet/cheaprouter-harness/commit/cef403b165c6dce3c53de71286afe678ad89f5a6))
- Fade the mobile page header over scrolling content ([da9c44e](https://github.com/ai-poet/cheaprouter-harness/commit/da9c44e5e4c29760372f825385df8a6317adde62))
- Redesign mobile model provider settings ([19084e5](https://github.com/ai-poet/cheaprouter-harness/commit/19084e5d964273ecc99156964cfbccc82f92a804))
- Fix mobile task ownership and provider edit recovery ([864d46e](https://github.com/ai-poet/cheaprouter-harness/commit/864d46ecd01ed4ab0de0416a3833523f9b8db88c))
- Fix mobile and plugin contract CI regressions ([d2c534d](https://github.com/ai-poet/cheaprouter-harness/commit/d2c534da35b85d728ee688cfacedbf6069674c9d))
- Configure the iOS development signing team ([aaf8fbd](https://github.com/ai-poet/cheaprouter-harness/commit/aaf8fbd5ffdba2dadb2158c4fc5047048863078e))
- Automate signed Mobile packages and TestFlight uploads ([8c7a481](https://github.com/ai-poet/cheaprouter-harness/commit/8c7a481d3249a593f2cbb88eb21acd7af0fce8fb))
- Allow checked source commits in Mobile packaging ([8b146a2](https://github.com/ai-poet/cheaprouter-harness/commit/8b146a207a912aaaece85571f30b2fbbb20b7a3b))
- Wait for recipient completion in guidance test ([098ab77](https://github.com/ai-poet/cheaprouter-harness/commit/098ab77d5d9a11714734eceda9b72f22d6421d47))
- Publish version releases before independent product packages ([8b1f165](https://github.com/ai-poet/cheaprouter-harness/commit/8b1f165b4c894d4bd616cb1f2bdace5c0b374e9a))
- Gate packages on product checks and scope release tooling CI ([dbc6df9](https://github.com/ai-poet/cheaprouter-harness/commit/dbc6df98a6a19d778a41d9cf2f8e3f3259fad0da))
- Recover transient model failures without replaying agent work ([b828317](https://github.com/ai-poet/cheaprouter-harness/commit/b828317fa272d0bb0a93f414fd2e1072cf5945f4))
- Add team protocol and client catalog projection ([76d276f](https://github.com/ai-poet/cheaprouter-harness/commit/76d276f1cd16fe028f663d9576425c2cb57eef76))
- Store teams and capture team rosters in Node sessions ([6b2e034](https://github.com/ai-poet/cheaprouter-harness/commit/6b2e03467a152bb9b93af7a0c8cac9c6cd93e377))
- Name the team save command PutTeam like PutRole ([1e9fdea](https://github.com/ai-poet/cheaprouter-harness/commit/1e9fdeae3ba610a02c610376e35fc50be00569d7))
- Tie member model lookup to the snapshot lifetime ([7cda07c](https://github.com/ai-poet/cheaprouter-harness/commit/7cda07c7f0f13f46b4682e2329ed16a4caff3742))
- Select teams and member models in the mobile model sheet ([3dfe76b](https://github.com/ai-poet/cheaprouter-harness/commit/3dfe76b2ceb3a217098978aaab6121908664bf10))
- Reuse shared test builds and scope integration-only CI ([ed69203](https://github.com/ai-poet/cheaprouter-harness/commit/ed692038e79560cff61af3b3ecced4acaccc5397))
- Add the core Teams settings page with team and role editors ([37011d7](https://github.com/ai-poet/cheaprouter-harness/commit/37011d70761472c2464ef6f3f48d9579ba29b348))
- Select teams and member models in the composer model popover ([dbf5dd8](https://github.com/ai-poet/cheaprouter-harness/commit/dbf5dd80730f4fbfe900204d9da89934aa04717a))
- Return the role presets menu as an element ([cdb7391](https://github.com/ai-poet/cheaprouter-harness/commit/cdb73917188fda15cc6af01260b5f9a347b5a96e))
- Move delegation and Flagship into the Node core ([4f0df05](https://github.com/ai-poet/cheaprouter-harness/commit/4f0df05754704579d18c86dfc20d8461fe71a88e))
- Point plugins at the catalog without delegation ([72aae91](https://github.com/ai-poet/cheaprouter-harness/commit/72aae917fd85ae4c8e829e679dc9d7fea7007ed8))
- Offer Flagship in the mobile mode selector ([8efba6b](https://github.com/ai-poet/cheaprouter-harness/commit/8efba6b0a138349d92b5c17b8176fc9d1268f3ad))
- List mobile work modes in a sheet so plugin modes fit ([2dd30b6](https://github.com/ai-poet/cheaprouter-harness/commit/2dd30b6f22e291522afdeed9a43825cab527bb6f))
- Update code-review for Flagship team delegation ([8761c52](https://github.com/ai-poet/cheaprouter-harness/commit/8761c52eaba91ab419f9e20b9efdb5d1f0c5beb3))
- Fix team concurrency controls and configuration recovery ([946088a](https://github.com/ai-poet/cheaprouter-harness/commit/946088a6a9b8d097e708101b15c479d5f5ae0e92))
- Put the team list back button in a header ([b4d3298](https://github.com/ai-poet/cheaprouter-harness/commit/b4d32987294fcec96ad9bc21663b619c2d80267a))
- Auto-save team concurrency with a slider above the tabs ([8f0ed61](https://github.com/ai-poet/cheaprouter-harness/commit/8f0ed6104ca7570b84d088e411f226869cd3ad3a))
- Give every role and team a visible appearance ([10ede2e](https://github.com/ai-poet/cheaprouter-harness/commit/10ede2ed2be5e8d7c9a041ca391426bbbd1ede6f))
- Show localized built-in role names in the model popover ([e398e55](https://github.com/ai-poet/cheaprouter-harness/commit/e398e55f3cdf7e4df67b51d343707ff08eb108eb))
- Show mobile context compaction as a plain marker ([bfa93a6](https://github.com/ai-poet/cheaprouter-harness/commit/bfa93a67bc344f5ed590b4672f18826fc417d045))
- Hide subagent sessions from the mobile list and drop their footer label ([692f799](https://github.com/ai-poet/cheaprouter-harness/commit/692f799a41ea1d43b4687b7f8fda0c7ea73ebd1c))
- Use the regular size for the team and role create buttons ([78b675d](https://github.com/ai-poet/cheaprouter-harness/commit/78b675d8b58a3d4e3a7d793fcbaf82fb5f7b6f0f))
- Put team selection in the mobile model sheet ([21580d1](https://github.com/ai-poet/cheaprouter-harness/commit/21580d13c7abc1fcd3d59c5a5e68f2208ff099e8))
- Remove the unused mobile subagent footer label ([fd9380a](https://github.com/ai-poet/cheaprouter-harness/commit/fd9380aa00430a9247343cbfc81c7b8de529622c))
- Fill identity avatars with their color and make the team icon a clear picker ([184528a](https://github.com/ai-poet/cheaprouter-harness/commit/184528a0fbc8ee0b4ed2293bd8ebbd2ebceac7a5))
- Left-align the team list back button and line up its rows ([9ee3722](https://github.com/ai-poet/cheaprouter-harness/commit/9ee37220147a04c494b90518e88f7f69d31df2c4))
- Enlarge team member avatars and align the team dialog close button ([d031156](https://github.com/ai-poet/cheaprouter-harness/commit/d03115639c10de76230616f81e6264baed28f3a2))
- Show raw reasoning efforts on mobile and let models add or remove them ([494f3e7](https://github.com/ai-poet/cheaprouter-harness/commit/494f3e7440532e1bbcd64006580e95abc6b4cfb5))
- Keep the lead's model features for members whose model supports them ([ddd9d44](https://github.com/ai-poet/cheaprouter-harness/commit/ddd9d4485ea6798ee6a71c60098d60fcce7d922d))
- Use a thin ring for the selected member avatar ([717eccd](https://github.com/ai-poet/cheaprouter-harness/commit/717eccddff8263b91bc5c60a2f775f9b9f23a8e8))
- Keep the team icon color on hover, outline Add role and drop its search ([91b14c3](https://github.com/ai-poet/cheaprouter-harness/commit/91b14c33988253fcc3d31594f39dbc1ca4125e52))
- Show the connection route as plain text ([0a57a77](https://github.com/ai-poet/cheaprouter-harness/commit/0a57a779d8650c5a0c17aa0b02ca1bc8a8b1fa36))
- Size team row avatars like role rows ([badf532](https://github.com/ai-poet/cheaprouter-harness/commit/badf53220bea6386493d9a60820ef79d5be580e2))
- Fade unselected member avatars instead of ringing the selected one ([28707f7](https://github.com/ai-poet/cheaprouter-harness/commit/28707f7cda8588be7f1be2e93fcb99a9175809c4))
- Drop the back header from the model popover team list ([335d022](https://github.com/ai-poet/cheaprouter-harness/commit/335d022b5bb341c8916093793c63a5e243979e8b))
- Show only the team icon in the model popover team chooser ([9dc8937](https://github.com/ai-poet/cheaprouter-harness/commit/9dc8937ee86a6eaac4ed1d57cb5e328923e50fbf))
- Update plugins for web-search 0.1.13 ([cab9e9b](https://github.com/ai-poet/cheaprouter-harness/commit/cab9e9b0f17c260c9ddce49910a5ba1c574a68e4))
- Update plugins for web-search 0.1.14 ([1ed3554](https://github.com/ai-poet/cheaprouter-harness/commit/1ed355465d9dcc43ecbbe4223f4c385a6aac06c3))
- Put the team chooser at the start and members at the end of the popover row ([55b6290](https://github.com/ai-poet/cheaprouter-harness/commit/55b6290a02a731a95a911a9b30348e8562915424))
- Draw team and role identities as provider-sized tiles ([4d6609b](https://github.com/ai-poet/cheaprouter-harness/commit/4d6609bd7329cb1e356a4ff45d0fc884890de7bc))
- Tint team and role identities with a translucent fill ([0c6d554](https://github.com/ai-poet/cheaprouter-harness/commit/0c6d554506c423b12a5ce7c43fac7a3257686a50))
- Right-align popover members inside their scroll area ([082ba02](https://github.com/ai-poet/cheaprouter-harness/commit/082ba0262f78227604a6db9ce68367103b8d1404))
- Show team member avatars side by side instead of stacked ([cda3ce9](https://github.com/ai-poet/cheaprouter-harness/commit/cda3ce9bdd6dd0d7a27e2dfa129475345cb74145))
- Give the team lead a member-style header ([6ca2e55](https://github.com/ai-poet/cheaprouter-harness/commit/6ca2e55cb494c524d76b7b1273c26e6a47232d37))
- Fix terminal environment, ownership recovery and direct closure ([1840d4a](https://github.com/ai-poet/cheaprouter-harness/commit/1840d4a0a49dc72763d655eaf78eac0834f16f54))
- Add a shared modal list and use it for connection methods ([d01ab13](https://github.com/ai-poet/cheaprouter-harness/commit/d01ab13c857e569d65d3cfe399d61423170673c6))
- Rewrite the team dialog as a single form with the shared list ([bfd416f](https://github.com/ai-poet/cheaprouter-harness/commit/bfd416f250afc9f31285dfe201b76488320f04e5))
- Show provider models in the shared modal list ([2663006](https://github.com/ai-poet/cheaprouter-harness/commit/2663006accb0d6a5494c2cb1ca1df685c1078727))
- Use the shared modal list for connector accounts and types ([ae0031c](https://github.com/ai-poet/cheaprouter-harness/commit/ae0031c9a78405642c38429b83aed61088aaad8a))
- Use the shared modal list for backup records and archives ([ce9f18a](https://github.com/ai-poet/cheaprouter-harness/commit/ce9f18a4f274417ad342e041d1730624e5630920))
- Use the shared modal list for skill install candidates ([8c5082d](https://github.com/ai-poet/cheaprouter-harness/commit/8c5082d664152029de3b678f9265b68ae8c0be28))
- Use the shared modal list for project session records ([64495d0](https://github.com/ai-poet/cheaprouter-harness/commit/64495d0db71e42109f3cf639baf32a93cb33aa39))
- Use the shared modal list for CLI launch profiles ([72ae5ac](https://github.com/ai-poet/cheaprouter-harness/commit/72ae5ace2eabd29011ef11e383192d742d8ecdd8))
- Use the shared modal list for the worktree manager ([d5d6909](https://github.com/ai-poet/cheaprouter-harness/commit/d5d6909bdbfc9b90061d2c58c714b3bae042091c))
- Use the shared modal list for host project sharing ([3fd8ef3](https://github.com/ai-poet/cheaprouter-harness/commit/3fd8ef34d2faab181711da15febe2a232f97dd4d))
- Let modal list rows show details without a disclosure ([5c94c6c](https://github.com/ai-poet/cheaprouter-harness/commit/5c94c6c4992944e0a1e56026cb9713314ecbbcce))
- Use the shared modal list for plugin updates ([d61cafd](https://github.com/ai-poet/cheaprouter-harness/commit/d61cafd1963794ccae3f97a10fd6aca182dae983))
- Fix plugin form modal height and scrolling ([657ae27](https://github.com/ai-poet/cheaprouter-harness/commit/657ae27989f2954d220a0ae8d79846c063a1a2fc))
- Unify model and team sessions for controllers and plugins ([8b13e13](https://github.com/ai-poet/cheaprouter-harness/commit/8b13e1392b0f1a3dcd2aa8dba44fcf8a0eb46618))
- Update scheduled tasks to shared session execution ([8f87467](https://github.com/ai-poet/cheaprouter-harness/commit/8f87467f6809b1dda370f43f8bf1623a025a2263))
- Fix team selectors and mobile mode controls ([f4ac27a](https://github.com/ai-poet/cheaprouter-harness/commit/f4ac27a5d95d573729be67695bef4aca9330954b))
- Complete mobile team and conversation location controls ([df9f1ef](https://github.com/ai-poet/cheaprouter-harness/commit/df9f1ef6c12db78b1871cb3fc6fb797f01f27de1))
- Soften default dark overlay borders ([1f19e9c](https://github.com/ai-poet/cheaprouter-harness/commit/1f19e9cc9e401979d937722ae91071ad8b2049ba))
- Fix mobile subtask controls and row presentation ([4c41146](https://github.com/ai-poet/cheaprouter-harness/commit/4c4114611f025fa46a41b31042a8863ff973170d))
- Fix session role captures and controller review findings ([ef61b57](https://github.com/ai-poet/cheaprouter-harness/commit/ef61b57d88c8e2486959c8dfd7375b0a53918195))
- Update validated plugin packages for stable release ([f914f56](https://github.com/ai-poet/cheaprouter-harness/commit/f914f569471eef1d16f3488022d2b83e5cf92140))
- Release Sailry 0.1.0 ([17c73d1](https://github.com/ai-poet/cheaprouter-harness/commit/17c73d1f4b4036b0b9e5f426773aa75ae96bf334))
- Wait for completion notices in Mobile activity contract ([715bb41](https://github.com/ai-poet/cheaprouter-harness/commit/715bb41a02ce324aff5934f6a842f5dc80beaf70))
- Refresh release actions and explicitly install pinned Rust ([307c7cc](https://github.com/ai-poet/cheaprouter-harness/commit/307c7ccc6287c9015cf3787cc9fb86c87bc0af19))

[Full changelog](https://github.com/ai-poet/cheaprouter-harness/compare/v0.1.0-alpha.6...v0.1.0)

## [0.1.0-alpha.3](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.0-alpha.3)

### Highlights

- Use native Turso storage with concurrent reads and short, conflict-aware
  transactions. Captured file-tool results can be persisted again without
  repeating the file operation.
- Refresh file views after checkpoint restores and release Node network
  resources on shutdown so the same endpoint can restart.
- Improve provider retry and WebSocket recovery, preserve uncertain outcomes,
  and retain useful provider error details.
- Keep plugin settings pages mounted, add Git initialization to the repository
  sidebar, and make delegated-role editors scroll completely.
- Clarify tool output and source-code headers, improve folding, and remove the
  extra partial-results footer from file changes.
- Keep structured Plan tools bundled and offer the Activity Board as a separate
  optional plugin.
- Show consistent session throughput and request latency in Statistics, with
  All selections and visible filter state. Statistics cover Sailry sessions.

### Downloads

- macOS Desktop: Apple silicon and Intel DMG installers and ZIP archives
- Sailry Host: macOS and Linux archives for arm64 and x86_64, with an installer
  and SHA-256 checksums

Storage uses new native Turso files. Existing SQLite databases are retained and
are not imported automatically.

This is an ordinary GitHub Release. Its alpha version remains in the experience
update channel.

## [0.1.0-alpha.1](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.1.0-alpha.1)

The first Sailry Harness alpha brings AI conversations and development tools into
a native workspace. Features and plugin APIs may change during alpha development.

### Highlights

- A macOS workspace with projects, conversations, files, diffs and terminals
- Local execution inside Desktop and persistent tasks on your own Sailry Host
- Plan review, guidance during execution, and delegated agents with configurable roles
- Plugins for code, research, documents, SSH and databases, with an extensible SDK
- Multiple model providers, compatible endpoints, and ChatGPT and Claude sign-in
- Document previews, conversation exports, themes, and optional speech, OCR and embedding models
- Ad-hoc macOS downloads first, followed by independent formal signing and notarization, plus Host packages for macOS and Linux

### Fixed

- Link XZ support statically so macOS packages do not depend on a developer's Homebrew libraries
- Keep background inference workers compatible with Linux systems using glibc 2.28
- Retry interrupted Apple ticket validation and recover verified successful builds during publication

### Changed

- Discover signed Desktop updates from exact-version Release catalogs and require matching Desktop and Host versions, with profile-preserving Host updates
- Reduce repeated Node and package setup in integration tests while preserving real service coverage
- Share dependency sources and compiler caches across validation and release builds within the repository cache budget

### Downloads

- Desktop: macOS 13 or later, Apple silicon and Intel
- Host: macOS and Linux, ARM64 and x64
