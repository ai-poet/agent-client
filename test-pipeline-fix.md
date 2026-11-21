# Tests 流水线修复报告

## A. 原生库与 CRT

- **CI 配置**：`.github/workflows/test.yml` 在 macOS ARM64 和 Windows x64 上安装固定版本 Rust 1.98.0 及对应目标，缓存 Cargo 依赖与构建目录，然后运行 `native/{iroh,turso,capture}/build.sh`。Windows 另暂存 MSVC CRT 并把 DLL 目录加入 `PATH`。Go 测试通过 `SAILRY_TURSO_LIB` 读取对应平台的库；iroh 和 capture 使用现有开发期暂存目录查找机制。
- **测试**：`native/fetch/windows_crt_test.go` 移除了已删除的 ONNX Runtime 锁文件依赖，仍检查 Desktop、Host 封印和不完整 CRT 被拒绝。此分支不再打包 ONNX Runtime。

## B. Go 测试接口

- **测试**：`internal/desktop/live/session_overview_test.go` 按 `InventoryVersion` 触发后台刷新，保留加载、翻页与重试断言。
- **测试**：`internal/node/logging_configuration_test.go` 为拒绝日志传入命令名；`internal/node/process/{process_test,stall_test}.go` 检查 `Wait` 的退出结果；`internal/node/web/search_test.go` 检查过滤后的 URL 和域名过滤标志。

## C. 行为与 UI

- **实现**：`internal/desktop/composer/models.go` 按触发控件上下的可用空间计算模型选择器高度，保留矮窗口的滚动。960 和 440 DIP 两种宽度都用原有面板边界断言验证。
- **Shell 测试**：`internal/desktop/shell/{frame_test,shortcuts_test}.go` 校验创作工作台的菜单、排序和偏好恢复；`rail.go` 同步说明。`branding_test.go` 仅放行 About 及其上层欢迎对话框里可见的归属文案和链接，其它界面仍检查品牌。`diff_panel_ux_test.go` 校验实际打开文件的选中态，从空编辑器用普通 Tab 导航到 Git 并验证 Enter/Space；Shift+Tab 仍是编辑器的模式快捷键。`composer_models_test.go` 和 `shell_test.go` 验证不支持推理的模型不显示档位控件。
- **视口相关测试**：`internal/desktop/acceptance/acceptance.go` 的无头演练先滚动到目标会话行再点击。`internal/desktop/shell/{project_menu_test,panes_test,sidebar_test,shell_test,shortcuts_test,composer_activity_test}.go` 复用侧栏行的滚动定位，`conversation_layout_test.go` 从实际列表边界定位滚动条。未放大测试窗口或改变生产侧栏布局。
- **演示数据与测试**：`internal/desktop/preview/demo_waiting.go`、`demo/{en,zh-CN}.yml`、`demo_test.go` 补齐当前问题和历史回执的独立检查操作，不重发已提交的答案。`internal/desktop/plugins/runtime_test.go` 按按钮角色核对真实的 1–9 页，不再把按钮和可见文字重复计数。
- **逻辑测试**：`internal/node/agent/model/{context_test,capture_fixtures_test}.go`、`internal/node/providers/model_limits_test.go` 对齐同名条目的容量最大值、有效容量下限和支持的档位；`internal/node/agent/engine/{instructions_test,turn_test}.go` 用有效容量覆盖压缩阈值和超量指令拒绝。`internal/node/process/stall_test.go` 用虚拟时钟覆盖仅需后续跟进的命令、首个成功记录的停滞和进程结束。`internal/node/store/{rust_archive_test,storage_path_test}.go` 验证历史归档不阻止新库初始化，原归档和 SQLite 文件不变。`internal/update/update_test.go` 覆盖显式空配置、公钥不匹配以及 macOS `.app` 内的原生更新探针。`ARCHITECTURE.md` 同步归档和更新配置的现行约定。

## 验证

- **本地 Linux**：Go 1.27.3、`CGO_ENABLED=0`，通过 `go vet ./...`、`go build ./...`、变更文件格式及 `git diff --check`。Tests YAML 可解析，三个原生构建脚本通过 `sh -n`。
- **定向测试**：运行了上述改动涉及的既有 Go 测试，包括回执演示、品牌与 About 链接、真实侧栏滚动与点击、Diff 键盘操作、两种宽度的模型选择器、更新器、模型与供应商规则、压缩、Turso 存储及 CRT 封印。Turso 测试复用了代码移动前已构建的 Linux 动态库，原生源码和锁文件未变，未重编。
- **交给 GitHub Actions**：macOS、Windows 上的原生构建、DLL/动态库实际加载、Cargo 缓存和 `go test ./...`、`go build ./...`。按要求没有在本机运行全量测试；无头交互不等同于原生窗口验收。
- 未改动 tag、`.github/workflows/release.yml` 或 `CHANGELOG.md`。
