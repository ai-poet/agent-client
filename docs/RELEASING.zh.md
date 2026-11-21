# CheapRouter 发布操作手册（Go/MyGo 两跳发布链）

本文记录 CheapRouter（Go/MyGo 客户端）的 CI 构建、签名、两跳热更新与对象存储同步流程。
更新源桶为 `cheaprouter-releases`，公开下载基址为 `https://s3.cheaprouter.cc/cheaprouter-releases`。

## 1. 两跳热更新模型

两把独立 Ed25519 密钥、同一个对象存储桶（`cheaprouter-releases`）共同服务两跳升级：

1. **第一跳（存量 v0.3.x 用户 → Go/MyGo v0.4.0+）**：
   - 密钥：`SPARKLE_PRIVATE_KEY`（公钥配置于 `cmd/sailry-desktop/mygo.json` 的 `macos.infoPlist.SUPublicEDKey`）；
   - macOS：`scripts/appcast.ts` 签发桶根 `appcast.xml`，指向 `CheapRouter-<v>.zip`；
   - Windows：`scripts/appcast-windows.ts` 签发桶根 `appcast-windows-x86_64.xml` 与 `appcast-windows-aarch64.xml`，指向 Inno Setup 安装器 `CheapRouter-<v>-<arch>-Setup.exe`（支持 `/SILENT /NORESTART /SP- /DIR=` 原地覆盖并清理旧可执行文件）。
2. **第二跳（Go/MyGo v0.4.0+ → 后续 Go/MyGo 版本）**：
   - 密钥：`MYGO_UPDATE_PRIVATE_KEY` / `MYGO_UPDATE_PUBLIC_KEY`（公钥配置于 `cmd/sailry-desktop/mygo.json` 的 `updates.publicKey`，并在打包期注入二进制）；
   - `cmd/mygo-manifest sign` 对四端 `.tar.gz` 的 SHA-256 摘要做 Ed25519 签名，产出 `mygo/update-{darwin-arm64,darwin-amd64,windows-amd64,windows-arm64}.json`，由客户端 `internal/update` 与 `mygo.Updater` 校验并原子替换。

## 2. 版本与身份单一真值

- **版本源**：`cmd/sailry-desktop/mygo.json` 的 `.version`，必须与 `internal/link/compatibility.go` 的 `ProductVersion` 及推送的 Git tag `v<version>` 一致，且禁止带 `-` prerelease 后缀。
- **身份对齐**：
  - `name=CheapRouter`、`slug=cheaprouter`、`identifier=sh.waku`；
  - macOS 数值化 `CFBundleVersion = major*1_000_000 + minor*1_000 + patch`（如 `0.4.0 → 4000 > 3008`）；
  - Windows Inno Setup `AppId={{7DC6C35B-FA40-4A95-B37A-626BF64556C5}}`（已铸死，**切勿修改**，否则 Windows 无法原地升级）。

## 3. GitHub Actions Secrets 配置

在仓库 Settings → Secrets and variables → Actions 中配置：

| Secret | 说明 |
|---|---|
| `MYGO_UPDATE_PRIVATE_KEY` | MyGo 第二跳更新 Ed25519 私钥（签 `mygo/update-<os>-<arch>.json`） |
| `MYGO_UPDATE_PUBLIC_KEY` | MyGo 第二跳更新 Ed25519 公钥（须与私钥及 `mygo.json` 一致，编译期注入二进制） |
| `SPARKLE_PRIVATE_KEY` | 第一跳 Ed25519 私钥（签 macOS `appcast.xml` 与 Windows `appcast-windows-*.xml`） |
| `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` | 可选 Developer ID Application 证书 `.p12` 的 base64 与密码（未配置时自动以 `ad-hoc` 签名构建） |
| `APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` / `APPLE_TEAM_ID` | 可选 Apple 公证凭据 |
| `WAKU_SIGNING_IDENTITY` | macOS 签名身份（如 `Developer ID Application: ...`） |
| `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` | 可选 Windows Authenticode `.pfx` 证书与密码 |
| `R2_ENDPOINT` / `R2_PROVIDER` | S3 兼容存储配置（如 `https://s3.cheaprouter.cc` / `Minio`） |
| `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` | 对象存储读写凭据 |
| `R2_BUCKET` | 桶名，默认 `cheaprouter-releases` |

## 4. 触发与发布步骤

1. 更新 `cmd/sailry-desktop/mygo.json`（`version` 与 `macos.infoPlist.CFBundleVersion`）、`internal/link/compatibility.go`（`ProductVersion`）及 `CHANGELOG.md`。
2. 推送版本 tag `v<version>`（或在 Actions → Release 手动触发 `workflow_dispatch`）。
3. CI 流水线（`.github/workflows/release.yml`）自动完成：
   - **macOS（`mygo-macos`：arm64 + amd64）**：编译 native 库（iroh / turso / capture），调用 `cmd/sailry-desktop/macos/package.sh --require-update-key` 产出 `.dmg`、第一跳 `.zip` 与第二跳 `.tar.gz`；`arm64` 节点生成并签名 `appcast.xml`。
   - **Windows（`mygo-windows`：x86_64 + aarch64）**：编译 native 库并提取对应架构 MSVC CRT（`native/windows/crt.ps1`），调用 `cmd/sailry-desktop/windows/package.sh -arch <amd64|arm64> --require-update-key` 使用 `mygo build -skip-installer` 与 Inno Setup（`cheaprouter.iss`）产出 `-Setup.exe`、便携 `.zip` 与第二跳 `.tar.gz`。
   - **汇签与 Draft（`mygo-draft-release`）**：签发第一跳 `appcast-windows-*.xml` 与第二跳 `update-*.json`，生成 `SHA256SUMS.txt` 并创建 draft GitHub Release。
4. 检查 draft 产物无误后点击 **Publish**，`.github/workflows/sync-release.yml` 自动同步到对象存储。

> 免证书 macOS 用户可通过一键脚本安装（无 `com.apple.quarantine` 属性）：
> ```sh
> curl -fsSL https://s3.cheaprouter.cc/cheaprouter-releases/install-mac.sh | sh
> ```

## 5. 对象存储路径与缓存策略（`sync-release.yml`）

上传严格遵循「先传不可变归档，再更新可变指针/清单」的顺序：

| 桶路径 | 内容 | Cache-Control |
|---|---|---|
| `r2:<bucket>/`（不可变） | `CheapRouter-<v>*.dmg`、`CheapRouter-<v>*.zip`、`CheapRouter-<v>.md`、`CheapRouter-<v>-<arch>-Setup.exe`、`cheaprouter-<v>-<triple>.zip`、`SHA256SUMS.txt` | `public, max-age=31536000, immutable` |
| `r2:<bucket>/mygo/`（不可变） | `cheaprouter-<v>-darwin-{arm64,amd64}.tar.gz`、`cheaprouter-<v>-windows-{amd64,arm64}.tar.gz`（及后续 `.delta`） | `public, max-age=31536000, immutable` |
| `r2:<bucket>/`（可变指针） | `appcast.xml`、`appcast-windows-x86_64.xml`、`appcast-windows-aarch64.xml`、`latest-windows.txt` | `public, max-age=300, must-revalidate` |
| `r2:<bucket>/mygo/`（可变清单） | `update-darwin-arm64.json`、`update-darwin-amd64.json`、`update-windows-amd64.json`、`update-windows-arm64.json` | `public, max-age=300, must-revalidate` |

## 6. 本地预检与回滚

### 本地打包预检（支持 Linux）

```sh
./cmd/sailry-desktop/macos/package.sh -arch arm64 --require-update-key --dry-run
./cmd/sailry-desktop/macos/package.sh -arch amd64 --require-update-key --dry-run
./cmd/sailry-desktop/windows/package.sh -arch amd64 --require-update-key --dry-run
./cmd/sailry-desktop/windows/package.sh -arch arm64 --require-update-key --dry-run
```

### 回滚方式

- **撤回第一跳**：通过 `sync-release.yml` 手动指定上一稳定 tag 回写桶根 `appcast.xml` / `appcast-windows-*.xml`；
- **撤回第二跳**：将 `r2:<bucket>/mygo/update-*.json` 恢复为上一稳定版本清单（最长 5 分钟 TTL 后生效）；对已升级用户发紧急修复则递增版本号重新发布（如 `0.4.1`）。
