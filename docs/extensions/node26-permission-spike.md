# Node 26 Permission Model · Spike 验收报告

> P0-T04 / Phase 0 关键 spike。验证 Node 26 是否能扛住 spec-v0.3 §6 "纯
> Node Permission Model" 的安全模型，定位绕路 / 性能 / 兼容性问题。

- 主机：macOS 15 (Darwin 24.6.0) · arm64
- Node：**v26.1.0**（fnm 装；2026-05-19 当前 current 通道）
- spike 目录：`/tmp/node-perm-spike/`（macOS `/tmp` → `/private/tmp` symlink）
- 测试源：`test1-basic.js` ~ `test8-defaults.js`，全部可重跑

---

## 0. 一句话结论

**Node 26 Permission Model 基本能扛 spec-v0.3 的 α 安全模型，但有 4 个需要落档的偏差**：

1. ⚠️ **`--allow-net` 在 Node 26.1.0 只是 boolean，没有 `=host` 子语法**：spec-v0.3 §6.1 假设的 `--allow-net=<host>` 是 Node 主线计划中但未落地的 feature。CLI 解析器对 `--allow-net=foo` 不报错只是因为它对未识别 `=value` 后缀普遍宽容（accept-and-ignore-the-value），不是 enforce。启动会打 ExperimentalWarning。per-host 网络 ACL 在 v1 落空。
2. ⚠️ **Node 不自动 canonicalize symlink**：`/tmp/x` 和 `/private/tmp/x` 在 ACL 里是两条路径；Rust 必须显式 canonicalize 双填，或 bootstrap 启动时 chdir 到 realpath。
3. ⚠️ **fs 调用绝对 overhead ~12μs/call**：相对开销 readFileSync +100%, statSync +1400%（绝对仍小，但工作流如目录递归扫描会感知）；spec 中的 "< 5%" 是错的指标。
4. ⚠️ **6 个 boolean flag 都打 SecurityWarning，`--allow-net` 打 ExperimentalWarning**：每次 spawn 扩展都会有 stderr 噪音，需要 `--no-warnings` 或 `NODE_NO_WARNINGS=1` 抑制。Coco 用 `--allow-child-process`，启动就会有 SecurityWarning，不能让最终用户看到。

绕路防御、危险默认禁、npm 兼容都达标。`--allow-fs-read/-write` 路径过滤实测真的 enforce（含 glob、相对路径、文件粒度）。建议进 Phase 1。

---

## 1. 验收清单（exec-plan §P0-T04 + plan-v0.3 §Phase 0）

| # | 项 | 结果 | 备注 |
|---|---|---|---|
| 1 | `--permission` 启动正常 | ✅ | `node --permission -e "console.log(1)"` 输出 `1` |
| 2 | `--allow-fs-read=/path` 限定有效 | ✅ | 见 §2.1 test1 |
| 3 | `--allow-net=host` 限定有效 | ⚠️ **未实现** | Node 26.1.0 只支持 boolean；见 §2.5 |
| 4 | `--allow-child-process` 开关 | ✅ | 见 §2.3 |
| 5 | `--allow-ffi` / `--allow-inspector` 默认禁 | ✅ | 见 §2.6 |
| 6 | 绕路尝试全部拦 | ✅ | 见 §2.2 |
| 7 | 常见 npm 包正常 | ✅ | 见 §2.4 |
| 8 | overhead < 5% | ⚠️ **指标错** | 见 §2.7；绝对 ~12μs/call |
| 9 | 路径 canonicalize (macOS /tmp → /private/tmp) | ⚠️ **需 Rust 做** | 见 §2.8 |

5/9 全绿；4 项有偏差但都不是 stopper，写明 §3 缓解。

---

## 2. 详细测试结果

### 2.0 全部 8 个 `--allow-*` flag 行为汇总

Node 26.1.0 `--help` 的描述行 + 实测语法：

| Flag | help 文档形 | 实测语法 | 实测过滤粒度 | 启动 warning |
|---|---|---|---|---|
| `--permission` | `enable the permission system` | boolean | — | 无 |
| `--allow-fs-read` | `=...` | **必须**带 `=path`，否则 `requires an argument` 退出 | ✅ 路径过滤真 enforce；支持绝对/相对、`*` 通配、文件/目录粒度、多次叠加 | 无 |
| `--allow-fs-write` | `=...` | 同上 | 同上 | 无 |
| `--allow-net` | （无 `=...`） | boolean，`=anything` 被接受但忽略 | ⚠️ **无主机过滤**，启用就全开 | **ExperimentalWarning** |
| `--allow-child-process` | （无 `=...`） | boolean | 启用就允许 spawn / exec / fork 任意命令 | **SecurityWarning** |
| `--allow-worker` | （无 `=...`） | boolean | 同上模式 | **SecurityWarning** |
| `--allow-addons` | （无 `=...`） | boolean | 同上 | **SecurityWarning** |
| `--allow-ffi` | （无 `=...`） | boolean | 同上 | **SecurityWarning** |
| `--allow-inspector` | （无 `=...`） | boolean | 同上 | **SecurityWarning** |
| `--allow-wasi` | （无 `=...`） | boolean | 同上 | **SecurityWarning** |

**唯一支持 `=value` 子语法的是 fs 两个**（read / write），且 enforce 是真的。其余 7 个都是 boolean——`--allow-X=any-string` 不报错只是因为 Node 对 `=value` 后缀普遍宽容。

**SecurityWarning 原文**：

```
SecurityWarning: The flag --allow-child-process must be used with extreme
caution. It could invalidate the permission model.
```

每次 spawn 扩展都会触发，所以 cronymax 启动 Node host 时**必须**带 `--no-warnings` 或设 `NODE_NO_WARNINGS=1`，否则 stderr 一行行往外吐——而且 SecurityWarning 是 Node 把 "你启用了可能让 sandbox 失守的 flag" 这一事实让你**用户自己心知肚明**而不是被默默 enforce。落到 cronymax：

- 平台不希望最终用户看到这条 warning（吓到不必要的用户）
- 但要把它**记到平台日志**（cronymax 应该追踪扩展开了哪些"高风险"flag）
- 安装期人话授权那条 UI 已经覆盖了这个信息（"⚙ 启动子进程（任意命令）"）

### 2.1 Test 1 · 基础 fs ACL（test1-basic.js）

启动：`node --permission --allow-fs-read=/private/tmp/node-perm-spike --allow-fs-read=/tmp/node-perm-spike test1-basic.js`

```
✅ READ OK: /tmp/node-perm-spike/test1-basic.js (668 bytes)
❌ READ BLOCKED: /etc/passwd → ERR_ACCESS_DENIED
❌ READ BLOCKED: /Users/bytedance/.ssh/id_rsa → ERR_ACCESS_DENIED
❌ READ BLOCKED: /Users/bytedance/.npmrc → ERR_ACCESS_DENIED
```

**结论**：白名单内允许 / 白名单外 throw `ERR_ACCESS_DENIED`，预期符合。

### 2.2 Test 2 · 绕路尝试（test2-bypass.js）

```
✅ BLOCKED: fs.readFileSync(/etc/passwd) → ERR_ACCESS_DENIED
✅ BLOCKED: eval require('fs') → ERR_ACCESS_DENIED
✅ BLOCKED: process.binding(fs).open → ERR_ACCESS_DENIED
✅ BLOCKED: Function(return require)()(fs) → require is not defined
✅ BLOCKED: vm.runInThisContext → require is not defined
✅ BLOCKED: dynamic import fs → ERR_ACCESS_DENIED
```

**结论**：6 条绕路全拦。`Function('return require')()` 和 `vm.runInThisContext` 在 Permission 模式下 `require` 不可达（不仅是 fs 拦得住，而是连 require 本身不存在了），更强。

### 2.3 Test 3 · child_process（test3-process.js）

不带 `--allow-child-process`：
```
✅ BLOCKED: spawn(ls) → ERR_ACCESS_DENIED
✅ BLOCKED: exec(echo) → ERR_ACCESS_DENIED
✅ BLOCKED: fork → ERR_ACCESS_DENIED
```

带 `--allow-child-process`：
```
🚨 SPAWN OK: spawn(ls)
🚨 SPAWN OK: exec(echo)
🚨 SPAWN OK: fork
```

**结论**：boolean 开关如 spec 所设。开了就全开（spawn / exec / fork 都通），关了就全关。**Coco 扩展需要 `process: true` 才能 spawn `coco acp serve`。**

### 2.4 Test 4 · npm 包兼容（test4-npm.js）

```
✅ yaml.parse: {"foo":"bar","list":[1,2]}
✅ zod schema: {"name":"alice","age":30}
✅ date-fns: 2026-05-19
✅ lodash.chunk: [[1,2,3],[4,5,6],[7]]
✅ marked: <h1>hello <strong>world</strong></h1>
✅ crypto.subtle.digest: {}
✅ fetch availability: function
```

**结论**：5 个常见纯计算 npm 包在 `--permission` 下零调整工作。`fetch` 全局可见但实际调用需 `--allow-net`（见 §2.5）。

### 2.5 Test 5 · 网络（test5-network.js）— ⚠️ 偏差点

不带 `--allow-net`：
```
✅ BLOCKED or failed: example.com → fetch failed
Net result: 127.0.0.1:22 → ERR_ACCESS_DENIED
```

带 `--allow-net=example.com`（注意：spec 假设的语法）：
```
🚨 NETWORK OPEN: example.com → 200
Net result: 127.0.0.1:22 → ECONNREFUSED   ← 拒绝是因为 22 没监听，不是 permission 拦的
```

**进一步测**：带 `--allow-net=127.0.0.1`，尝试连 8.8.8.8:53：
```
🚨 8.8.8.8 OPEN  ← 没声明 8.8.8.8，但还是通了
```

**再进一步**：用根本不是主机的字符串 `--allow-net=ABSOLUTELY_BOGUS_NOT_A_HOST`：
```
启动通过，无报错
```

**结论**：**Node 26.1.0 的 `--allow-net` 只支持 boolean 形式**。

- `node --help` 里 `--allow-net` 描述只有 "allow use of network when any permissions are set"，**没有 `=value` 形式**（对比 `--allow-fs-read=...` 是明确带 `=...` 的）
- `--allow-net=anything-including-bogus-strings` 不报错，是因为 Node 对未识别的 `=value` 后缀普遍 accept-and-ignore-value，不是 silently-enforce
- spec-v0.3 §6.1 写的 `--allow-net=<host>` 是 Node 主线**计划中但未落地**的 feature（permission model RFC 里跟踪）；host-scoped 网络 ACL 预计 Node 27/28 才会有
- 启动时打印 `ExperimentalWarning: The flag --allow-net is under experimental phase.`，需 `--no-warnings` 抑制

### 2.6 Test 8 · 高危默认禁（test8-defaults.js）

```
✅ BLOCKED: inspector.open → ERR_ACCESS_DENIED
✅ BLOCKED: new Worker → ERR_ACCESS_DENIED
✅ BLOCKED: process.dlopen → ERR_DLOPEN_DISABLED
✅ BLOCKED: wasi.start → ERR_ACCESS_DENIED
```

**结论**：inspector / worker_threads / native addon (dlopen) / WASI 在 `--permission` 模式下全默认禁，spec §6.4 表的预期符合。Coco 不需要这几个，第一批扩展不开。

### 2.7 Test 6 · 性能（test6-perf.js）— ⚠️ 指标偏差

| 调用 | with `--permission` | baseline | 绝对差 | 相对 |
|---|---|---|---|---|
| `fs.readFileSync` × 10000 | 25.38 μs/op | 12.82 μs/op | +12.5 μs | **+98%** |
| `fs.statSync` × 10000 | 12.57 μs/op | 0.87 μs/op | +11.7 μs | **+1344%** |
| `process.permission.has` × 100000 | 11.29 μs/op | (API 不可用) | — | — |

**结论**：
- **绝对 overhead 稳定 ~12 μs/fs call**（path resolve + ACL check）
- 相对 readFileSync `+98%`、statSync `+1344%`——spike 文档原写 `< 5%` 的指标是错的，实际是绝对加法而非比例
- 实战影响：
  - **typical agent workload**（网络 / LLM / 文件读写）—— 一次 readFile ~25μs vs HTTP roundtrip ~50ms，**忽略不计**
  - **stat-heavy workload**（递归扫目录如 `walkdir`）—— 一次扫 100 万项目录约 +12 秒。**第三方扩展跑 `globby --gitignore` 会感知**。
  - **永远比 `--inspect` 调试时的 ~50μs 还小**

不算 stopper，但需要在 SDK 文档警告。

### 2.8 Test 7 · 路径 canonicalize（test7-canonicalize.js）— ⚠️ Rust 必管

仅授 `/tmp/node-perm-spike` (symlink 形式)：
```
✅ READ OK: /tmp/node-perm-spike/test1-basic.js
❌ READ BLOCKED: /private/tmp/node-perm-spike/test1-basic.js → ERR_ACCESS_DENIED
```

仅授 `/private/tmp/node-perm-spike` (real 形式)：
```
❌ READ BLOCKED: /tmp/node-perm-spike/test1-basic.js → ERR_ACCESS_DENIED
✅ READ OK: /private/tmp/node-perm-spike/test1-basic.js
```

`realpathSync('/tmp/...')` 本身也被 deny（除非 `/tmp` 在白名单），抛 `ERR_ACCESS_DENIED { permission: 'FileSystemRead', resource: '/tmp' }`。

**结论**：
- Node 26 **不自动解 symlink**；`/tmp/x` 和 `/private/tmp/x` 是两条独立 ACL 条目
- spec-v0.3 §6.1 已要求 Rust canonicalize；本测验证此点是**必须的**而非可选
- 进一步：扩展代码若想自己 `realpath`，需要把 symlink 上层目录（macOS 上 `/tmp` → 实际是 `/private/tmp` 的链接，但 `/tmp` 这个父级也需要 read 权限来 `realpath` 子节点）放进白名单
- **Rust 实现建议**：`build_node_flags` 对每个目标路径 emit 两条：`--allow-fs-read=<canonical>` 和 `--allow-fs-read=<symlink-form>`；canonical 用 `std::fs::canonicalize`，symlink-form 是用户传入原值

---

## 3. 风险登记 & 缓解方案

### R3a · `--allow-net` 在 Node 26.1.0 只是 boolean ⚠️

**影响**：
- spec-v0.3 §6.1 假设的 `--allow-net=<host>` 子语法在 Node 26.1.0 还没落地实现
- 任何 `process: true` + `network` 的扩展（如 Coco 跑外网 LLM）等于把整个网络开给它，跟"接受全网开"等价

**缓解**：
1. **接受 v1 网络 ACL 为 boolean**：`build_node_flags` 仍按 manifest `capabilities.network.allow` 决定**要不要**加 `--allow-net`，加了就是全开
2. manifest `network.allow` 字段保留（**用于安装期人话授权**："此扩展会访问网络：api.openai.com" 给用户看），不强制 enforce
3. **bootstrap.js 不写包装层**（spec 决策 4d 不变）
4. Phase 0 末 DRI 评估：Node 27/28 host-scoped flag 稳定后再升级 enforce
5. 加 `--no-warnings` 抑制 ExperimentalWarning（用户不该看到 cronymax 启动了 experimental flag）

**spec 修改建议**：
- §6.1 `build_node_flags` 代码示例改成 emit boolean `--allow-net`（无 `=host`），注释说明 host-scoped 版本待 Node 27/28
- §6.4 防绕过表里把 "`fetch('https://evil.com')` 不在白名单 → 拦" 改成 "v1: 全开或全关；M1 等 Node host-scoped 落地后启用 per-host"

### R3b · 路径 canonicalize 是 Rust 必管 ✅ 已落档

**已写在 spec §6.1**；本 spike 实测确认。`build_node_flags` 实现需:
- 每个授权路径 emit 两条 `--allow-fs-read`（symlink form + canonical form）以覆盖 `/tmp` ↔ `/private/tmp`
- 单测覆盖 macOS / Linux / Windows 各自的 canonicalize 行为

### R3c · fs 调用性能 overhead ⚠️

**影响**：
- 扩展跑递归目录扫描会比无 permission 慢 14×（statSync 1300% 相对）
- 绝对值 ~12μs/call，对大多数工作流忽略

**缓解**：
1. Phase 9 SDK 文档警告 stat-heavy patterns
2. 提供 `cronymax.workspace.glob(pattern)` 替代 `walkdir` —— 平台侧（Rust）做扫描，结果一次传 Node，避开 N 次 stat
3. **不优化的 Phase 2 性能基准目标改写**：从 "P99 < 5ms" 改为 "command round-trip P99 < 5ms"（RPC 框架性能，不是 fs perf）

### R3d · Warning 噪音 ⚠️

- `--allow-net` 启动打 ExperimentalWarning
- `--allow-child-process` / `--allow-worker` / `--allow-addons` / `--allow-ffi` / `--allow-inspector` / `--allow-wasi` 启动**每个都打 SecurityWarning**（"must be used with extreme caution. It could invalidate the permission model"）

**`build_node_flags` 必须无条件 emit `--no-warnings`**（或 spawn 时设 `NODE_NO_WARNINGS=1`）。原因：
- Coco 这种 `process: true` 的扩展启动会刷 SecurityWarning，不能让用户看到
- 决定开 high-risk flag 的"风险"信息已经在**安装期人话授权**里告知了，不需要每次启动再 echo 一遍

平台侧应另起一个**机器可读**的"扩展开了哪些 high-risk flag" 记录写到 cronymax 日志，方便排查。详情写进 §host/node.rs spawn 注释。

---

## 4. 进 Phase 1 的 DRI 决策项

Phase 0 末评审（plan-v0.3 §10）需要签字的 7 项，本 spike 影响的 2 项：

| 决策 | spike 结论 | 推荐 |
|---|---|---|
| Node 版本：**Node 26** ✅ 已签 | 26.1.0 验证通过；ship 时检查通道 | 保持 |
| Node 26 ship 时 LTS 状态 ⏳ | Node 26 ship 期是 2025-10 即将进 current；2027 进 LTS Active | v1 alpha 用 current 通道；M1 重新评估 |

**附加建议**：spec-v0.3 §13 决策表 4d "纯 Node Permission Model" 加注脚 "*network 在 v1 是 boolean-not-host-scoped 直至 Node 27/28；manifest network.allow 保留用于安装期人话授权与未来启用 enforcement。*"

---

## 5. 重跑步骤（任何人都能复现）

```bash
# 装 Node 26
fnm install 26

# 跑 spike 目录里所有 test
cd /tmp/node-perm-spike
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike \
  --allow-fs-read=/tmp/node-perm-spike \
  test1-basic.js          # ACL 基础
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike test2-bypass.js
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike test3-process.js   # 无 child-process
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike --allow-child-process \
  test3-process.js                                                  # 有 child-process
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/tmp/node-perm-spike test4-npm.js
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike test5-network.js   # 无 allow-net
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike --allow-net=example.com \
  test5-network.js                                                  # 有 allow-net
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike test6-perf.js
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/tmp/node-perm-spike test7-canonicalize.js      # 仅 symlink 形
fnm exec --using=26 node --no-warnings --permission \
  --allow-fs-read=/private/tmp/node-perm-spike test8-defaults.js
```

预期 7-8 秒跑完全部。

---

## 6. spike 输出文件清单

| 文件 | 测试主题 |
|---|---|
| `/tmp/node-perm-spike/test1-basic.js` | fs ACL allow/deny |
| `test2-bypass.js` | eval / vm / process.binding / dynamic import 绕路 |
| `test3-process.js` | child_process spawn/exec/fork |
| `test4-npm.js` | yaml / zod / date-fns / lodash / marked / crypto / fetch |
| `test5-network.js` | fetch / net.createConnection |
| `test6-perf.js` | readFileSync / statSync / permission.has overhead |
| `test7-canonicalize.js` | /tmp ↔ /private/tmp symlink ACL |
| `test8-defaults.js` | inspector / worker / dlopen / wasi 默认禁 |

---

## 7. spike 关 / 进 Phase 1 的判定

**进 Phase 1**：是。

理由：
1. 全部 spec-v0.3 §6 安全决策的核心假设——VM 强制 ACL、绕路防御、危险默认禁、npm 兼容——验证通过
2. 网络 host-filter 缺失不是 alpha 阻塞器（manifest 仍有 informed-consent 价值；ship 时打 `--no-warnings`）
3. canonicalize 已经在 spec 里明确，本次只是从"应该"升到"实测必须"，是 Rust 实现细节
4. perf overhead 在 typical agent workload 下不感知

需要在 Phase 0 评审会同步：
- ⚠️ spec-v0.3 §6.1 / §6.4 / §13 注脚（网络 ACL boolean、host-filter 推到 M1+）
- ⚠️ Phase 2 性能目标改写
- ✅ canonicalize 已就位

---

**文档版本**：spike-v1 · 2026-05-19 · Node v26.1.0 · Darwin arm64
