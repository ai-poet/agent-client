# 更新日志 / Changelog

本文件是**应用内更新提示所显示的发版说明的唯一来源**：[`scripts/release.ts`](scripts/release.ts)
取出标题与待发布版本号（`MARKETING_VERSION`）一致的那一节，随更新一起发布，Sparkle
在更新提示里显示它；发版流水线也把同一节写进 GitHub release，官网的更新日志从那里读取。

All notable changes to Waku. This file is the **source of truth for the release
notes shown in the in-app updater**: [`scripts/release.ts`](scripts/release.ts)
extracts the section whose heading matches the version being released
(`MARKETING_VERSION`) and publishes it next to the update, so Sparkle shows it in
the update prompt. The release workflow writes the same section into the GitHub
release, which the website's changelog reads.

格式遵循 [Keep a Changelog](https://keepachangelog.com)：每次发版在顶部新增一节
`## [<版本号>]`，与 `Cargo.toml` 中的版本一致。每一条先写中文，空一行、缩进两格再写英文——
两种语言同属一条，官网和更新提示会把它们显示在一起。

Format follows [Keep a Changelog](https://keepachangelog.com). Add a new
`## [<version>]` section at the top for each release, matching the version in
`Cargo.toml`. Write each entry in Chinese first, then, after a blank line and
indented two spaces, in English — both languages stay one entry, shown together
on the website and in the update prompt.

发版说明写给最终用户看，不是开发过程记录。功能尚未发布时，它的修复与改进并入原条目，不要另起新条目。

Write release notes for the final product users receive, not the development
history. When a feature is still unreleased, fold its fixes and refinements into
the original feature bullet instead of adding separate entries for them.

## [unreleased]

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

## [0.2.0]

- 自定义端点改为配置档：每个 CLI 可以保存多个（网关、官方密钥、其他中转），在「设置 → 服务商」里切换

  Custom endpoints are now profiles: keep several per CLI (the gateway, an official key, another relay) and switch between them from Settings → Providers

- 可为端点添加备用域名，一次测完所有域名，并通过响应最快的那个转发——也可以设为自动选择

  Add alternate domains to an endpoint, measure them all at once, and route through whichever answers fastest — automatically, if you ask it to

- 可在「设置 → CheapRouter 账号」中选择 Agent 通过服务的哪个域名访问。登录仍停留在你登录时使用的域名，某个域名不再响应时会自动回退

  Pick which of the service's domains the agents reach it on, from Settings → Cloud Account. Signing in stays on the domain you signed in with, and a domain that stops answering falls back on its own

- 可从端点本身获取它的模型列表

  Fill an endpoint's model list from the endpoint itself

- 可在提示覆盖路由的环境变量的警告里直接移除这些变量。移除前会先备份，可随时恢复；Windows 的系统级变量仍会给出需以管理员身份运行的命令

  Remove the environment variables that override routing, from the warning that reports them. Values are backed up first and can be restored; machine-wide Windows variables still hand you the command to run as administrator

- 可按平台开启自动故障转移：当前分组报告故障时切换到健康的分组，原分组恢复后再切回

  Optional automatic failover per platform: when the group you are on reports an outage, switch to a healthy one and switch back once yours recovers

- 模型目录：Codex 列表中 GPT-6-Sol 取代 GPT-5.6-Luna，Claude Code 新增 Claude Opus 5.5，Grok 新增 Grok 4.7（推理强度最高到 extra-high）

  Model catalogs: GPT-6-Sol replaces GPT-5.6-Luna in the Codex list, Claude Opus 5.5 added for Claude Code, and Grok 4.7 (up to extra-high effort) added for Grok

- 账号页和账户菜单显示你的订阅：剩余天数，以及每日、每周、每月额度的用量

  Your subscriptions on the account page and in the account menu: days left, and spend against each daily, weekly and monthly limit

- 内置 Agent 让每个模型走服务它的那个分组：DeepSeek、Kimi、GLM 等模型使用你的订阅，而不是 Codex 分组（它会回复「no available channel」）。模型选择器会注明模型经由哪个订阅

  The built-in agent sends each model through the group that serves it: DeepSeek, Kimi, GLM and the like use your subscription instead of the Codex group, which answered them with "no available channel". The model picker says which subscription a model goes through

- 未安装驱动时开启「电脑操作」不再中断对话：Agent 在没有桌面控制的情况下继续，内置 Agent 仍可生成图片

  Turning on Computer Use without its driver installed no longer stops a conversation: the agent carries on without desktop control, and the built-in agent still generates images

- 不再发布 Linux 版本

  Linux builds are no longer published

- Codex 的会话标题和提交信息改用 GPT-5.6-Terra 生成

  Codex session titles and commit messages are generated with GPT-5.6-Terra

- 内置 Agent 不再在十步之后停止任务

  The built-in agent no longer stops a task after ten steps

- 内置 Agent 中的 DeepSeek、GLM（4.5 及以后）和 Kimi K3 模型可选择推理强度：低、高和最高

  DeepSeek, GLM (4.5 and later) and Kimi K3 models in the built-in agent have a reasoning effort choice: low, high and max

- 按服务文档的方式配置 Codex：GPT-5.6-Sol 搭配用于审查的 GPT-5.6-Terra，开启图片生成，密钥只保存在 `config.toml` 中——`auth.json` 里你的 ChatGPT 登录不再被替换，被旧版本替换掉的也会还给你

  Codex is set up the way the service documents it: GPT-5.6-Sol with GPT-5.6-Terra for reviews, image generation on, and the key kept in `config.toml` only — your ChatGPT sign-in in `auth.json` is no longer replaced, and one replaced by an earlier version is given back

## [0.1.21]

- 修复仅与守护进程的连接断开时，应用却彻底失去守护进程的问题（「Waku daemon disconnected」提示每隔几秒出现一次，直到重启应用）：应用现在会重新连上仍在运行的守护进程，运行中的任务继续进行。重连期间顶部有一条横幅说明情况，耗时过长时提供重启；其他地方不再重复提示，期间未能保存的状态和草稿会在连接恢复后写入

  Fix the app losing its daemon for good when only the connection to it dropped (a "Waku daemon disconnected" notice that came back every few seconds until a relaunch): the app now reconnects to the still-running daemon and running tasks carry on. While it reconnects, a strip across the top says so and offers a restart if it takes too long, nothing else repeats the notice, and state and drafts that could not be saved meanwhile are written once the connection is back

## [0.1.20]

- 修复应用在草稿发送前就对其作出反应的问题：在输入框里打字可能让任务出现在侧栏、替换空白状态，并且每输入几个字符就弹出一次「Waku daemon disconnected」。现在打字时仍会预热提供商，但在按下回车前会话不会有任何变化，为其他模型或模式启动的预热进程会被丢弃而不会被使用

  Fix the app reacting to a draft before it is sent: typing in the composer could make the task appear in the sidebar, replace the empty state, and raise a "Waku daemon disconnected" notice once per few characters. The provider is still warmed up while you type, but nothing about the session changes until you press Enter, and a warm process started for a different model or mode is discarded rather than used

- Agent 没有回答就结束一轮时会明确说明：没有产出的一轮会报告 Agent 写到错误输出的内容，并把原因显示在任务的失败标记上，而不是只显示「Turn completed」

  An agent that ends a turn without answering now says so: a turn that produces nothing reports whatever the agent wrote to its error output, with the reason on the task's failure badge, instead of the bare "Turn completed" line

## [0.1.19]

- 终端、文件、浏览器和审查按钮移到窗口标题栏：点一下打开对应界面、切到它的标签页，已在最前时则隐藏面板。快捷键 ⇧⌘T / E / O / D（Windows 和 Linux 上为 Ctrl+Shift），也可从「视图」菜单和命令面板打开；旧的右侧面板开关按钮已移除（⇧⌘B 仍可开关面板）

  Terminal, Files, Browser and Review buttons now sit in the window header: one click opens the surface, switches to its tab, or hides the panel when it is already in front. Shortcuts ⇧⌘T / E / O / D (Ctrl+Shift on Windows and Linux), plus View-menu and command-palette entries; the old right-panel toggle button is gone (⇧⌘B still toggles the panel)

- 可在「设置 → 通用」中检查更新；更新就绪时窗口顶部出现横幅，点击即可安装

  Check for updates from Settings → General; when an update is ready a banner appears across the top of the window and installs on click

- 服务商设置重做为每个 CLI 一张卡片：安装状态、「已安装但无法运行」诊断、Node/npm 运行环境状态、环境变量冲突警告，以及更清晰的自定义端点表单（带标签的字段、隐藏密钥、URL 校验、端点测试及结果、清空前确认）

  Providers settings rebuilt as one card per CLI: install status, "installed but not runnable" diagnostics, Node/npm runtime status, environment-variable conflict warnings, and a cleaner custom endpoint form (labelled fields, hidden keys, URL validation, an endpoint test with its result, confirmation before clearing)

- CLI 检测现在会查找你的 shell 所查找的位置（登录 shell 的 PATH，以及 nvm/fnm/volta/pnpm/scoop 目录），并区分「未安装」和「已安装但损坏」；npm 安装完成后会验证安装结果，权限和网络失败时给出具体提示

  CLI detection now looks where your shell does (login-shell PATH, nvm/fnm/volta/pnpm/scoop directories) and tells "not installed" from "installed but broken"; installs are verified after npm finishes, with specific hints for permission and network failures

- 新安装的引导清单：登录、安装 CLI、打开项目——会记住完成和关闭状态

  Onboarding checklist for new installs: sign in, install a CLI, open a project — remembers completion and dismissal

- 无法回退时，可编辑并重新发送已发送的消息（铅笔按钮和右键菜单）

  Edit and resend a sent message when rewind is not available (pencil button and context menu)

- 失败的任务在侧栏显示失败标记，悬停可看错误，并提供删除按钮（删除前会确认）

  Failed tasks show a failure badge in the sidebar with the error on hover, and a remove button that asks before removing

- 修复一段时间后被登出云账号的问题；会话过期时现在会明确说明，并恢复本地 CLI 配置

  Fix being signed out of the cloud account after a while; an expired session now says so and restores the local CLI configs

- 首条消息更快：本轮快照复用仓库索引，提供商与之并行启动，打字时预热 CLI

  Faster first message: the turn snapshot reuses the repository's index, the provider starts in parallel with it, and the CLI is prewarmed while you type

- Codex 模型列表跟随所选云分组：切换分组会丢弃 Codex 缓存的清单并立即重新探测 CLI

  The Codex model list follows the selected cloud group: switching a group drops Codex's cached manifest and re-probes the CLI right away

- 模型目录：新增 Claude Fable 5.1 以及 CLI 未报告的精选 Claude 条目；Codex 目录新增 GPT-6-Astra

  Model catalogs: Claude Fable 5.1 and curated Claude entries the CLI does not report; GPT-6-Astra added to the Codex catalog

- Codex、Claude Code 及其他提供商支持 /resume；侧栏会把选中的任务滚动到可见位置；环境摘要中显示实时工作指示

  /resume for Codex, Claude Code and the other providers; the sidebar scrolls the selected task into view; live work indicator in the environment summary

- 修复 Claude 模型发现（#185），以及导航栏滚动误传到对话记录的问题

  Fix Claude model discovery (#185) and navigation-rail scrolls reaching the transcript

- 修复在 Linux 和 macOS 上，CLI 的 shell 脚本留下子进程时，CLI 探测会一直挂到超时的问题

  Fix a CLI probe hanging for the full timeout on Linux and macOS when the CLI's shell script left a child running

## [0.1.18]

- 修复 Windows 上每次云账号请求（余额刷新、公告、分组状态、支付轮询）都会闪出控制台窗口的问题

  Fix a console window flashing on Windows for every cloud-account request (balance refresh, announcements, group status, payment polling)

## [0.1.17]

- 修复 Codex 会话因地区受限的 OpenAI 文档 MCP 服务器而报告启动错误的问题

  Fix Codex sessions reporting a startup error for the geo-blocked OpenAI docs MCP server

- 登录后每个平台自动走其第一个可用分组；移除「账号默认」选项

  Sign-in now routes each platform through its first available group automatically; the "account default" option is removed

- 修复起始快照引用在一轮中途消失（Agent 执行 git gc、回退清理）时，对话因「turn starting checkpoint … is unavailable」失败的问题；检查点现在会回退到上一轮的差异基准

  Fix turns failing with "turn starting checkpoint … is unavailable" when the starting snapshot ref disappears mid-turn (agent-run git gc, rewind cleanup); the checkpoint now falls back to the previous turn's diff base

## [0.1.16]

- Codex 线程目标：输入 /goal 设定一个任务会持续追求的目标——第一条消息前后都可以——其自主推进过程实时显示在对话记录中，状态标签显示实时预算或已用时间，还可在对话框中编辑、暂停、恢复或清除目标（Waku Web 同样支持）

  Codex thread goals: type /goal to set a persistent objective the task keeps pursuing — before or after the first message — with its autonomous pursuit streaming into the transcript, a status chip showing live budget or elapsed time, and a dialog to edit, pause, resume, or clear the goal (also in Waku Web)

- 修复 Codex 会话在较新版本的 Codex CLI 上无法启动的问题（「invalid transport」配置错误）

  Fix Codex sessions failing to start on newer Codex CLI versions ("invalid transport" config error)

- 修复 Grok Build 和其他 ACP Agent 在 Windows 上无法启动的问题（找不到路径）

  Fix Grok Build and other ACP agents failing to launch on Windows (path not found)

- 从已安装的 Agent CLI 中发现其原生斜杠命令和技能，包括多行 YAML 描述

  Discover provider-native slash commands and skills from installed agent CLIs, including multiline YAML descriptions

- 为 Grok 增加推理强度选择

  Add reasoning effort selection for Grok

- 连接中断后自动重连远程守护进程会话

  Reconnect remote daemon sessions automatically after connection interruptions

- 修复提供商开始流式输出后 Command/Ctrl+Enter 引导失效的问题

  Fix Command/Ctrl+Enter steering after a provider response starts streaming

- 修复 Windows 上对话记录中的文件链接

  Fix transcript file links on Windows

- 修复 OpenCode 丢失第一个流式事件，以及在 Windows 上取消时卡住的问题

  Fix OpenCode dropping the first streamed event and hanging during cancellation on Windows

## [0.1.14]

- 侧栏任务可按项目或更新日期分组、按最新或最早排序，并可折叠分组

  Group sidebar tasks by project or update date, order them newest or oldest first, and collapse sections

- 页内查找：用 cmd-f 或 ctrl-f 按关键词搜索整个对话记录

  Find in page: Search the full transcript by keywords using cmd-f or ctrl-f

- 用 Ctrl+Tab 和 Ctrl+Shift+Tab 在最近的任务之间切换

  Switch between recent tasks with Ctrl+Tab and Ctrl+Shift+Tab

- 新任务沿用当前访问模式，并在重启后记住

  Carry the current access mode into new tasks and remember it between launches

- 修复 OpenCode 访问模式权限，恢复会话时还原待处理的权限提示

  Fix OpenCode access-mode permissions and restore pending permission prompts when resuming sessions

- Codex 的文件读取、列目录和搜索显示为文件活动，而不是原始命令

  Show Codex file reads, listings, and searches as file activity instead of raw commands

- 过长的面板和后台任务标题保持在一行并截断显示

  Keep long panel and background-work titles on one truncated line

- 提高最小界面字号，更易阅读

  Increase the minimum UI text size for better legibility

## [0.1.13]

- 新增 Vercel Fx 支持

  Add Vercel Fx support

- 支持 DeepSeek Harness 0.1.1，无需打开其网页界面

  Support DeepSeek Harness 0.1.1 without opening its web UI

- 进行中的一轮产生新输出时，折叠更早的活动分组

  Collapse earlier activity groups when a running turn moves on to newer transcript output

## [0.1.12]

- 用 Codex、Pi 和 Oh My Pi 的原生语法调用它们的技能

  Invoke Codex, Pi, and Oh My Pi skills with their native syntax

- 实时显示 Claude 后台任务的输出

  Stream live output from Claude background tasks

- 在空输入框中按 Command/Ctrl+Enter，用最早排队的后续消息引导任务

  Steer the oldest queued follow-up with Command/Ctrl+Enter in an empty composer

- 修复 Cursor 的模型和推理选项选择

  Fix model and reasoning option selection for Cursor

- 修复 Windows 上通过 npm 安装的提供商检测

  Fix npm-installed provider detection on Windows

- 修复守护进程终端会话在关闭时卡住的问题

  Fix daemon terminal sessions hanging during shutdown

- 从 Codex 分叉会话复制来的历史不再计入用量统计

  Exclude copied history from forked Codex sessions from usage totals

- Codex 的不同推理段落分行显示

  Keep separate Codex reasoning sections on separate lines

## [0.1.11]

- 文件编辑器支持 Markdown 高亮，可在源码与渲染预览之间切换

  Highlight Markdown in the file editor, and toggle between source and a rendered preview

- 新增界面字号和代码字号设置

  Add UI and code font size settings

- macOS：新增「打开方式」按钮，用选定的应用打开项目文件夹

  macOS: Add "Open in.." button to open project folder in selected application

## [0.1.10]

- 新增 Kimi Code 支持

  Add Kimi Code support

- 新增 Oh My Pi 支持

  Add Oh My Pi support

- 修复 Markdown 表格渲染

  Fix markdown table rendering

## [0.1.8]

- 修复 Windows 上的 `PATH` 解析

  Fix `PATH` resolution on Windows

## [0.1.4]

- 修复差异视图中的文本选择

  Fix text selection in diff view

## [0.1.3]

- Codex 和 Claude 的提交信息生成固定使用低价模型：gpt-5.6-luna 和 claude-4.5-haiku

  Pin Codex and Claude commit message generation to cheap models: gpt-5.6-luna and claude-4.5-haiku

- 侧栏加入动画

  Animate sidebars

- 在对话记录中以内联差异显示提供商的文件编辑

  Render provider file edits as inline diffs in the transcript

- 修复 Claude 任务标题生成

  Fix claude task title generation

## [0.1.2]

- 修复回归问题：用户消息气泡应适应内容宽度

  Fix regression: user bubble should fit its content width

## [0.1.1]

- 嵌套 Markdown 使用完整的消息宽度

  Give nested Markdown the full message width

- 限制输入框高度，超出部分用悬浮滚动条滚动

  Cap composer height and scroll overflow with an overlay scrollbar

- 拖动选择文本超出输入框边界时仍能继续选择

  Keep drag-selecting text past the input bounds

- 修复滑动实时推理窗口时的字符边界崩溃

  Fix char boundary panic when sliding the live reasoning window

## [0.1.0]

- 新增独立的 Waku 守护进程和浏览器客户端

  Add standalone Waku daemon and browser client

- 新增 Linux 支持（X11 和 Wayland，目前需要从源码构建）

  Add Linux support (X11 and Wayland, you need to build from source for now)

- 直接在输入框中回答 Agent 的提问

  Answer agent questions directly in the composer

- 排队的后续消息重新设计为输入框卡片，可逐条引导

  Redesign queued follow-ups as composer cards with per-message steering

- 新增 DeepSeek Agent 预设选择（Standard、Code、Minimal 和 Creator）

  Add DeepSeek agent preset selection (Standard, Code, Minimal, and Creator)

- 新增 Claude 上下文窗口和 ultracode 推理强度选项

  Add Claude context window and ultracode effort options

- 新增 /fast 命令，为 Codex 开关快速模式

  Add /fast command to toggle fast mode for Codex

- 在实时对话记录的标题中显示最新活动

  Show the latest activity in live transcript headers

- 新增软换行和键盘复制反馈

  Add soft wrapping and keyboard copy feedback

- 终端新增悬浮滚动条，并按字体测量字符宽度

  Add terminal overlay scrollbar and measure cell width from the font

- 重启后恢复窗口位置、大小和所在显示器

  Restore window position, size, and display across launches

- 活动和命令输出视图中的滚轮滚动不再外溢

  Contain wheel scrolling in activity and command output viewports

- Markdown 流式输出更流畅，并降低流式输出时的 CPU 占用

  Smooth streaming markdown and reduce CPU usage while streaming

## [0.0.13]

- 新增 DeepSeek Harness 提供商

  Add DeepSeek Harness provider

- 用户消息按 Markdown 渲染，裸 URL 自动转为链接

  Render user message as Markdown and linkify bare URLs

- 同一工作区的各会话共用一个常驻的 OpenCode serve

  Share one resident OpenCode serve per workspace across sessions

## [0.0.12]

- 提供商命令继承登录 shell 的环境变量

  Inherit the login-shell environment for provider commands

- 修复切换提供商时的模型特性

  Fix model traits across provider switches

- 分支改动计数保持最新，并包含未跟踪的文件

  Keep branch change counts current and include untracked files

- 规范提供商子进程的 SIGCHLD 处理

  Normalize SIGCHLD for provider children

- 修复 Grok 模型发现

  Fix Grok model discovery

## [0.0.11]

- 修复通过 nvm、fnm 等 shell PATH 管理器安装的 CLI 的提供商检测

  Fix provider detection for CLIs installed through shell PATH managers such as nvm and fnm

- 显示 Pi 扩展注册的模型

  Show models registered by Pi extensions

- 修复在搜索框输入空格时模型选择器被关闭的问题

  Fix the model picker closing when entering a space in search

- 修复恢复 ACP 会话时对话历史重复和交互模式丢失的问题

  Fix duplicate transcript history and lost interaction mode when resuming ACP sessions

## [0.0.10]

- 修复输入法组字导致的崩溃

  Fix crash due to IME composition

- 修正错别字

  Fix typo

## [0.0.9]

- 用量浮窗新增 OpenCode Go 支持

  Add OpenCode Go support in usage popover

- 修复应用图标

  Fix app icon

- 修复 Cursor 模型检测

  Fix Cursor model detection

## [0.0.8]

- 首个版本

  Initial release
