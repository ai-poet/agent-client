# Agent 迭代记录

这里存放在迭代过程中写给后续 agent（和评审者）看的记录：根因分析、对照审查、修复与验收证据、阶段报告。它们随功能一起演进，不是面向用户的文档。

- 面向用户/贡献者的文档仍在 `docs/`（`RELEASING.zh.md` 等）与仓库根的 `README.md`、`ARCHITECTURE.md`、`CHANGELOG.md`。
- 新增此类记录请直接放进本目录，不要放在仓库根。

## 索引

| 文件 | 内容 |
| --- | --- |
| `compact-failure.md` | `/compact` 失败提示只显示兜底文案的排查与文案透出修复 |
| `compact-provider.md` | 压缩与会话模型选择（provider 解析、保存顺序） |
| `compact-provider-root.md` | 旧会话引用已停用统一 provider 的继任规则与验证 |
| `gateway-origin.md` | 网关端点切换：Rust→Go 映射、认证留在登录端点、测速与回退 |
| `harness-ask-user.md` | Ask User 提问自动展开选项的交互与焦点策略 |
| `harness-fixes-a.md` | macOS 图标、按分组原生协议、自动更新弹窗三项修复 |
| `harness-recharge.md` | 充值/套餐购买按 Rust 移植的对照与验证 |
| `macos-onboarding-branding.md` | macOS 引导重复弹出根因与 Agent 品牌自称修复 |
| `test-pipeline-fix.md` / `test-pipeline-round2.md` | CI 测试流水线两轮修复记录 |
| `upstream-sync-027.md` | 上游 sailry 0.2.7 的 64 条提交分类与 13 条移植映射 |
