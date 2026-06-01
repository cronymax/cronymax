# Permission Model 撤回决策记录

**日期**：2026-05-20
**决策范围**：cronymax 扩展平台 v1 alpha
**前置状态**：spec-v0.3 §6（"安全模型 · α 阶段"）原设计 Node 26 Permission Model + manifest declared capability flags + 安装期 per-cap consent UI

---

## TL;DR

去除 Node 26 Permission Model 整套（spawn 时**不**带 `--permission`、**不**带任何 `--allow-*`）。Manifest 的 `capabilities.{fs,network,process,workers,native_addons,secrets,events.subscribe,events.emit,ui-slots,extension-points,auth.providers}` 全部撤回（schema 还接受，但不解释）。安装弹窗不列风险、不列 capability。Audit log 整套撤回。

**保留**：per-extension Node host（崩溃隔离）、`cronymax.*` namespace 锁定（platform-RPC 层）、operational logging（`host.log` / `output.log` / `extension-host.log` 用于 debug，不是 security audit）。

---

## 决策时序

| 时间 | 设计状态 |
|---|---|
| Phase 0 评议（2026-05-20）通过 | Node 26 capability flag gate 是核心承诺 |
| Phase 1 实施 | manifest `capabilities.fs` 平台变量白名单、`build_node_flags` 翻译表、19 单测覆盖各种 deny case 都已落地 |
| Phase 2 实施 | bootstrap.js / host spawn / fd 3 RPC 真实跑通；e2e 测试验证 `/etc/passwd` 真被 Node 拒 |
| 设计讨论 | 用户提出"加 workspace folder 后扩展看不见新 folder 因为 flag 不可变"问题 |
| 阶段性结论 | 撤回整个 permission model |

## 推动撤回的理由

### 1. Agent 类扩展的 UX 死结

Coco（spec §11 的 dogfood 扩展）是 LLM agent。用户跟它说"帮我看下 `~/notes/foo.md`"或者"参考 `/Users/alice/Documents/spec.pdf`"是常态。

- manifest 在**安装时**写死能访问哪些目录
- 用户在**运行时**临时让 agent 看新文件
- 这两者**对不上**。开发者无法预测 / 限制用户会让 agent 访问什么

VS Code 选择"扩展默认有完整 fs 权限"恰恰是为了让这类场景可用。cronymax 当时的逆向选择（更严格的 cap gate）在 agent 场景下变成 UX 死结。

### 2. Workspace 切换的运行时困境

Node 26 实测确认（见 [`node26-permission-spike.md`](node26-permission-spike.md) §2.6 + 本 session 复测）：

```js
typeof process.permission.deny  === 'undefined'
typeof process.permission.grant === 'undefined'
// Object.getOwnPropertyNames(process.permission) === ['has']
```

flag 一旦 spawn 后**完全不可变**。意味着：

- 用户加一个 workspace folder → 新 folder 对已活扩展的 fs flag 集合外 → 扩展看不见 → 必须**杀掉重启**整个进程
- 用户在做长任务（譬如 coco 正 stream LLM）时加 folder → 杀掉等于丢任务进度

我们花了好几轮讨论 "idle 静默重启 / busy 排队 / per-workspace host" 各种妥协，每个都引入新的复杂度。**根因**是"manifest 静态 declared fs gate"跟"workspace 动态变化"的结构冲突。

### 3. Node 26 `realpathSync` 隐藏要求

实测发现：require('/tmp/foo/main.js') 触发 `realpathSync` 对每个**祖先符号链接**单独施加 fs-read 检查。macOS 上 `/tmp` `/var` `/etc` 都是 symlink，意味着授权 `/tmp/foo/ext` 还不够，得也授权 `/tmp` 本身。

这让"开发者声明 fs 路径"的心智模型变得**不可预测** —— 开发者声明的明明对，运行时 Node 还会因为不相关的祖先 symlink 拒掉。错误信息里 `resource: '/tmp'` 把开发者整懵了。

可以加复杂的"祖先 symlink 自动追加"逻辑兜底，但越加越脏。

### 4. 跟 VS Code 调研之后的市场定位

[VS Code 扩展宿主调研](https://github.com/microsoft/vscode/issues/79782) 证实：

- VS Code 一个窗口共享一个 EH 进程 → 单扩展崩溃带翻所有扩展（800+ 用户 issue）
- VS Code 扩展默认有完整 Node API 权限 → 没有 capability gate

cronymax 的差异化卖点应该是：
- ✅ **per-extension host**（VS Code 没有的崩溃隔离）—— 保留
- ❌ **per-extension cap gate**（更严格的 OS gate）—— 撤回

撤回后，cronymax 的真实卖点是"VS Code 同位的扩展开放度 + 比 VS Code 强的崩溃隔离"。这个组合是真的差异化；之前那个"capability gate"卖点撤回后反而更聚焦。

### 5. 撤回后的代码净影响

| 文件 | 变化 |
|---|---|
| `capability.rs` | 461 行 → 110 行（删 19 个 build_node_flags 测试） |
| `manifest.rs` | 删 fs path 白名单 PLATFORM_VARS 表、5 个 validator 子函数中的 fs / contributions 两个、24 个 fs path 测试 |
| `logging.rs` | 删 AuditWriter / `audit::` 10 个事件常量 / `LogKind::ExtensionAudit` / 2 个 audit 测试 |
| `bootstrap.js` | 删所有 `rpcNotify("audit", ...)` / `rpcNotify("log/console", ...)` / `rpcNotify("log/error", ...)`；activate 的 try/catch-then-audit 也撤回 |
| `error.rs` | 删 `FsPathInvalid` / `ContributionNotDeclared` 两个变体 |
| `tests/p2_node_host_e2e.rs` | 删 `fs_permission_denial_propagates_to_bootstrap_log` 测试 |
| **总计** | **删码 ~1500 行**，删 42 个单测 |

仍然 0 clippy warning / fmt clean / 全部 196 测试过（189 lib + 4 acceptance + 3 e2e）。

## 撤回前 vs 撤回后的能力对比

| 项 | 撤回前 | 撤回后 |
|---|---|---|
| 扩展 fs 访问范围 | manifest 申报 + Node 26 强制 | 完整（任何用户级路径） |
| 扩展 spawn 子进程 | 申报 `process: true` 才行 | 任何 child_process 调用都行 |
| 扩展 fetch / 网络 | 申报 `network` 才能联网 | 任何网络调用都行 |
| 扩展加载 native addon | 申报 `native_addons: true` | 任何 `.node` 都行 |
| 扩展读 `~/.ssh/id_rsa` | ❌ 拦 | ❌ 不拦 |
| 扩展 emit 假冒 `cronymax.*` 事件 | ✅ 拦 | ✅ 拦（platform-RPC 层） |
| 一个扩展 crash 影响别的 | ❌ 不会（per-ext host） | ❌ 不会（保留） |
| 加 workspace folder | 需要重启扩展（flag 重 emit） | 不需要重启（folder 信息走 RPC notify） |
| 安装弹窗 | per-cap 同意列表 + 风险标注 | 只有"由 \<publisher\> 提供，是否安装" |
| 撤销单扩展某项权限 | UI 可撤（下次 spawn 不 emit） | 不存在该概念 |
| Audit log | 每扩展独立 `audit.log`（activate.ok/failed/crashed/etc.） | 只有 host.log / output.log 操作日志 |

## 风险 / 反对意见 / 被劝退的备选

### 反对方案 A：保留 fs gate，撤其余

考虑过"只去除 network / process 那些 boolean gate，保留 fs path scoped grant"。原因是 fs 的细粒度 gate 阻拦面最大。

**为什么没采纳**：fs gate 同样卡 agent UX（Section 1）；保留它就保留了 workspace 切换的 spawn 重启难题（Section 2）。半截方案两头不到岸。

### 反对方案 B：保留 install-time per-cap 同意，撤运行时强制

考虑过"manifest 仍写 `capabilities`，安装时给用户看，但不再 enforce"。

**为什么没采纳**：这把"用户以为有效的 cap 列表"和"实际平台不强制"两件事混在一起，比 VS Code 那种"什么都不许诺"更危险（用户看了同意列表会以为有 OS 拦截，实际没有）。要么真做，要么完全不做。

### 反对方案 C：opt-in 严格模式

考虑过让安全敏感型扩展（譬如 linter / formatter / 渲染器）可以 manifest 写 `"sandbox": "strict"` 主动启用 Node permission。

**为什么没采纳**：v1 alpha 没有用户会用这个 opt-in；增加未来 v2 设计自由度的"留个钩子"对当下零产出。延后。M1 阶段如果有人申请再说。

### 真正的风险

- **恶意扩展可以读 `~/.ssh` 等高敏文件**。靠用户的发行方信任来防。如果未来出现 cronymax 上的真实事故，加 §7 那种平台层 sandbox-exec 兜底
- **用户可能误以为安装弹窗"由 X 提供"足够安全**。需要文档 / 安装 UI 文案明确告诉用户"扩展有完整本地权限"

## 后续工作（不阻塞 v1 alpha）

1. **撤销脚本**：用户发现自己装了不该信任的扩展 → 一键 `cronymax ext uninstall` + 删 storage 目录。已就位
2. **发行方信任 UI**：安装时显示 GitHub URL / stars / 最近发布时间等元数据，帮用户判断
3. **运行时观察**：`extension-host.log` 已记录 spawn / crash / restart 事件。要不要扩展性地观察 fs 访问模式？v1 alpha 不做（需要 monkey-patch fs，违反"不做 require hook"原则）
4. **§7 OS sandbox 兜底**：M1+ 视情况实施。要写**平台级**配置（cronymax 全局开关），不是扩展级

## 决策签字

单人项目，签字 = 把这份决策记录写下来。
