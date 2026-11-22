# Sailry 0.2.7 核心同步

基线 `main@a7200abf1`，上游范围 `a285ba78a..b179f97fc`。只同步可独立验证的核心修复，不升级发布版本、协议或包指针。

## 移植前分类

| # | 上游提交 | 分类 | 决定 | 理由 |
|---:|---|---|---|---|
| 1 | [`a35b04b3c`](https://github.com/sailry/sailry-harness/commit/a35b04b3c291ea1fe6e2fb9040670940acdda5c3) | 不相关 | 跳过 | 中继部署指南不是 fork 的核心运行时。 |
| 2 | [`73418283e`](https://github.com/sailry/sailry-harness/commit/73418283ee103430e7fd7f70155d0e7b0de30f56) | 不相关 | 跳过 | 下载与校验示例指向上游发布包。 |
| 3 | [`ceb39aa04`](https://github.com/sailry/sailry-harness/commit/ceb39aa048ec8ff07782b6ee16e788ceab0f8c5d) | 不相关 | 跳过 | fork 没有对应的 pane_keys 分栏放大钩子。 |
| 4 | [`c3bc1929d`](https://github.com/sailry/sailry-harness/commit/c3bc1929d6b12c95f76e0cc0cd6f70937f06b6c6) | 不相关 | 跳过 | 现有索引服务没有该按需派生缓存管线。 |
| 5 | [`754f34af9`](https://github.com/sailry/sailry-harness/commit/754f34af9189477b59244923705044caf9212219) | 可选 | 选择 | 共用有界回放可避免为每个观察者复制待交付帧。 |
| 6 | [`a9368ce3e`](https://github.com/sailry/sailry-harness/commit/a9368ce3e84c5081b561959c8ad932d9cb8c4ff1) | 不相关 | 迁移 | 渠道费率和不完整 token 响应计数改变用量协议与计费口径，不改 CheapRouter 定价。 |
| 7 | [`fa707229d`](https://github.com/sailry/sailry-harness/commit/fa707229dc616457a635a4b47b5d02fbf8b069ff) | 可选 | 跳过 | 已有投影复用；新增活动和发布模型不作为独立缓存补丁引入。 |
| 8 | [`870385780`](https://github.com/sailry/sailry-harness/commit/87038578088fbee216e0e355547751b47cb734d5) | 不相关 | 跳过 | 保存费率依赖未引入的渠道计费字段。 |
| 9 | [`ddf869419`](https://github.com/sailry/sailry-harness/commit/ddf869419ffe8e259828cabdf561e942cfeccc5e) | 不相关 | 跳过 | 上游重组委派标签、消息托盘和焦点，保留 fork 已验收的呈现。 |
| 10 | [`35cecf18f`](https://github.com/sailry/sailry-harness/commit/35cecf18f0819391325003e6f53a0891799036db) | 不相关 | 跳过 | 常规工作区标签改变本地导航和分组。 |
| 11 | [`8ea8d764c`](https://github.com/sailry/sailry-harness/commit/8ea8d764c0b56223144df1fff0d18fe244f1ff00) | 不相关 | 跳过 | 仅上调上游统计插件指针，fork 内置包不是该 Gitlink。 |
| 12 | [`abc5adf58`](https://github.com/sailry/sailry-harness/commit/abc5adf580f160f9594c5a516d825a77852cccd9) | 可选 | 跳过 | 键帽与快捷键录制是设置呈现改版。 |
| 13 | [`60f3a1cc1`](https://github.com/sailry/sailry-harness/commit/60f3a1cc1cff0a27b872d9744b6d09f4286ec1aa) | 不相关 | 跳过 | 几何锚点测试针对未采用的发送交接布局。 |
| 14 | [`94b28ef51`](https://github.com/sailry/sailry-harness/commit/94b28ef51db5986b63f56758cb7d859df358c8c2) | 必须 | 选择 | 预取时过滤会永久丢失在交付前重新取得权限的事件，应按交付时权限读取。 |
| 15 | [`f6f633449`](https://github.com/sailry/sailry-harness/commit/f6f6334496c81e236f6cd0bc8c563f906fe5050b) | 可选 | 跳过 | 单独提交前导计划改变工具批处理、审批顺序与提示词。 |
| 16 | [`4170270d8`](https://github.com/sailry/sailry-harness/commit/4170270d8ee0f805413e203ab8e5685e86c4c4ea) | 可选 | 跳过 | 上下文压力与证据回读策略成套改变模型看到的历史。 |
| 17 | [`ff363f3cb`](https://github.com/sailry/sailry-harness/commit/ff363f3cb466b72520427e4bb4a199cc20960ed2) | 可选 | 选择 | 仅同步实际采用的事件交付等边界说明，不宣称已有上游计费与上下文策略。 |
| 18 | [`03b56c7c3`](https://github.com/sailry/sailry-harness/commit/03b56c7c3918d084e4d567592b381fe7a88ef373) | 不相关 | 跳过 | 本地投影缺少识别单个未持久化工具草稿被隐藏的前置，不能一概复用清单。 |
| 19 | [`09f4afab3`](https://github.com/sailry/sailry-harness/commit/09f4afab3e39cda9183aa146e4389a06ce23667b) | 可选 | 选择 | 跳过无资产的工具载荷，降低已有会话资产查询的解析成本。 |
| 20 | [`82bb6d8ca`](https://github.com/sailry/sailry-harness/commit/82bb6d8cad350b6d394eedb87fa7c52da9aab9e3) | 可选 | 选择 | 在上一项基础上识别纯文本展示，同时保留文件、制品和转义格式。 |
| 21 | [`ed4958e3d`](https://github.com/sailry/sailry-harness/commit/ed4958e3db772bef6b1d9ada9c68b73e190dd13f) | 不相关 | 跳过 | 仅更新统计插件无障碍展示的上游指针。 |
| 22 | [`6a997c7f9`](https://github.com/sailry/sailry-harness/commit/6a997c7f9bf631948e751e83f988a6027c1cd9f0) | 不相关 | 跳过 | 命令回执压缩依赖上游模型历史证据回读管线。 |
| 23 | [`df2439ce6`](https://github.com/sailry/sailry-harness/commit/df2439ce6de222478cc0231eea10a802cf829cbc) | 可选 | 跳过 | 端口转发行改为直接打开，属于桌面布局和交互变化。 |
| 24 | [`b0ea0d0fe`](https://github.com/sailry/sailry-harness/commit/b0ea0d0fe263d53e1d7acf6e949cf2b3a2f37ba0) | 不相关 | 跳过 | 针对未移植的键帽录制器和费率设置补焦点、译文。 |
| 25 | [`1dce101b8`](https://github.com/sailry/sailry-harness/commit/1dce101b8497e6d48fe7f458277acbfcca89df3b) | 不相关 | 跳过 | 依赖第 9、13 项的托盘投影和几何锚点，不重做 fork 的桌面发送区。 |
| 26 | [`61a6a804f`](https://github.com/sailry/sailry-harness/commit/61a6a804f8bc1db1ec69cc6678b9ee1b846e7e72) | 可选 | 跳过 | 窗口偏好合并与 Windows 模态窗口诊断不属于核心同步。 |
| 27 | [`2bf61ec85`](https://github.com/sailry/sailry-harness/commit/2bf61ec85dc825a86ecd71644fdb9d372e24778f) | 必须 | 选择 | 连续拨号失败不应耗尽所有通用重试，TLS 证据也不应在收到 HTTP 响应时丢失。 |
| 28 | [`1ade277d9`](https://github.com/sailry/sailry-harness/commit/1ade277d9250d7d950b480b254714b8c599a2bc6) | 不相关 | 迁移 | 新增 Git/SSH fault 原因并改变远程可见错误过滤，停止整条。 |
| 29 | [`1928527f4`](https://github.com/sailry/sailry-harness/commit/1928527f401c2a7d7e8f61ecaa4cc54af3e692aa) | 可选 | 跳过 | 本地命令报告还涉及服务发现故障分类，不改变 fork 提供商的公开诊断。 |
| 30 | [`9319b5f7a`](https://github.com/sailry/sailry-harness/commit/9319b5f7a6c4e403d429aebdc45b9d57c5001647) | 可选 | 跳过 | 原生浏览器地址栏草稿同步是面板交互修复，非本轮核心边界。 |
| 31 | [`082e6ca2e`](https://github.com/sailry/sailry-harness/commit/082e6ca2e8ab2bedb7f49769293191b0a4d84092) | 可选 | 跳过 | 默认压缩策略需要回填旧库并重新解释捕获预算，不引入新的持久化策略。 |
| 32 | [`d8845e37a`](https://github.com/sailry/sailry-harness/commit/d8845e37ab385540ced1b2b9aa86ee7286f686e4) | 不相关 | 跳过 | 依赖上一项保存的默认压缩策略及输出预留。 |
| 33 | [`87c605c6e`](https://github.com/sailry/sailry-harness/commit/87c605c6e831f8811a24568a6536b44dcf36233e) | 不相关 | 跳过 | 只验证上述未引入的恢复预算。 |
| 34 | [`a1f465068`](https://github.com/sailry/sailry-harness/commit/a1f465068f240039caf066f9770b2703e6988662) | 不相关 | 跳过 | fork 未提供 SSH 安装及其诊断。 |
| 35 | [`f3dedffe7`](https://github.com/sailry/sailry-harness/commit/f3dedffe774956e0241cb15ddd8f2c41e60b5dee) | 必须 | 选择 | 补足插件证书失败分类和代理拨号上限，与直连重试使用同一策略。 |
| 36 | [`08a35b8ac`](https://github.com/sailry/sailry-harness/commit/08a35b8acf8c5bd74cbcd0f93320b8577cc38a89) | 不相关 | 跳过 | 各只读入口依赖第 28 项的 openReadWorktree，不摘取不完整的 Git 工作树配置支持。 |
| 37 | [`74d5619b5`](https://github.com/sailry/sailry-harness/commit/74d5619b5e8e21f4b02f49325e31a7ddf6d01a43) | 不相关 | 跳过 | Flutter/FFI 已移除，不为移动端的队列呈现重写共享投影。 |
| 38 | [`2409be604`](https://github.com/sailry/sailry-harness/commit/2409be6044bc73c55033a7bc46b164831b07648f) | 不相关 | 迁移 | 资产查询新增 Checkpoint/Delta 及分页合并语义，不能只更新一端。 |
| 39 | [`b7d0c852c`](https://github.com/sailry/sailry-harness/commit/b7d0c852c16a1756d802c478a10ef433b6a7e600) | 不相关 | 跳过 | 命令历史引用依赖未采用的第 16、22 项证据裁剪。 |
| 40 | [`283b98bc8`](https://github.com/sailry/sailry-harness/commit/283b98bc8fc5292f40a63a7bcb6657356a999376) | 可选 | 跳过 | 合并统计刷新要求引入新的发布与关闭生命周期，暂保留现有刷新机制。 |
| 41 | [`a67012146`](https://github.com/sailry/sailry-harness/commit/a67012146a37dccc1f7aedd739549c6b62f588bb) | 不相关 | 跳过 | 扩大普通会话的工具授权范围，还涉及已删除的 SSH 与数据库连接。 |
| 42 | [`253e991fa`](https://github.com/sailry/sailry-harness/commit/253e991fa787ad6406167e6cf9fbbc9c4742b821) | 不相关 | 跳过 | 来源主机绑定基于第 10 项窗格、标签与置顶布局，不引入整套上游导航。 |
| 43 | [`d25807201`](https://github.com/sailry/sailry-harness/commit/d258072011f35a0927f245db4a47b842f14197b9) | 必须 | 选择 | 停止及恢复应同时推进队列修订并让客户端重新取得状态，避免残留待发送项。 |
| 44 | [`363e72f39`](https://github.com/sailry/sailry-harness/commit/363e72f3957f04586590784205e71d3d0be7aa14) | 必须 | 选择 | 引用行的身份需包含来源与种类，不能仅按 ID 合并。 |
| 45 | [`0dccdb87b`](https://github.com/sailry/sailry-harness/commit/0dccdb87be623174022d88fe6431fd88f29d98df) | 不相关 | 跳过 | 摘要及用量快速读取与未移植的上下文压力和 processed history 管线共同工作。 |
| 46 | [`2bcba298e`](https://github.com/sailry/sailry-harness/commit/2bcba298e6b26501a1ad5bdda76646eb5bc59d6d) | 不相关 | 跳过 | 优化的是第 4 项按需代码图，fork 无该入口。 |
| 47 | [`3bfe5e86d`](https://github.com/sailry/sailry-harness/commit/3bfe5e86dc3d46859f381f90816ef54fc1de2723) | 不相关 | 跳过 | 长会话输出回放依赖第 16、39、45 项的证据引用与恢复。 |
| 48 | [`02dd31cc2`](https://github.com/sailry/sailry-harness/commit/02dd31cc2a9c40346709504e32079327591d5931) | 必须 | 选择 | 保留已有 Responses 缓存断点并稳定校验错误顺序，不改变分组路由。 |
| 49 | [`460ccbbe1`](https://github.com/sailry/sailry-harness/commit/460ccbbe1a0557faf216657e3437d7944c7d1207) | 不相关 | 迁移 | 用量协议新增缓存写入统计，并联动插件、SDK 和模型声明。 |
| 50 | [`9400c0493`](https://github.com/sailry/sailry-harness/commit/9400c049390e741fa0796a2a1fd0feb342f1b6f5) | 不相关 | 迁移 | 在会话观测协议添加有序请求输入变化，需要跨端协调。 |
| 51 | [`334578783`](https://github.com/sailry/sailry-harness/commit/3345787839b355aff2355c6ae20ec5d544d7aa51) | 可选 | 跳过 | 自动生成显式缓存断点依赖上游 developer 前缀布局，fork 继续使用 instructions。 |
| 52 | [`94e22145c`](https://github.com/sailry/sailry-harness/commit/94e22145cfb8e50c0fb4aa37d57c770da4b9c5ab) | 不相关 | 跳过 | 一次性真实模型对比程序于第 59 项移除。 |
| 53 | [`16a02f5a4`](https://github.com/sailry/sailry-harness/commit/16a02f5a420b2ecbb42b40145a660f5677ae64ca) | 不相关 | 跳过 | 仅描述未引入的缓存对比程序。 |
| 54 | [`51878d2df`](https://github.com/sailry/sailry-harness/commit/51878d2df8e9144faa58f25b00db84c95beb7f05) | 必须 | 选择 | 限制 HTTP 524 恢复等待，已向消费者交付完成信号的请求不得重放。 |
| 55 | [`34cb3fa7a`](https://github.com/sailry/sailry-harness/commit/34cb3fa7ab6b9ac9c23a7aaf3b990536c295313e) | 必须 | 选择 | 收集子任务后不再让父执行持有已收集实例，保留本地取消链。 |
| 56 | [`411d70b58`](https://github.com/sailry/sailry-harness/commit/411d70b586681a4c7ca38f8e63b8038222d7ff80) | 不相关 | 跳过 | 发送位置修复延续第 9、25 项的托盘与交接状态机。 |
| 57 | [`90b5fd8b2`](https://github.com/sailry/sailry-harness/commit/90b5fd8b2124b212a8e305abf4cf9418bc7bb3d5) | 必须 | 选择 | 截断的 Git 工作树列表不足以证明可安全删除；目录取消复用已有 fault，不改核心类型。 |
| 58 | [`ea2a43e0e`](https://github.com/sailry/sailry-harness/commit/ea2a43e0e8827d533784bcf7c631c920af72fd96) | 可选 | 跳过 | 活动浮层间距与磨砂效果不属于核心同步。 |
| 59 | [`2e8f64d17`](https://github.com/sailry/sailry-harness/commit/2e8f64d17bce98d615b758d169ae5b561700f887) | 不相关 | 跳过 | 对比程序未引入，没有对应的删除操作。 |
| 60 | [`45d65c108`](https://github.com/sailry/sailry-harness/commit/45d65c108594370e37f7a7acefad27a45bbb64c3) | 可选 | 跳过 | 立即打断前台工具会改变现有问答、审批与命令中断语义，保留 fork 的 Ask User 行为。 |
| 61 | [`68ad4cac2`](https://github.com/sailry/sailry-harness/commit/68ad4cac2cfdf0e7745da1c408d1a1f8fc315188) | 不相关 | 迁移 | 新增完整请求耗时的会话与用量字段，跨端吞吐率口径需要一起迁移。 |
| 62 | [`0c7e4f089`](https://github.com/sailry/sailry-harness/commit/0c7e4f0893bd769448c8ca6b24bce2bb24e24d21) | 不相关 | 迁移 | 新增 provider directory 请求和分页快照，不替换 CheapRouter 的按分组目录。 |
| 63 | [`62ad7680b`](https://github.com/sailry/sailry-harness/commit/62ad7680b4d8bf3a07a3f9c9cd486f9d4e1f13a8) | 可选 | 跳过 | 可见分栏刷新和菜单依赖不同的桌面布局，保留现有工作台。 |
| 64 | [`b179f97fc`](https://github.com/sailry/sailry-harness/commit/b179f97fc8ae24535c50a07dcbf12736b952fe1d) | 不相关 | 跳过 | fork 未采用完整协议、移动端或上游发布物，不声明 Sailry 0.2.7 兼容版本。 |
