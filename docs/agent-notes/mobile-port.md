# Mobile Remote Controller Port Report (`feat/mobile-remote`)

从 `upstream/main`（`github.com/sailry/sailry-harness`）将远控端（Go `internal/client` 投影导出面、`internal/mobile`、`cmd/sailry-mobile` C ABI 与 Flutter `apps/mobile`）移植到 `github.com/ai-poet/cheaprouter-harness`，并按产品决策去掉云端更新/账号体系，仅保留 P2P 配对远控并完成品牌替换。

## 提交记录（每阶段一提交）

1. `dda2578e8` — `feat(client): port WatchConversationView and referenceRows transcript projection`
2. `b6e53f7bc` — `feat(mobile): port internal/mobile runtime and cmd/sailry-mobile C ABI`
3. `81f02f22d` — `feat(mobile): port Flutter app without cloud update/account dependencies`
4. `236db9e1b` — `feat(mobile): rebrand mobile app to CheapRouter Harness (cc.cheaprouter.harness)`
5. Stage 5（本提交）— `docs(mobile): add mobile build guide, Android debug workflow, and port notes`

## 一、账号 / 云端功能核查与删除清单

### 核查事实
- `internal/mobile`、`cmd/sailry-mobile` 及上游 `apps/mobile` 本身不包含用户登录、注册、账号中心、订阅或充值页面/接口（这些能力仅存在于桌面端 CheapRouter 扩展中，未引入移动端）。
- 上游 `apps/mobile` 唯一的云端/托管分发耦合为 `lib/features/updates/`（向 `https://api.github.com/repos/sailry/sailry-harness/releases` 拉取 `Sailry-android-arm64.apk` 版本并提示更新）。

### 已删除文件与依赖
- 删除目录与文件：
  - `apps/mobile/lib/features/updates/releases.dart`
  - `apps/mobile/lib/features/updates/service.dart`
  - `apps/mobile/lib/features/updates/presentation.dart`
  - `apps/mobile/test/updates_service_test.dart`
  - `apps/mobile/test/support/updates.dart`
- 删除页面/入口绑定：
  - `apps/mobile/lib/app.dart`：移除 `AppUpdates`、`UpdateDelivery` 及构造参数 `updates`
  - `apps/mobile/lib/features/settings/settings_page.dart`：移除 `UpdateSettings` 及 `updates` 参数
  - `apps/mobile/test/demo_test.dart`：移除 `support/updates.dart` 依赖与更新检查断言
- 删除未使用依赖（`apps/mobile/pubspec.yaml`）：
  - `http`、`package_info_plus`、`pub_semver`
- 删除 i18n 文案（11 个 `apps/mobile/lib/l10n/app_*.arb` 及对应 `generated/app_localizations*.dart`、`generated/lookup.dart`）：
  - `updatesVersion`、`updatesCheck`、`updatesChecking`、`updatesAvailable`、`updatesDownload`、`updatesCurrent`、`updatesUnpublished`、`updatesCheckFailed`、`updatesOpenFailed`

## 二、保留与调整的远控能力

- **Go Client 导出面（`internal/client`）**：
  - 新增 `ConversationViewOptions{ReferenceRows, Recent}` 与 `WatchConversationView(ctx, session, options)`
  - 新增 `indexTranscriptSources` 与 `transcriptSegments(..., referenceRows)` 索引化投影
  - 桌面端默认路径 `watchConversationView` 传 `ConversationViewOptions{}`（`ReferenceRows: false`），完全沿用老的内联投影行为，现有测试无回归
- **Go 移动端桥接（`internal/mobile` + `cmd/sailry-mobile`）**：
  - 完整保留 `open`、`pair`（iroh ticket）、`pair_pin`（6 位配对码）、`peers`/`saved_peers`/`forget`、`connect`、`watch*`（含 `reference_rows`/`recent`）、`prepare`/`execute`/`resolve`、`forward_port`/`forward_service`、`upload_attachment`/`download_attachment`/`download_file`、`open_command_detail`、`notifications`、`watch_usage` 与同步规则 `Sync`（`session_lanes`/`project_modes`/`select_mode`/`session_link`）
  - import 全部改写为 `github.com/ai-poet/cheaprouter-harness/...`
- **Flutter 远控 App（`apps/mobile`）**：
  - **首屏改为添加/选择主机**：`apps/mobile/lib/app.dart` 中 `_ShellState` 初始 `_index = 1`（`HostsPage`），无已配对主机时直接落在主机添加/选择页；若启动加载出已配对主机且用户未手动切页，自动进入会话列表（`TasksPage`）
  - **配对表单支持直连 iroh Ticket 与 6 位配对码**：`apps/mobile/lib/features/resources/pair_form.dart` 使用已有 `Segments` 组件提供 `pairTicket`（默认，走 `session.pair(ticket)` 直连 P2P，不依赖中继配对服务）与 `pairCode`（6 位数字，走 `session.pairPin(pin)`）两种配对方式，同步补齐 11 语言 `pairTicket` / `pairTicketInvalid` 文案
  - **核心远控工作流完整保留**：主机列表与状态监控（`HostsPage`、`HostSheet`）、会话列表与筛选/创建/归档/删除（`TasksPage`）、会话详情/消息发送/附件图片/执行轨迹/工具输出/审批与问答交互/队列管理（`features/conversations/`）、远程终端（`features/terminal/`）、项目/文件浏览/Git/端口转发（`features/resources/`）、本地通知与后台保活（`features/notifications/`、`runtime/background.dart`）、远控相关设置（语言、主题外观、强调色、通知开关、后台连接、主机用量统计与远程主机模型 Provider 管理）

## 三、包名与图标落点

- **显示名称**：`CheapRouter Harness`
  - Android：`apps/mobile/android/app/src/main/AndroidManifest.xml`（`android:label`）
  - iOS：`apps/mobile/ios/Runner/Info.plist`（`CFBundleDisplayName`、`CFBundleName`、`NSLocalNetworkUsageDescription`）、`apps/mobile/ios/Runner/zh-Hans.lproj/InfoPlist.strings`
  - Flutter / Web：`apps/mobile/lib/app.dart`（`MaterialApp.title`）、`apps/mobile/lib/l10n/app_*.arb` 与 `generated/app_localizations*.dart`（`brand`）、`apps/mobile/web/index.html`、`apps/mobile/web/manifest.json`
- **包名 / Bundle ID**：`cc.cheaprouter.harness`
  - Android `namespace` & `applicationId`：`apps/mobile/android/app/build.gradle.kts`
  - Android MainActivity：`apps/mobile/android/app/src/main/kotlin/cc/cheaprouter/harness/MainActivity.kt`（旧 `com/sailry/sailry_mobile` 已移除）
  - iOS `PRODUCT_BUNDLE_IDENTIFIER`：`apps/mobile/ios/Runner.xcodeproj/project.pbxproj`（`cc.cheaprouter.harness` 与 `cc.cheaprouter.harness.RunnerTests`）
- **图标生成（源图：`assets/branding/cheaprouter.png`，1024×1024 RGBA）**：
  - `apps/mobile/pubspec.yaml` 中 `flutter_launcher_icons` 与 `apps/mobile/test/launcher_icons_test.dart` 均指向 `../../assets/branding/cheaprouter.png`
  - Android 传统图标（5 档）：`apps/mobile/android/app/src/main/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png`（48/72/96/144/192）
  - Android 自适应前景（5 档，72% 安全区居中）：`apps/mobile/android/app/src/main/res/drawable-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher_foreground.png`（108/162/216/324/432）
  - Android / Flutter 启动图：`apps/mobile/assets/branding/{mark,splash}.png`、`drawable-*/{splash,android12splash}.png`、`drawable-night-*/android12splash.png`
  - iOS `AppIcon.appiconset` 全套 21 个图标（合成白底、无 Alpha 通道 PNG color type 2）：`apps/mobile/ios/Runner/Assets.xcassets/AppIcon.appiconset/Icon-App-*.png`
  - iOS 启动图与 Web 图标：`LaunchImage*.png`、`web/favicon.png`、`web/icons/Icon-*.png`

## 四、构建与验证结果

### 本机已通过验证（Linux x86_64，Go 1.27.3）
- `GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go build ./...`：通过
- `GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go vet ./...`：通过
- `GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0 go test -count=1 ./internal/client/... ./internal/mobile/...`：全部通过（`internal/client` 0.078s，`internal/mobile` 0.260s）
- `GOTOOLCHAIN=go1.27.3 CGO_ENABLED=1 go build -buildmode=c-shared -o /tmp/libsailry_mobile_final.so ./cmd/sailry-mobile`：通过
- `apps/mobile` 静态校验（Python 脚本，因本机未安装 Flutter/Dart SDK）：
  - 全部 322 个 `.dart` 文件的相对/包内/桥接/外部 import 校验 0 断链、0 已删除符号残留
  - 全部 11 个 locale × 725 条 i18n key、占位符、`lookup.dart` 与 `app_localizations*.dart` 一致性校验通过
  - 21 个 iOS AppIcon（尺寸 + PNG color type 2 RGB 不透明）与 10 个 Android mipmap/adaptive 图标尺寸校验通过

### 本机未运行项（环境限制）
- 本机无 Flutter SDK / Android SDK / Xcode，未能本机执行 `flutter analyze`、`flutter test`、`flutter build apk` 或 iOS 构建；已提供 `.github/workflows/mobile-android-debug.yml`（`workflow_dispatch`）用于在 GitHub Actions `ubuntu-24.04` 上运行 `flutter analyze`、`flutter test` 并产出 Android debug APK。

## 五、iOS / Android 需人工确认项

1. **刷新 `apps/mobile/pubspec.lock` 并跑 Flutter CI**：由于移除了 `http`、`package_info_plus`、`pub_semver`，首次在带 Flutter SDK 的环境运行 `flutter pub get` 后建议检查并提交更新后的 `apps/mobile/pubspec.lock`；可在 GitHub Actions 手动触发 `Mobile Android Debug` workflow 实测 `flutter analyze`、`flutter test` 与 `app-debug.apk` 产物。
2. **Android 签名与真机验证**：
   - 如需发布签名 APK，需在构建环境配置 `ANDROID_KEYSTORE_PATH`、`ANDROID_STORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` 与 `SAILRY_ANDROID_RELEASE=1`。
   - 真机安装 `app-debug.apk`，确认桌面图标、显示名 `CheapRouter Harness`、首屏进入主机页、粘贴 iroh ticket 配对、会话收发消息与审批交互正常。
3. **iOS 签名与真机验证（需 macOS + Xcode）**：
   - 在 Xcode 打开 `apps/mobile/ios/Runner.xcworkspace`，为 `cc.cheaprouter.harness` 选择 Apple Developer Team 与 Provisioning Profile。
   - 确认真机本地网络权限弹窗文案、AppIcon 图标展示以及 `libsailry_mobile.a` 静态库链接与真机配对远控正常。
4. **扫码配对（可选后续）**：当前 `PairForm` 已支持粘贴 iroh Ticket 与输入 6 位配对码，未额外引入相机权限/扫码插件依赖；如后续需要二维码扫码输入，可再评估引入扫码依赖并补 Android/iOS 相机权限声明。
5. **桌面端与现有流水线核对**：未改动 `internal/desktop/`、未改动 `.github/workflows/release.yml` 与 `test.yml`、未打 tag、未合并 `main`。
