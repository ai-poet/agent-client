# 「工作区操作失败」刷屏低风险修复

- 分支：`fix/workspace-toast-noise`
- 诊断依据：`/workspace/reports/workspace-action-failed-diagnosis.md`（§0、§1.1–1.3、§5）

## 1. 改动总览

### 1.1 Kit Toast 去重
- `internal/desktop/kit/list.go`：`Toast(c, outcome, message)` 设置 `ui.Toast.ID = message`（与 `kit.Message` 保持一致）。
- `internal/desktop/kit/feedback.go`：
  - `Recover(c, outcome, message, again)` 设置 `ui.Toast.ID = message`（无重试或非 Failed/Uncertain 时仍复用 `Toast`）。
  - `Error(c, title, detail)` 设置稳定 ID（`title` 与非空 `detail` 用 `\x00` 拼接），同标题同详情去重、不同详情独立显示。
- `internal/desktop/kit/toast_test.go`：新增 `TestToastDeduplicatesIdenticalCopy`，覆盖 `Toast`、`Recover`、`Error` 的同文案去重与不同文案保留。

### 1.2 工作区读取失败分流
- `internal/desktop/live/workspace_state.go` 新增 `workspaceReadFailureKey(key, err)`，在 `workspaceProject.read()` 失败上报前按错误码分流（`conflict`/`revision_conflict` → `workspace_changed` 与 `outcome_unknown` → `workspace_action_unknown` 仍由 `reportKeys` 保持）：
  - `not_found`：`file/`、`directory/` 前缀 key 已有 `entryMissing`/`fileUnavailable`/`directoryState(Failed=true)` 处理，返回空键**不再**弹通用失败提示；其它 key 保持 `workspace_action_failed`。
  - `busy`：映射到现有 `live_read_busy`。
  - `permission_denied`：映射到新增 `workspace_read_permission_failed`。
  - `unavailable` 且消息为 `resource host is offline`：保持 `resource_offline_action`。
  - 其余错误码保持 `workspace_action_failed`。

### 1.3 失败退避与轻量诊断日志
- 退避常量：`const workspaceReadBackoff = 5 * time.Second`（`internal/desktop/live/workspace_state.go`）。
- 状态字段：给 `workspaceProject` 增加 `failedAt map[string]time.Time`，失败时记录 `time.Now()`；读成功时同时清理该 key 的 `failedReads` 与 `failedAt`。
- 限流策略：
  - `readBackedOff(key)` 判断 `failedAt[key]` 存在且 `time.Since(at) < workspaceReadBackoff`。
  - `readBlocked(key)` 统一检查 `loading[key] || failedReads[key] || readBackedOff(key) || trashBlocksRead(key)`，`read()` 命中即直接返回，不重发读命令。
  - **保留 `refreshFiles()` 与 `readGit()` 现有清 `failedReads` 逻辑不动**，由 `failedAt` 独立承担 5s 限流窗口。
  - 配套边界处理：
    - `requestDirectory()`（`workspace_state.go`）与 `readGitLog()`（`workspace_git_history.go`）在置 `Loading = true` 前同样检查退避，避免 `p.read()` 因退避早退导致目录或 Git 历史永远停留在加载中状态。
    - `projectWorkspaceComposer()`（`workspace_activity.go`）在 `failedReads["git-status"] || readBackedOff("git-status")` 时保持 `composer.Unavailable`，避免 `readGit()` 清掉 `failedReads` 后退避期间短暂闪回 `Inspecting`。
    - `refresh()`（用户显式刷新）、`recoverFailedReads()`（重连/显式重试，见 `reads_recovery.go`）与 `entryFound()`（缺失路径重新出现，见 `workspace_files_removed.go`）会清理对应 `failedAt`，保证用户或恢复事件可以立即重试。
- 诊断日志：每次 `read()` 失败记录：
  ```go
  slog.Warn("workspace read failed", "key", key, "command", command.Kind, "code", faultCode(err))
  ```
  仅输出相对 `key`、命令类型和错误码，不输出文件内容、宿主绝对路径或原始 `err.Error()`。

## 2. i18n 变更

- 新增 key：`workspace_read_permission_failed`
  - `en`（`locales/en.yml`）：`Reading the workspace was refused; check file permissions`
  - `zh-CN`（`locales/zh-CN.yml`）：`工作区读取被拒绝，请检查文件权限`
  - 其余 8 种语言（`locales/application.{zh-TW,ja,ko,fr,de,es,pt-BR,ru}.yml`）均已补齐对应译文（`en`/`zh-CN` 的 `workspace_action_failed` 原先即位于 `locales/en.yml` 与 `locales/zh-CN.yml`，同文件放置以避免 `i18n/gen` 重复键校验失败）。
- 更新 `internal/desktop/i18n/keys.txt` 并执行 `go generate ./internal/desktop/i18n`，重新生成 10 份 `internal/desktop/i18n/locales/*.yml`。

## 3. 测试结果

环境：Linux，`unset GOROOT && export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0`。按约束未运行全量 `go test ./...`。

- `go generate ./internal/desktop/i18n`：退出 0。
- `go test -v ./internal/desktop/i18n/...`：PASS（`TestCatalogKeys/{CatalogsComplete,RemovedControls,SourceKeys}`、`TestPlaceholdersAndMatch`、`TestReadLang` 全部通过）。
- `go test -v ./internal/desktop/kit -run 'Toast'`：PASS（含新增 `TestToastDeduplicatesIdenticalCopy`，32.0s）。
- `go test -v ./internal/desktop/live -run 'Toast|Workspace'`：PASS（2.4s，含新增 `TestWorkspaceReadFailureClassification`、`TestWorkspaceReadBackoffToastAndLogging`）。
- `go test -v ./internal/desktop/live -run 'TestWorkspaceRead|TestPanelProjection'`：PASS（24.9s）。
- `go test -v ./internal/desktop/live -run '/workspace_uploads|/workspace_git_busy|/queued_read_repeated'`：PASS（23.7s；该筛选也触发了包内无子测试过滤的其它单元测试，全部通过，现有断言无需削弱）。

## 4. 遗留与后续建议

1. **Node 侧 `readOnly` 命令仍不落 Node 日志**：`internal/node/logging/logging_commands.go` 对 `read_file`、`read_git_log`、`list_git_branches`、`read_git_diff` 等 `readOnly` 命令仍不建 span，因此 Node 侧 `current.log` 仍看不到这些命令的失败记录。本次已在桌面客户端 `workspaceProject.read()` 补了 `slog.Warn`，使桌面标准日志/错误输出具备 `key + command + code` 线索；若后续需要统一在 Node 的 `current.log` 检索所有读失败，建议把 `readOnly` 命令改为「仅失败或慢请求记录轻量日志」（类似 `slow` 策略），以免成功热路径刷日志。
2. **Git `not_configured` 与其它高频原因文案**：本次保持范围收敛，未改 `inspect_git` 的 `not_configured` 专属提示（如「未找到 Git」）、未调整 `readGit()` 每次重置 `locationsLoaded` 的行为，也未触及 Save all（`closing.go`）、会话分叉/移动（`workspace_forks.go`）、写文件 ENOSPC/只读文案及未使用 worktree 清理，均留作后续独立任务处理。
3. **跨平台验证**：本次仅在 Linux 跑 Go 单元测试（`CGO_ENABLED=0`），macOS TCC / Windows Defender 权限拒绝的真实桌面行为需在对应平台验收。
