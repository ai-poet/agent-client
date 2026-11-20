# CheapRouter 发布操作手册（fork 版）

上游流程说明在 [../RELEASING.md](../RELEASING.md)；本文是 fork 的实际操作步骤，
域名/命名已按品牌修正（`releases.cheaprouter.cc`、`cheaprouter-releases` 桶、
`CheapRouter-*` 产物名）。按顺序做，一次性步骤只做一遍。

## 阶段 1（一次性）：生成自己的 Sparkle 密钥 ← 最先做

`resources/Info.plist` 里的 `SUPublicEDKey` 目前还是上游的公钥，你没有对应
私钥 → 无法签名更新源，客户端也只信这把钥匙。在 **发布用的 Mac** 上：

```sh
# 下载 Sparkle 工具（或跑一次 bundle.sh 后用 .waku-cache/sparkle/<版本>/bin）
# https://github.com/sparkle-project/Sparkle/releases 解压后：
./bin/generate_keys
# 输出 "A key has been generated and saved in your keychain" + 公钥字符串
./bin/generate_keys -p          # 再次打印公钥
./bin/generate_keys -x sparkle_private_key.txt   # 导出私钥做备份
```

然后：
1. 把公钥粘进 `resources/Info.plist` 的 `SUPublicEDKey`（替换上游值）并提交。
2. `sparkle_private_key.txt` 存进密码管理器（**丢失 = 存量用户永远无法更新**），
   内容同时填到 GitHub 仓库 secret `SPARKLE_PRIVATE_KEY`。
3. 本机保留 keychain 里的那份（`bun run release` 直接用）。

## 阶段 2（一次性）：Cloudflare R2 更新源

1. Cloudflare 控制台 → R2 → Create bucket → 名字 **`cheaprouter-releases`**。
2. 进入桶 → Settings → Custom Domains → 绑定 **`releases.cheaprouter.cc`**
   （域名在 Cloudflare 托管则自动出证书；绑定后对象可公开访问）。
3. R2 → Manage API Tokens → 建一个 **Object Read & Write** token，范围含这个桶。
   记下 Access Key ID / Secret Access Key / 账户 ID。
4. 发布机配置 rclone（`~/.config/rclone/rclone.conf`）：

```ini
[r2]
type = s3
provider = Cloudflare
access_key_id = <ACCESS_KEY_ID>
secret_access_key = <SECRET_ACCESS_KEY>
endpoint = https://<ACCOUNT_ID>.r2.cloudflarestorage.com
no_check_bucket = true
```

验证：`rclone lsf r2:cheaprouter-releases --s3-no-check-bucket`（空输出=通）。

### 阶段 2 实际部署：Zeabur MinIO（已配置完成 ✅）

更新源用的是部署在 Zeabur 的 MinIO，S3 API 与公开下载共用
`https://s3.cheaprouter.cc`，无需反代：

- 桶 **`cheaprouter-releases`** 已创建，匿名只读（GetObject）策略已设好并
  实测通过；写操作仍需密钥。
- 下载基址 = **`https://s3.cheaprouter.cc/cheaprouter-releases`**（路径式），
  客户端 `RELEASES_BASE_URL`、`Info.plist` 的 `SUFeedURL`、appcast 默认下载
  前缀均已指向它。
- 还剩两件事：发布 Mac 配 rclone（见下）；CI secrets 填
  `R2_ENDPOINT=https://s3.cheaprouter.cc`、`R2_PROVIDER=Minio`、
  `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`（MinIO 的账号密码）。

发布 Mac 的 `~/.config/rclone/rclone.conf`：

```ini
[r2]
type = s3
provider = Minio
access_key_id = <MinIO 用户名>
secret_access_key = <MinIO 密码>
endpoint = https://s3.cheaprouter.cc
no_check_bucket = true
```

验证：`rclone lsf r2:cheaprouter-releases --s3-no-check-bucket`。

以下是当初的通用自建方案，留作参考（自己服务器 + Caddy 时用）。

1. 服务器上（可并入现有 deploy compose）：

```yaml
  minio:
    image: minio/minio
    command: server /data
    environment:
      MINIO_ROOT_USER: <管理账号>
      MINIO_ROOT_PASSWORD: <管理密码>
    volumes:
      - ./minio-data:/data
```

2. 建桶并开放匿名只读下载（写仍需密钥）：

```sh
mc alias set self https://s3.cheaprouter.cc <管理账号> <管理密码>
mc mb self/cheaprouter-releases
mc anonymous set download self/cheaprouter-releases
```

3. Caddy 两个站点：

```caddy
s3.cheaprouter.cc {
    reverse_proxy minio:9000
}
releases.cheaprouter.cc {
    rewrite * /cheaprouter-releases{uri}
    reverse_proxy minio:9000
}
```

4. 发布 Mac 的 rclone remote 改为：

```ini
[r2]
type = s3
provider = Minio
access_key_id = <管理账号或专用AccessKey>
secret_access_key = <对应Secret>
endpoint = https://s3.cheaprouter.cc
no_check_bucket = true
```

5. CI secrets 用 `R2_ENDPOINT=https://s3.cheaprouter.cc`、`R2_PROVIDER=Minio`
   替代 `R2_ACCOUNT_ID`（sync-release.yml 已支持）。

注意：更新包的防篡改靠 EdDSA 签名，服务器被攻破也推不了恶意更新；但桶要
**定期备份**——旧版本 zip 是增量补丁和跨版本升级的原料，且更新源宕机期间
老用户收不到新版本（应用本身不受影响）。

## 阶段 3（可选）：Apple 签名与公证

**当前走的是免证书的 ad-hoc 路线**：CI 的 mac job 检测不到
`APPLE_CERTIFICATE` secret 时自动以 `--adhoc` 构建（ad-hoc 签名、不公证），
产物照常出（DMG、zip、签名 appcast）。用户通过一键脚本安装：

```sh
curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
```

curl 下载不打 quarantine 隔离属性，所以 ad-hoc 应用零弹窗直接打开；后续
升级走 Sparkle（验我们自己的 EdDSA 签名，与公证无关）。脚本源码在
`scripts/install-mac.sh`，改动后需重新上传到桶（key `install-mac.sh`）。
浏览器直接下载 DMG 的用户会遇到 Gatekeeper 拦截，需在
系统设置 → 隐私与安全性 点"仍要打开"——下载页应主推脚本方式。

以后加入 Apple Developer Program 后按下述配置证书与公证，CI 检测到
secrets 自动转正，存量 ad-hoc 用户经 Sparkle 无缝升级到公证版本。

1. 加入 [Apple Developer Program](https://developer.apple.com/programs/)（$99/年）。
2. 在发布 Mac 的 Xcode（Settings → Accounts → Manage Certificates）创建
   **Developer ID Application** 证书。
3. [appleid.apple.com](https://appleid.apple.com) 生成一个 App 专用密码，然后：

```sh
xcrun notarytool store-credentials NOTARY \
  --apple-id 你的AppleID邮箱 --team-id 你的TeamID
```

4. 装工具：`brew install bun create-dmg rclone`。
5. `cp .env.example .env`，填 `WAKU_SIGNING_IDENTITY`（形如
   `Developer ID Application: Your Name (TEAMID)`，可用
   `security find-identity -v -p codesigning` 查看）。

## 阶段 4：在 Mac 上打正式包（最简路径）

```sh
git pull
# 1. 改 Cargo.toml 顶部 version（唯一版本源，如 0.2.0）
# 2. CHANGELOG.md 顶部加一节： ## [0.2.0] 以及更新说明
bun run release
```

脚本自动完成：编译 → 签名 → 公证 → `CheapRouter-<v>.dmg` +
`CheapRouter-<v>.zip` → 生成增量补丁 → 签名 `appcast.xml` → 全部上传 R2。
结束后：

- 下载地址：`https://releases.cheaprouter.cc/CheapRouter-<v>.dmg`
- 老版本客户端菜单"Check for Updates…"即可收到更新。

只想本地验证不发布：`bun run release --local`；跳过公证的试打包：`--adhoc`。

## 阶段 5：CI 出 Windows 包（推荐路径）

Windows 安装包由 GitHub Actions 打（本地打需要 Inno Setup，一般没必要）。

1. 仓库 Settings → Secrets and variables → Actions，配齐：

| Secret | 说明 |
|---|---|
| `SPARKLE_PRIVATE_KEY` | 阶段 1 导出的私钥（签第一跳 macOS `appcast.xml` 与 Windows `appcast-windows-*.xml`）|
| `MYGO_UPDATE_PRIVATE_KEY` | MyGo 第二跳更新 Ed25519 私钥（签 `mygo/update-<os>-<arch>.json` 中 `.tar.gz` 的 SHA-256 摘要，缺失即失败）|
| `MYGO_UPDATE_PUBLIC_KEY` | MyGo 第二跳更新 Ed25519 公钥（须与 `MYGO_UPDATE_PRIVATE_KEY` 及 `desktop/cmd/sailry-desktop/mygo.json` 一致，编译期注入二进制）|
| `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` | Developer ID 证书 .p12 的 base64 与密码 |
| `APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` / `APPLE_TEAM_ID` | 公证凭据 |
| `WAKU_SIGNING_IDENTITY` | 同 .env 里那串 |
| `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` | 阶段 2 的 token |
| `R2_ENDPOINT` / `R2_PROVIDER` | 自建 MinIO / 兼容 S3 时配置（如 `https://s3.cheaprouter.cc` / `Minio`）|
| `R2_BUCKET` | `cheaprouter-releases`（可省，已是默认）|
| `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` | 可选 Authenticode；没有则包不签名，用户首启有 SmartScreen 提示 |

2. 触发：推 tag `v<版本>`（须与 `Cargo.toml` 或 `desktop/cmd/sailry-desktop/mygo.json` 之一唯一匹配），或 Actions → Release → Run workflow（可通过 `runtime` 输入选择 `auto` / `rust` / `mygo`）。
3. 工作流按 `runtime` 互斥执行（同一 tag 只触发一条链，且已发布 tag 会自动跳过防止重复发布）：
   - `runtime=rust`（`main`）：保持既有 Rust 构建行为不变，产出 `CheapRouter-<v>-<arch>-Setup.exe`、便携 zip、macOS DMG/zip 与第一跳 appcast。
   - `runtime=mygo`（`desktop/`）：执行两跳发布链，见下方阶段 6。
4. 检查 draft 没问题后点 **Publish** → `sync-release.yml` 自动把所有文件同步进 R2（先传不可变归档，再更新短缓存清单指针，任一失败即 job 失败）—— 这一步完成，双端自动更新才对用户生效。

## 阶段 6：`desktop/`（Go/MyGo）两跳热更新发布流程

从 `main`（Rust/GPUI v0.3.x）升级到 `desktop/`（Go/MyGo v0.4.0+）由同一套 CI 流水线完成两跳产物构建与签名：

### 6.1 版本与身份单一真值

- 版本源：`desktop/cmd/sailry-desktop/mygo.json` 的 `.version`（须与 `desktop/internal/link/compatibility.go` 的 `ProductVersion` 及推送的 Git tag 一致，且禁止带 `-` prerelease 后缀）。
- 身份对齐：`name=CheapRouter`、`slug=cheaprouter`、`identifier=sh.waku`、macOS 数值化 `CFBundleVersion = major*1_000_000 + minor*1_000 + patch`（如 `0.4.0 → 4000 > 3008`）、`macos.infoPlist.SUPublicEDKey` 与 `resources/Info.plist` 一致、Windows Inno Setup `AppId={{7DC6C35B-FA40-4A95-B37A-626BF64556C5}}` 与 `resources/windows/waku.iss` 一致。

### 6.2 双平台构建与封装

- **macOS（`mygo-macos`：arm64 + x86_64）**：
  - 在 `macos-latest`（arm64）与 `macos-15-intel`（x86_64）分别编译 native 库（iroh / turso / capture）并调用 `desktop/cmd/sailry-desktop/macos/package.sh --require-update-key`；
  - 经 `mygo build`、补丁 `Info.plist`、`-seal`、代码签名与可选公证/staple 后，同一终态 `CheapRouter.app` 同时产出 DMG、第一跳 Sparkle zip 及第二跳 MyGo `.tar.gz`；
  - `arm64` 节点调用 `scripts/appcast.ts`（内置 Sparkle 2.9.4 `generate_appcast`）用 `SPARKLE_PRIVATE_KEY` 签发 `appcast.xml`。
- **Windows（`mygo-windows`：x86_64 + arm64）**：
  - 在 `windows-latest` 与 `windows-11-arm` 编译 native 库并提取对应架构 MSVC CRT（`desktop/native/windows/crt.ps1 -Arch <amd64|arm64>`）；
  - 调用 `desktop/cmd/sailry-desktop/windows/package.sh -arch <amd64|arm64> --require-update-key`：使用 `mygo build -skip-installer`（**完全不依赖 NSIS**）与 **Inno Setup**（`desktop/cmd/sailry-desktop/windows/cheaprouter.iss`）封装 `CheapRouter-<v>-<arch>-Setup.exe`、便携 zip 及第二跳 MyGo `.tar.gz`；
  - Inno Setup 安装器支持 `/SILENT /NORESTART /SP- /DIR=`，安装前自动结束旧 `client-daemon.exe`/`waku.exe`/`waku_js_repl.exe`、清理旧 Rust 文件并拉起 `CheapRouter.exe`。
- **汇签与 Draft（`mygo-draft-release`）**：
  - 第一跳：从 `https://s3.cheaprouter.cc/cheaprouter-releases/appcast-windows-<arch>.xml` 拉取历史条目，调用 `scripts/appcast-windows.ts` 用 `SPARKLE_PRIVATE_KEY` 签发 `appcast-windows-x86_64.xml` / `appcast-windows-aarch64.xml`；
  - 第二跳：调用 `desktop/cmd/mygo-manifest sign` 与 `verify`，用 `MYGO_UPDATE_PRIVATE_KEY` 对 4 个平台 `.tar.gz` 的 SHA-256 摘要做 Ed25519 签名，产出 `update-darwin-arm64.json`、`update-darwin-amd64.json`、`update-windows-amd64.json`、`update-windows-arm64.json`；
  - 生成 `SHA256SUMS.txt` 并写入 draft GitHub Release。

### 6.3 R2 对象存储路径与缓存策略（`sync-release.yml`）

| R2 路径 | 内容 | Cache-Control |
|---|---|---|
| `r2:<bucket>/`（不可变） | `CheapRouter-<v>*.dmg`、`CheapRouter-<v>*.zip`、`CheapRouter-<v>.md`、`CheapRouter-<v>-<arch>-Setup.exe`、`cheaprouter-<v>-<triple>.zip`、`SHA256SUMS.txt` | `public, max-age=31536000, immutable` |
| `r2:<bucket>/mygo/`（不可变） | `cheaprouter-<v>-darwin-{arm64,amd64}.tar.gz`、`cheaprouter-<v>-windows-{amd64,arm64}.tar.gz`（及后续 `.delta`） | `public, max-age=31536000, immutable` |
| `r2:<bucket>/`（可变指针） | `appcast.xml`、`appcast-windows-x86_64.xml`、`appcast-windows-aarch64.xml`、`latest-windows.txt` | `public, max-age=300, must-revalidate` |
| `r2:<bucket>/mygo/`（可变清单） | `update-darwin-arm64.json`、`update-darwin-amd64.json`、`update-windows-amd64.json`、`update-windows-arm64.json` | `public, max-age=300, must-revalidate` |

### 6.4 本地验证与回滚

- Linux/本地打包脚本预检（无需 macOS/Windows 工具）：
  ```sh
  cd desktop
  ./cmd/sailry-desktop/macos/package.sh -arch arm64 --require-update-key --dry-run
  ./cmd/sailry-desktop/macos/package.sh -arch amd64 --require-update-key --dry-run
  ./cmd/sailry-desktop/windows/package.sh -arch amd64 --require-update-key --dry-run
  ./cmd/sailry-desktop/windows/package.sh -arch arm64 --require-update-key --dry-run
  unset GOROOT && export GOTOOLCHAIN=go1.27.3 CGO_ENABLED=0
  go test ./cmd/sailry-desktop/ ./internal/update/ ./cmd/mygo-manifest/ -count=1
  ```
- **回滚方式**：
  - 撤回第一跳（停止 `main` 用户继续升到新版本）：重新上传不含问题版本的 `appcast.xml` / `appcast-windows-*.xml`（或通过 `sync-release.yml` 手动指定上一稳定 tag 回写指针）；
  - 撤回第二跳（停止 Go 用户继续升到新版本）：将 `r2:<bucket>/mygo/update-*.json` 恢复为上一稳定版本的清单（由于 `.tar.gz` 不可变保留在桶中，最长 5 分钟 TTL 后生效）；对已升级用户发紧急修复则直接递增版本号发布（例如 `0.4.1`）。

## 自动更新是怎么工作的（心智模型）

- **两把独立 Ed25519 密钥、一个 R2 桶** 服务两跳更新：
  - **第一跳（`SPARKLE_PRIVATE_KEY` → `SUPublicEDKey`）**：签原始安装包字节，写入桶根 `appcast.xml`（macOS Sparkle）和 `appcast-windows-<arch>.xml`（Windows Rust updater + Inno Setup `/SILENT /DIR=`）；
  - **第二跳（`MYGO_UPDATE_PRIVATE_KEY` → `MYGO_UPDATE_PUBLIC_KEY`）**：签 `.tar.gz` 的 SHA-256 摘要，写入 `mygo/update-<os>-<arch>.json`，由 Go 端 `desktop/internal/update` 与 `mygo.Updater` 校验并原子替换。
- 指针与清单 5 分钟缓存；其余版本化归档永久缓存、永不覆盖，且上传严格先传归档再更新指针。
- debug 构建永不自更新；测试更新流：装一个旧版正式包 → 菜单或设置页检查更新。

## 常见坑

- **AppId（`resources/windows/waku.iss` 与 `desktop/cmd/sailry-desktop/windows/cheaprouter.iss`）已铸死为 fork 自己的 GUID，永远不要再改**：
  改了 Windows 就认不出旧安装，更新会装出第二份。
- Rust 发布版本源在 `Cargo.toml`，MyGo 发布版本源在 `desktop/cmd/sailry-desktop/mygo.json`（与 `desktop/internal/link/compatibility.go` 一致）；带 `-beta` 等 prerelease 后缀的版本会被稳定频道发布链拒绝。
- `CHANGELOG.md` 没有对应版本小节时本地 `bun run release` 会失败——先写再发。
- Rust 版 Windows exe 内部文件名保持 `waku.exe`，Go/MyGo 版为 `CheapRouter.exe`；Inno Setup 在第一跳升级时会自动结束旧进程并清理 `waku.exe`/`client-daemon.exe`。
