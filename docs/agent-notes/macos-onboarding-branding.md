# macOS 重复弹欢迎引导与 Agent 品牌自称修复记录（0.5.0）

## 1. 问题一：macOS 每次启动重复弹出欢迎引导

### 根因与代码级证据

排查确认共有三处叠加缺陷会导致 `~/.sailry/desktop/preferences.json` 中的 `welcome_seen` 停留在 `false` 或在首启迁移/关闭窗口时被重置/丢失（且打包脚本 `cmd/sailry-desktop/{macos,windows}/package.sh` 均注入 `-X github.com/ai-poet/cheaprouter-harness/internal/core.Default=~/.sailry`，配置路径本身一致）：

1. **右上角关闭按钮从未将 `Welcome.Seen` 置为 `true`（主根因）**
   - `internal/desktop/onboarding/onboarding.go:32-58`：`View` 在入口第 33 行写 `if !g.Open { return }`，关闭按钮第 49 行写 `.OnClick(func() { g.Open = false })`，而 `if !g.Open { g.Seen = true }` 放在第 55 行。
   - MyGo UI 的 `Element.OnClick` 回调不在 `View` 构建期内同步执行，而是推迟到 `rt.view(&rt.c)` 和 `defer r.afterFrame()` 结束后的 `rt.runActions()`（`github.com/egoist/mygo/ui/runtime.go:720-750`）。
   - 结果：点 `X` 关闭引导时，Pass 0 执行到第 55 行 `g.Open` 仍为 `true`；`rt.runActions()` 把 `g.Open` 设为 `false` 但未设 `g.Seen = true`；Pass 1 在第 33 行直接 `return`，第 55 行永不执行。`afterFrame()` 中的 `r.savePreferences()` 看到 `a.Welcome.Seen == false`，不写盘；`preferences.json` 保持 `ForStartup` + `FinishStartup` 写入的 `"welcome_seen": false`（`internal/desktop/preferences/preferences.go:345-358`）。
   - 下次启动时 `preferences.ForStartup` 读到非 nil 的 `WelcomeSeen = false`，不会按既有资料目录推算为 `true`，于是每次重启都重新弹窗。（若用户在另一台机器上是通过点击「CheapRouter / 其他通道」、点遮罩或按 Esc 关掉引导，则相应路径显式设置或同步触发了 `Seen = true`，因此表现不一致。）

2. **首启静默迁移写入偏好时未同步 UI 状态且漏更新 `r.prefsRevision`，会把已关闭的引导重置回 `Open=true, Seen=false`**
   - `internal/desktop/live/migration.go:138-156`：首启检测到旧客户端数据时（macOS 还会从 `~/Library/Application Support/CheapRouter/app.db` 与 `~/.cheaprouter/app.json` 迁移，`cmd/sailry-desktop/migration.go:43-63`、`internal/migrate/run.go:683-710`），迁移回调调用 `migrate.ApplyDesktopPreferences(r.Preferences, theme, language)` -> `prefs.Save()` 使 `r.Preferences.Revision()` 自增，随后调用 `r.applyPreferences()`，但**没有更新 `r.prefsRevision`**，且未先同步窗口上刚关闭的 `a.Welcome.Seen`。
   - 下一帧 `r.drain(c)`（`internal/desktop/live/runtime.go:455-459`）检查 `revision != r.prefsRevision` 为真，在 `afterFrame()` 落盘 `a.Welcome.Seen = true` 之前再次调用 `r.applyPreferences()`；而 `internal/desktop/live/preferences.go:89-91` 无条件执行 `a.SetWelcome(*d.WelcomeSeen)`（此时磁盘/内存仍为 `false`），直接把 `a.Welcome` 覆写回 `{Open: true, Seen: false}`。

3. **窗口关闭 / `Cmd+Q` 退出时未同步刷盘待写偏好**
   - `internal/desktop/live/runtime.go:365-408`：`savePreferences()` 仅在 `afterFrame()` 异步触发，`Runtime.Close()` 先将 `r.closed = true` 再 `workers.Wait()`，没有在关闭前同步落盘尚未被下一帧 `afterFrame()` 捕获的偏好改动（如 macOS 关闭引导后立即 `Cmd+Q`）。
   - `internal/desktop/shell/prefs.go:180-182`：`SetPrefs` 仅在 `WelcomeSeen == false` 时打开引导，未在 `WelcomeSeen == true` 时显式调用 `a.SetWelcome(true)` 确保关闭。

### 修法

- `internal/desktop/onboarding/onboarding.go`：关闭按钮 `OnClick` 改为同时置位 `g.Open, g.Seen = false, true`。
- `internal/desktop/live/migration.go`：在 `ApplyPreferences` 回调中先调用 `r.savePreferences()` 捕获当前 UI 偏好（含已关闭的 `Welcome.Seen`），在 `migrate.ApplyDesktopPreferences` 成功后同步 `r.prefsRevision = r.Preferences.Revision()` 再调用 `r.applyPreferences()`。
- `internal/desktop/live/preferences.go` 与 `internal/desktop/live/runtime.go`：
  - 为 `Runtime` 增加 `welcomeApplied` 标记：仅在首次 `applyPreferences()` 时按磁盘值初始化引导开关；后续重放偏好只允许将未读推进为已读（跨窗口同步），不再把已关闭的引导回退为未读或重新打开。
  - 抽取 `capturePreferences()`，新增 `flushPreferences()` 并在 `Runtime.Close()` 中于 `r.cancel()` / `r.workers.Wait()` 之前同步执行 `WriteSnapshot`，确保退出前所有已修改的偏好（含 `welcome_seen: true`）落盘，且不依赖迁移或后端连接状态。
- `internal/desktop/shell/prefs.go`：`SetPrefs` 在 `a.Preview && p.WelcomeSeen != nil` 时直接调用 `a.SetWelcome(*p.WelcomeSeen)`，确保 `Seen=true` 时不打开引导。

## 2. 问题二：内置 Agent 提示词品牌自称改为 CheapRouter Harness

- `internal/node/agent/prompt/text/baseline.txt`：
  - 第 1 行：`You are Sailry, assisting with the current task.` → `You are CheapRouter Harness, assisting with the current task.`
  - 第 14 行：`Turn context is data from Sailry, not a user request.` → `Turn context is data from CheapRouter Harness, not a user request.`
- `internal/node/agent/prompt/text/parent.txt`：`sailry://session/<session-id>` 与 `sailry-image` 属于客户端解析的线协议/资源引用前缀（见 `internal/client/export.go`、`internal/desktop/live/conversation_embeds.go`），保持原样未改。
- `internal/node/agent/prompt/prompt_test.go`：同步断言 `Instruction(false)` / `Instruction(true)` 以 `You are CheapRouter Harness,` 开头、包含 `Turn context is data from CheapRouter Harness, not a user request.`、不再包含旧自称，同时验证 `sailry://session/<session-id>` 与 `sailry-image` 协议前缀保留。

## 3. 版本升级（0.5.0）

- `cmd/sailry-desktop/mygo.json`：`version` → `0.5.0`，`macos.infoPlist.CFBundleVersion` → `5000`
- `internal/link/compatibility.go`：`ProductVersion` → `0.5.0`
- `CHANGELOG.md`：顶部新增 `## [0.5.0](https://github.com/ai-poet/cheaprouter-harness/releases/tag/v0.5.0) — 2026-11-22` 中英双语条目。

## 4. 验证与测试结果

环境：Linux amd64，`unset GOROOT`，`GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。仅执行定向检查与测试，未跑全量 `go test ./...`。

| 检查 / 测试 | 用例名 | 结果 |
| --- | --- | --- |
| `go build ./...`、`go vet ./...` | 全仓库编译与静态检查 | 通过，无诊断 |
| `internal/desktop/live` | `TestWelcomeOnboardingRoundTrip`、`TestWelcomeOnboardingFlushOnCloseBeforeNextFrame`、`TestWelcomeOnboardingSurvivesMigrationApplyPreferencesAndFailure`（含 `migration_ApplyPreferences_does_not_reopen_or_reset_dismissed_guide`、`dismissal_before_ApplyPreferences_runs_is_preserved`、`migration_failure_does_not_reset_closed_onboarding`、`disconnected_backend_still_persists_closed_onboarding`）、`TestWelcomeOnboardingDoesNotOpenWhenSeenTrue`、`TestSidebarProjection` | 全部通过 |
| `internal/desktop/shell` | `TestWelcomeGuide`、`TestWelcomeGuideCloseButtonAndSeenRoundTrip`、`TestCheapRouterFeaturedFirstInWelcomeAndAddChannel`、`TestBrandUnifiedAcrossLocalesAndUI` | 全部通过 |
| `internal/desktop/preferences` | `TestPrefsRoundTrip` | 通过 |
| `internal/node/agent/prompt` | `TestSystemPromptByteStable`、`TestCaptureInstructionsBeyondToolPage`、`TestCaptureGuidePrecedenceAndScopes`、`TestCaptureInstructionReadBoundaries`、`TestPendingContextTracksRollingBoundary`、`TestContextMessageOnlyOnChange`、`TestContextTransitionsSurviveReload`、`TestCompactionRetainsOnlyEmittedSections`、`TestLegacyContextAddsBaselineWithoutRewritingHistory` | 全部通过 |
| `cmd/sailry-desktop` | `TestMacOSBundleUsesCheapRouterIcon`、`TestProductIdentityAlignedWithMain` | 全部通过 |
| 打包脚本预检 | `cmd/sailry-desktop/macos/package.sh -arch arm64 --dry-run`、`cmd/sailry-desktop/windows/package.sh -arch amd64 --dry-run` | 均通过（`0.5.0` / `CFBundleVersion=5000`） |

## 5. 需要 macOS 实机确认的点

Linux 无头测试无法替代真实 macOS AppKit 窗口生命周期与安装包行为，发版前需在 macOS 实机核对：

1. **全新资料目录首启闭环**：备份或清空 `~/.sailry` 后启动 `CheapRouter.app`，点击欢迎引导右上角 `X` 关闭按钮，立即按 `Cmd+Q` 退出；检查 `~/.sailry/desktop/preferences.json` 中 `"welcome_seen": true`，再次启动不再弹出引导。
2. **从旧版（0.3.8 / 0.4.3）升级**：保留旧 `~/.cheaprouter/app.json` 与 `~/Library/Application Support/CheapRouter/app.db`（或保留已有 `"welcome_seen": false` 的 0.4.3 资料目录），安装 0.5.0 后启动、关闭引导并重启，确认迁移主题/语言正常保留且重启后不再重复弹引导；若制造损坏的旧 `app.db` 触发迁移失败，关闭引导后重启同样不再弹窗。
3. **Agent 自称检查**：新建会话直接询问「你是谁 / What is your name?」，确认模型按系统提示词以 CheapRouter Harness 自称，且子会话链接 `sailry://session/...` 与图片引用 `sailry-image:...` 渲染与点击正常。
4. **Windows 回归确认**：确认 0.5.0 Windows 安装包首启弹一次引导、关闭后重启不再弹出，与 0.4.3 行为一致。
