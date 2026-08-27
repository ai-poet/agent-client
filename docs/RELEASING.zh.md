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

## 阶段 3（一次性）：Apple 签名与公证（mac 包硬需求）

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
| `SPARKLE_PRIVATE_KEY` | 阶段 1 导出的私钥（签 Windows appcast 用同一把）|
| `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` | Developer ID 证书 .p12 的 base64 与密码 |
| `APPLE_ID` / `APPLE_APP_SPECIFIC_PASSWORD` / `APPLE_TEAM_ID` | 公证凭据 |
| `WAKU_SIGNING_IDENTITY` | 同 .env 里那串 |
| `R2_ACCOUNT_ID` / `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` | 阶段 2 的 token |
| `R2_BUCKET` | `cheaprouter-releases`（可省，已是默认）|
| `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` | 可选 Authenticode；没有则包不签名，用户首启有 SmartScreen 提示 |

2. 触发：推 tag `v<版本>`（须与 Cargo.toml 一致），或 Actions → Release →
   Run workflow（不用 tag，按 Cargo.toml 版本出 draft）。
3. 工作流产出 Windows x64/arm64 的 `CheapRouter-<v>-<arch>-Setup.exe`、便携
   zip、签名过的 `appcast-windows-<arch>.xml`，连同 mac/Linux 产物挂在一个
   **draft GitHub release** 上。
4. 检查 draft 没问题后点 **Publish** → `sync-release.yml` 自动把所有文件同步进
   R2 —— 这一步完成，两个平台的自动更新才对老用户生效。

## 自动更新是怎么工作的（心智模型）

- **一把 EdDSA 密钥、一个 R2 桶** 服务两个平台。`appcast.xml`（mac）和
  `appcast-windows-<arch>.xml`（win）是"最新版本指针"，5 分钟缓存；其余文件
  按版本命名、永久缓存、永不覆盖。
- mac 由内嵌 Sparkle 完成校验+安装（zip/增量补丁）；Windows 由 `src/updater.rs`
  自实现同一契约：验签 → 下载 Setup.exe → `/SILENT` 静默重装 → 自动重启。
  公钥编译期取自 Info.plist，两平台不可能用错钥匙。
- 撤回版本 = 重新生成不含该版本的 appcast 上传；老文件留桶里不影响。
- debug 构建永不自更新；测试更新流：装一个旧版正式包 → 菜单检查更新。

## 常见坑

- **AppId（`resources/windows/waku.iss`）已铸死为 fork 自己的 GUID，永远不要再改**：
  改了 Windows 就认不出旧安装，更新会装出第二份。
- 版本号只改 `Cargo.toml`；带 `-beta` 后缀的版本会被发布脚本拒绝。
- `CHANGELOG.md` 没有对应版本小节时发布会失败——先写再发。
- Windows 的 exe 内部文件名仍是 `waku.exe`（快捷方式显示 CheapRouter），
  这是有意为之，别改。
