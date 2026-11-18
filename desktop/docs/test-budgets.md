# 测试预算（`internal/node`）

## 规则

- `testBudget()`（`internal/node/testbudget_test.go`）是 fixture 给一次操作的墙钟预算：普通构建 **30s**，`-race` 构建 **×4 但不超过 3 分钟**（`maxTestBudget`）。
- 上限的意义：预算放大本来是为了容纳 race 插桩的 4 倍开销，不是给真正 hang 住的用例发四倍宽限。超过 3 分钟仍未结束，按 hang 处理。
- 新写用例请用 `testBudget()` 或它的整数倍，不要再写裸 `30 * time.Second`。

## 非竞态语义不变

换算保持相对量级，**普通构建的数值与原值完全一致**：

| 原值 | 现写法 | 非竞态实际值 | race 实际值 |
| --- | --- | --- | --- |
| 30s | `testBudget()` | 30s | 120s |
| 60s | `2*testBudget()` | 60s | 120s（封顶） |
| 120s | `4*testBudget()` | 120s | 120s（封顶） |
| 15s | `testBudget()/2` | 15s | 60s |
| 10s | `testBudget()/3` | 10s | 40s |
| 5s | `testBudget()/6` | 5s | 20s |

因此**非竞态全量仍然是原来的强度**，不存在「把性能回归检测放水成永远能过」。

## 不能改的预算（刻意验证超时本身）

下列位置的短预算或超时断言是**被测行为**的一部分，不是等待上限，改动会让测试失去意义：

| 位置 | 为什么必须保留原值 |
| --- | --- |
| `internal/node/multi_host_integration_test.go:425` | 断言「访问被撤销后远端调用以超时结束且不是取消」，超时是断言对象（该文件带 `//go:build integration`，见下节） |
| `internal/node/computer/computer_test.go:316` | 桩实现返回 `context.DeadlineExceeded`，用于验证上层对超时的映射 |
| `internal/node/agent/model/policy_test.go:214,377` | 分类表把 `DeadlineExceeded` 映射为 `Network`，超时是输入而非等待 |

## integration 用例不在默认全量内

`internal/node/multi_host_integration_test.go` 首行是 `//go:build integration`，**默认的 `go test ./internal/node/` 根本不会编译它**，所以「默认全量绿」不等于覆盖了这个文件。跑它需要：

```bash
SAILRY_IROH_LIB=$PWD/.runtime/native/linux-amd64/libsailry_iroh.so \
LD_LIBRARY_PATH=$PWD/.runtime/native/linux-amd64 \
go test -tags integration ./internal/node/ -run TestMultiHostControllerRoutesIntegration -count=1 -timeout 12m
```

它覆盖真实鉴权 loopback 链路：controller 路由、跨节点知识引用、插件作用域、撤销后订阅关闭。要把它纳入「全量」结论，CI 必须单开一个 `-tags integration` job，并备好 iroh / turso 动态库与足够内存。

### 当前已知失败（2026-11-18，本机）

`TestMultiHostControllerRoutesIntegration` 在第 215 行 `keyword index through executor controller storage route` 稳定失败，**不是慢**：把外层预算从 `2*testBudget()`（60s）提到 `6*testBudget()`（180s）后仍在同一行失败，整条用例跑了 183s。

`query_knowledge`（keyword 模式，经 executor 的 controller storage 路由）返回的 `KnowledgePage` 三个来源全部带错误、`Hits` 为空，重复约 800 次完全一样：

```
outcome_unknown: completion unavailable; retry with the same request identifier   (×2)
unavailable: knowledge source Node is unavailable                                (×1)
```

对应 `internal/link/remote.go:249` 与 `internal/node/resource_service.go:263`：远端 knowledge 读取没有拿到 completion，另有一个来源的 Node 不在候选地址里。这是路由/传输层的确定性缺口，不是预算问题，需要单独排查后再决定是否进 CI。在这条修好之前，「全量绿」只代表默认构建，不代表 integration 覆盖。

临时加日志复跑后（`aggregateKnowledge` 收结果处 + `dispatchCall` 的 completion 读取处），每次失败的实际链路是：

```
AGG result node=<A> err=unavailable: knowledge source Node is unavailable
AGG result node=<B> err=outcome_unknown: completion unavailable; retry with the same request identifier
AGG result node=<C> err=outcome_unknown: ...
REMOTE completion err=unavailable: Link connection is unavailable kind= bound=true
```

即：候选集合本身是齐的（self + 两台对端都进了聚合），但**每一路远端读取都在 completion 阶段拿到 `Link connection is unavailable`**，另有一路是缓存来源但不在已知地址里。所以问题不在「executor 少收集了节点」，而在资源流打开、收到 receipt 之后，读 completion 时链路被判为不可用。复现时可用上面的两处日志定位。

## race 全量

`-race` 全量在本机（15GB）无法在 30–50 分钟内跑完，属环境限制，不是测试失败：

- 判据：**全量 `-race -timeout 40m` 跑完且绿**；
- 若被环境 kill，单独报 `DATA RACE` 计数（实际观测为 0），不要与非竞态失败混为一谈；
- CI 侧：给 ≥16GB 内存 + `-timeout 60m`，或按文件组拆成 3–4 个 job 并行。

## 实测记录（2026-11-18，15GB 机器）

| 项 | 结果 |
| --- | --- |
| 非竞态全量 `./internal/node/` | ok 354s |
| `TestPluginTurnFacts`（`-race` 单跑） | ok 47.2s（改造前 30.6s 超时） |
| `TestGoalLoop/the_turn_limit`（26 轮，skip 已删，`-race` 单跑） | **ok 10.6s** |
| `DATA RACE` | 未出现 |

注意：26 轮那条只跑一次转是 10 秒级，之前把它当成"整组跑不完"是误读整组耗时（`TestGoalLoop` 包含其它重子用例）。本仓库不再为它保留 skip。

## CI 建议

- 单 job：`CGO_ENABLED=1 go test -race -timeout 60m ./internal/node/`，内存 ≥16GB；
- 或拆 3–4 个并行 job：`plugin_*` / `knowledge_*` / `migration_*`+`backup_*` / 其余；
- 全量 race 在 15GB 机器上跑不完属**环境限制**，不是测试失败，也不代表存在竞态（实测 `DATA RACE` 计数为 0）。
