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

- 修复内置 Agent 使用 GPT 模型时提示词缓存经常失效、费用偏高的问题：同一个任务的请求现在都会带上会话标识，始终交给同一个上游账号处理并复用缓存；长期记忆在一个任务里只读取一次，任务进行中新写入的记忆不会再让之前的缓存全部作废，下一个任务会读到最新的记忆；待办清单的进度提醒和目标模式的进度不再写进系统提示词，每完成一项待办不会再让整段对话的缓存失效（Claude 和国产模型同样受益）；GPT 的系统提示词也不再重复发送，每次请求少约 2000 个 token

  Fixes the built-in agent's prompt cache often missing on GPT models, which made long tasks cost more: every request of a task now carries the conversation's identity, so the task stays on one upstream account and reuses its cache; long-term memory is read once per task, so a memory saved mid-task no longer throws the cache away — the next task picks the new memory up; the to-do reminder and goal progress no longer live in the system prompt, so finishing a to-do no longer invalidates the cache for the whole conversation (on Claude and the Chinese models too); and GPT no longer receives the system prompt twice, saving about 2,000 tokens per request

- 修复内置 Agent 的子 Agent 出错或被停止后，说「继续」会把它做过的事从头再做一遍的问题：子 Agent 没做完时，主 Agent 现在能看到它已经做了什么——改过哪些文件、调用过哪些工具、哪一步失败了、最后写到哪里——并从中断处接着做；按停止时已经完成的子 Agent，它的结果也不会再丢失

  Fixes "continue" redoing everything a built-in agent's sub-agent had done after it failed or was stopped: when a sub-agent does not finish, the main agent now sees what it got done — the files it changed, the tools it called, the step that failed and the last thing it wrote — and carries on from there; a sub-agent that had already finished when you pressed Stop no longer loses its report either

- 修复画图页的模型列表有时只剩 gpt-image-2 的问题：模型目录还在加载时会显示「正在加载模型」，加载失败时显示原因并可点击重试，失败后也会很快自动重试，不再把默认模型当作全部可用模型；列表没加载好之前不能提交，避免用错模型

  Fixes the image studio's model list sometimes showing only gpt-image-2: while the model catalog loads the picker says so, a failed load shows why with a retry and is retried automatically within seconds, instead of passing the default model off as the whole list; drawing waits for the list, so a picture is never drawn with a model you did not pick

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
