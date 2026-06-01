# Cronymax 扩展日志系统 · 设计 v0.2

> 状态：Phase 0 评议引出的新设计，纳入 v1 alpha 范围
> 触发：评议 §B（warning 抑制）暴露的更上层问题——"普通用户、扩展开发者、客服分别怎么排错"
> 参考：VS Code 的 Output panel / per-extension log / extension host 错误处理
> 历史：v0.1（推 M1）→ **v0.2 当前**（按 VS Code 对齐，纳入 v1）

---

## 0. 一句话目标

cronymax 给每个扩展一份**结构化、分级、可定位、可导出**的日志面，让：

- **普通用户**遇到扩展不工作时，能在 UI 内一键看到"它说了什么"
- **扩展开发者**写代码时 `console.log` 不丢，且能用 `createOutputChannel` 拿到分 channel / 分级的 VS Code 风格 API
- **客服**远程拿用户日志包不用让用户翻 ~/.cronymax 目录
- **平台**永远知道每个扩展开过哪些 high-risk flag、发生过哪些 permission 拒绝、崩溃多少次

---

## 1. 五个角色 · 五条排错路径

| 角色 | 触发场景 | 期望路径 |
|---|---|---|
| **普通用户 Alice** | "Coco 不回答了" | 设置 → 扩展 → Coco → **日志** tab；channel 下拉选 "Coco/ACP"；看红色 error；右上"复制到剪贴板 / 反馈" |
| **扩展开发者 Bob** | 自己开发 Mermaid 渲染器 | `cronymax ext dev ./mermaid --watch`：终端实时滚 stdout/stderr + 平台事件；自己 `createOutputChannel` 写的内容同时进 channel 文件 |
| **cronymax 内部开发者 Carol** | 怀疑 RPC framing bug | 命令面板 "Developer: Show Logs..." → 选 "Extension Host (bytedance.coco)" → 滚动 + 过滤 |
| **客服 Dave** | 用户报"装不上" | "帮助 → 导出诊断包"生成 `cronymax-diag-<ts>.zip`（含全部 session 的日志 + manifest 清单 + 脱敏）→ 用户上传 |
| **SRE Eve** | 大规模故障定位 | v1：本地 NDJSON 可解析；M1：可选 OTLP 上报 |

---

## 2. 四类日志源

每个扩展同时产生四类记录。落到**同一根目录**下不同文件，便于关联。

### 2.1 `host.log` — Node 进程层（stderr）

Node host 自身的 stderr。cronymax `Stdio::piped()` 抓住后追加。包括：

- 扩展 `console.error / console.warn`（Node 默认走 stderr）
- 扩展未捕获异常的栈（Node 默认 dump 到 stderr）
- Node 自身的 SecurityWarning / ExperimentalWarning（如果没 `--no-warnings`）

**纯文本**，不解析。

### 2.2 `output.log` — stdout 兜底（裸文本）

Node host 的 stdout。包括：

- 扩展 `console.log / console.info / console.debug`
- 扩展直接 `process.stdout.write`

**纯文本**，不解析。这是 VS Code 风格的"扩展 console.log 兜底"——即使扩展不用 `createOutputChannel` SDK，输出也不丢。

### 2.3 `channels/<channel-id>.log` — 结构化 channel 输出

扩展通过 `cronymax.window.createOutputChannel(name)` 创建的 channel。每 channel 一个文件，**NDJSON 格式**：

```json
{"t":"2026-05-19T19:42:01.234Z","level":"info","msg":"Coco 启动","args":[]}
{"t":"2026-05-19T19:42:01.301Z","level":"error","msg":"ACP handshake failed","args":[{"code":"ETIMEDOUT"}]}
```

`<channel-id>` 由 channel name kebab-slugify 而来（"Coco / ACP" → `coco-acp`）。

### 2.4 `audit.log` — 平台事件层

只有平台写，扩展只读。NDJSON：

```json
{"t":"...","event":"activate","ms":127}
{"t":"...","event":"activate.failed","error":{"name":"TypeError","message":"...","stack":"..."}}
{"t":"...","event":"flags.high_risk","flags":["--allow-child-process","--allow-net"]}
{"t":"...","event":"rpc.error","method":"commands/execute","ms":4.2,"code":"timeout"}
{"t":"...","event":"uncaught","error":{...}}
{"t":"...","event":"unhandledRejection","error":{...}}
{"t":"...","event":"hung","last_rpc":"agents/prompt"}
{"t":"...","event":"crashed","exit":139,"restart_count":1}
{"t":"...","event":"deactivate","reason":"user"}
{"t":"...","event":"disabled","reason":"too_many_crashes","restart_count":3}
```

---

## 3. 文件落地结构

```
~/.cronymax/logs/
├── <session-id>/                       ← cronymax 每次启动 = 一个 session
│   ├── platform.log                    ← cronymax 自己
│   ├── extension-host.log              ← host manager 跨扩展事件
│   └── extensions/
│       ├── bytedance.coco/
│       │   ├── host.log                ← §2.1 (stderr 兜底)
│       │   ├── output.log              ← §2.2 (stdout 兜底)
│       │   ├── audit.log               ← §2.4 (平台写)
│       │   └── channels/
│       │       ├── coco.log            ← createOutputChannel("Coco")
│       │       ├── coco-acp.log        ← createOutputChannel("Coco / ACP")
│       │       └── coco-requests.log
│       └── acme.mermaid-renderer/
│           ├── host.log
│           ├── output.log
│           ├── audit.log
│           └── channels/
│               └── mermaid-renderer.log
└── current → <latest-session-id>       ← symlink
```

**Session id**：cronymax 启动时生成 `YYYY-MM-DDTHH-MM-SS-<random4>`。

**Rotate**：
- 单文件 > 10 MB → 切到 `<name>.log.1`，最多 3 个历史
- 整个 logs/ > 500 MB → 从最老 session 整目录删
- session 总数 > 50 → 同上

**保留**：默认 7 天 / 50 session 限额（取严），settings 可配。

---

## 4. SDK API（mirror VS Code，纳入 v1 IDL freeze）

### 4.1 新增 IDL `cep-idl/v1/logging.ts`

```ts
// FROZEN 单次例外说明：见 §11 风险 L5

import { Disposable } from "./primitives";

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

export interface OutputChannel extends Disposable {
  readonly name: string;
  /** 写一行纯文本（不分级）。 */
  appendLine(value: string): void;
  /** 写文本不带换行。 */
  append(value: string): void;
  /** 显示这个 channel 到 Output 面板 / 设置 → 扩展 → 日志 tab。 */
  show(preserveFocus?: boolean): void;
  /** 清空内存 buffer + 落盘文件。 */
  clear(): Promise<void>;
}

/** `createOutputChannel(name, { log: true })` 返回这个，多 5 个分级方法。 */
export interface LogOutputChannel extends OutputChannel {
  trace(message: string, ...args: unknown[]): void;
  debug(message: string, ...args: unknown[]): void;
  info(message: string, ...args: unknown[]): void;
  warn(message: string, ...args: unknown[]): void;
  error(message: string | Error, ...args: unknown[]): void;
}

export interface CreateOutputChannelOptions {
  /** 等于 true 返回 LogOutputChannel；否则普通 OutputChannel。 */
  log?: boolean;
}
```

### 4.2 `cep-idl/v1/window.ts` 增量

```ts
export interface Window {
  // ... 原有方法 ...

  /**
   * 创建一个命名 output channel。在设置面板 / 命令面板的 "显示日志" 下拉里
   * 可见。Disposable — 调用 dispose 移除（日志文件保留直至 rotate）。
   */
  createOutputChannel(name: string): OutputChannel;
  createOutputChannel(name: string, options: { log: true }): LogOutputChannel;
}
```

### 4.3 扩展端用法

```ts
import * as cronymax from "@cronymax/extension";

export async function activate(ctx: cronymax.ExtensionContext) {
  // 推荐：用 log channel（带分级）
  const log = cronymax.window.createOutputChannel("Coco / ACP", { log: true });
  ctx.subscriptions.push(log);

  log.info("Coco 启动");
  try {
    await connect();
  } catch (e) {
    log.error("ACP handshake failed", e);
    log.show();   // 弹日志面板
  }

  // 也可以：纯文本 channel
  const trace = cronymax.window.createOutputChannel("Coco / Trace");
  ctx.subscriptions.push(trace);
  trace.appendLine("step 1");
}
```

### 4.4 console.* 兜底（不需要扩展显式适配）

即使扩展不用 `createOutputChannel`：

- `console.log/info/debug` → Node 写 stdout → 平台 pipe → `output.log`
- `console.warn/error` → Node 写 stderr → 平台 pipe → `host.log`

设置面板"日志" tab 的 channel 下拉里**自动出现两条**："stdout (console.log)" 和 "stderr (console.error)"，让用户不知道扩展用什么 API 都能找到日志。

---

## 5. UI 入口

### 5.1 设置 → 扩展 → `<ext-id>` → "日志" tab（v1 简版）

```
┌─ 扩展：Coco ─────────────────────────────────────┐
│ [常规] [权限] [日志▼] [配置]                       │
├───────────────────────────────────────────────────┤
│ Channel: [Coco / ACP                         ▼]  │
│           ├ stdout (console.log)                  │
│           ├ stderr (console.error)                │
│           ├ Coco                                  │
│           ├ Coco / ACP                  ←current  │
│           └ Coco / Trace                          │
│ Level:   [全部 ▼]   时段: [最近 1 小时 ▼]          │
│ ────────────────────────────────────────────────  │
│ 19:42:01.234 [info]  Coco 启动                     │
│ 19:42:01.301 [error] ACP handshake failed          │
│   ETIMEDOUT                                        │
│ ...                                                │
│                                                    │
│ [清空]  [导出 .log]  [复制]  [打开日志夹]          │
└───────────────────────────────────────────────────┘
```

v1 实现简版：channel 下拉 + 文本滚动 + 时段过滤；level 过滤先只对 channel `log: true` 类生效。M1 加搜索、正则、多 channel 合并视图。

### 5.2 命令面板

- "Developer: Show Logs..." → 二级下拉 `platform / extension-host / <ext1> / <ext2>` → 进入查看器
- "Developer: Open Log Folder" → 系统 file manager 打开 `~/.cronymax/logs/current/`

### 5.3 dev 模式

`cronymax ext dev <dir> --watch`：

- 扩展 stdio piped + **同时**透传到当前终端
- 平台 audit event 实时 echo 到终端（不同颜色：`[activate]` 绿，`[crashed]` 红，`[rpc.error]` 黄）
- 文件改动 → kill + 重新激活
- 终端透传**不抑制** Node SecurityWarning / ExperimentalWarning（开发者需要看到）

---

## 6. 客服流程

新增 CLI 子命令 + UI 入口：

```
$ cronymax diagnostic-bundle
✓ 收集当前 session（platform.log + extension-host.log）
✓ 收集所有已激活扩展 host.log / output.log / audit.log / channels/*
✓ 收集最近 3 个 session（rotate 保留范围内）
✓ 收集 manifest 列表（去 secrets）
✓ 收集 cronymax 版本 / OS / Node 版本
✓ 脱敏：HOME → ~  ·  Authorization 头 → REDACTED  ·  audit.log 的 args 字段整列删
✓ 生成 cronymax-diag-2026-05-19T19-42-01.zip (3.2 MB)

诊断包路径：~/Downloads/cronymax-diag-2026-05-19T19-42-01.zip
```

UI 入口：帮助 → 导出诊断包。

---

## 7. **Extension Host 错误处理 · 6 层（核心）**

cronymax **每扩展独立 Node 进程**，比 VS Code 的共享 EH 精度高、爆炸半径小。下表对每层定义平台行为。

### A. 扩展 `activate()` 同步 throw / activate Promise reject

**bootstrap.js**：
```js
try {
  const mod = require(extEntryPath);
  await mod.activate(buildContext());
  rpcNotify("extension/activate.ok");
} catch (e) {
  rpcNotify("extension/activate.failed", { error: serializeError(e) });
  process.exit(1);     // 让平台知道这次是"激活失败 exit"，不是崩溃
}
```

**平台**：
- 收到 `activate.failed` → audit.log `{event:"activate.failed", error}`
- 扩展面板红 badge
- Toast：「扩展 Coco 激活失败 [查看日志]」
- **不自动重启**——是 bug，重试无用
- 状态：installed + enabled + **activated-with-errors**（在 "已启用/未激活" 之上加 errors 标记）

### B. 扩展 `console.log/warn/error`

- stdout/stderr 走 §2.1/2.2 兜底文件
- 同时 bootstrap.js 拦截 `console.*`（global 替换，不是 require hook，**不违反 spec §6.1**）→ RPC notify `log/console` → 平台落到 audit 路径 + 设置面板下拉里"stdout/stderr" 自动 channel

```js
// bootstrap.js
const orig = { ...console };
for (const lvl of ["log","info","warn","error","debug","trace"]) {
  console[lvl] = (...args) => {
    orig[lvl](...args);                        // 仍写原 stream（让 §2.1/2.2 兜底）
    rpcNotify("log/console", { lvl, args });   // 平台同时知道
  };
}
```

### C. 扩展 `unhandledRejection` / `uncaughtException` 非 activate 期

**bootstrap.js**：
```js
process.on("uncaughtException", (e) => {
  rpcNotify("extension/uncaught", { error: serializeError(e) });
  // 不 process.exit —— mirror VS Code，让扩展可能 limp on
});
process.on("unhandledRejection", (e) => {
  rpcNotify("extension/unhandledRejection", { error: serializeError(e) });
});
```

**平台**：
- audit.log `{event:"uncaught"|"unhandledRejection", error}`
- emit 平台事件 `cronymax.extension.errored`（订阅者可以是"扩展面板" UI 把状态显示成 ⚠）
- **不主动 kill**——扩展自己定义 boundary，下一个 RPC 可能还能服务
- 多次（默认 ≥ 5 次/分钟）→ 升级为 hung 处理（见 D）

### D. 扩展死循环 / 阻塞 event loop

RPC 层定时 ping（spec §10 已提"健康检查"）。

**平台**：
- 主进程每 5s 通过 socket 发 `host/ping` notify
- 期望扩展进程 10s 内回 `host/pong`
- 错过 2 次 ping = 20s 没响应 → 标 hung
- audit.log `{event:"hung", last_rpc:"agents/prompt"}`
- Toast：「Coco 卡住了 [强制重启] [等等看]」
- 用户选重启 / 等待；自动等 60s 不恢复 → 自动 kill + restart（计入 restart_count）
- 跟 VS Code 比：**只 kill 这一个扩展**，其他扩展继续

### E. Node 进程真崩（OOM / segfault / `Error` 没人接 → exit non-zero）

cronymax wait child 进程，监听 exit code。

**平台**：
- exit code ≠ 0 → audit.log `{event:"crashed", exit, signal, restart_count}`
- 自动 restart 计数 +1
- 计数 ≤ 3（spec §10 默认 N=3）→ 静默 respawn
- 计数 > 3 → 标记 disabled-by-platform
  - Toast：「Coco 多次崩溃已禁用 [重新启用] [查看日志]」
  - audit.log `{event:"disabled", reason:"too_many_crashes"}`
  - 用户重新启用 → 计数清零，重新激活

### F. 扩展自己调 `process.exit()`

- exit code = 0 + **没有**先收到正常 `extension/deactivate.ok` 握手 → 视为"异常退出"
- audit.log `{event:"abnormal_exit"}`
- 按 E 处理（restart 计数 +1）
- 防止扩展用 exit 当"我不行了" 替代 throw——平台仍然 auto-restart

### 错误处理矩阵小结

| 层 | 检测方 | bootstrap | 平台 | 重启策略 |
|---|---|---|---|---|
| A activate throw | bootstrap | try/catch | audit + toast | **不** 重启 |
| B console.* | bootstrap | global 替换 | log/console RPC | n/a |
| C uncaught/rejection | bootstrap | process.on | audit + 事件 | 不主动；超频升 D |
| D 阻塞 event loop | 平台 ping/pong | — | toast + 等用户 / 60s 自动 kill | restart 计数 |
| E 进程崩 | OS exit | — | audit | restart 计数 ≤ 3，超后禁用 |
| F process.exit() | OS exit + 握手缺失 | — | 同 E | 同 E |

### 实装状态（2026-06-01 · P10-T01 / P10-T02）

D / E / F 已落地(`extensions/host/node.rs` 的 exit-watcher + health monitor → `HostEvent` mpsc → `extensions/runtime.rs` 的 supervisor)。与本节原设计的两处差异:

- **`audit.log` 撤回**:随 v1 撤回 Node 26 Permission Model 一并删除(见 [`permission-removal.md`](permission-removal.md))。crashed / hung 事件现记入 `host.log`(NDJSON,与 stderr drain 同格式,在 Logs tab 合并展示)+ 结构化 `tracing`,**不是** security audit。
- **D(hung)直接升级为 restart**:`$/ping` 超时 → 平台**直接** kill + restart(不再「等用户 / 等 60s」),按 E 的计数走。
- **E/F 重启策略(实装)**:exit≠0 / 异常退出 / hung → 自动重启,**退避 立即 → 500ms → 2s**;计数 **> 3**(`max_restarts` 默认 3,可配)→ `set_disabled_by_crash`(registry 持久化 `disabled_reason: "crash"`)+ **toast 通知**(`extensions/notice` topic)。设置面板显示 **Disabled (crashed)**,手动 Enable 清零重试。
- **进程泄漏修复(配套)**:app 退出经 `ExtensionRuntime::shutdown_all`(优雅)/ `kill_all_blocking`(硬退出)+ bootstrap.js fd-3-close → `process.exit(0)`,三层兜底,不再 orphan node host。
- **内存观测(P10-T02 缩版)**:health tick 采样 RSS,超阈值(默认 ~1.5 GB)→ 日志告警 + 一次 toast,**不杀**(与 install-time 信任模型一致;不做硬资源强杀)。

端到端验证见 `tests/p10_host_lifecycle_e2e.rs`(真 Node 26:kill→restart / 风暴→disable+notice / shutdown_all 无 orphan)。

---

## 8. console.* / Node warnings / Extension throw 的统一处理（更新）

| 来源 | bootstrap.js 行为 | 落到哪 |
|---|---|---|
| 扩展 `console.log/info/debug` | global 替换：原 console 写 stdout + RPC `log/console` | `output.log` + 设置面板 "stdout" 自动 channel |
| 扩展 `console.warn/error` | 同上，原 console 写 stderr | `host.log` + 设置面板 "stderr" 自动 channel |
| 扩展 `createOutputChannel(name).info(...)` | SDK 内直接 RPC `log/channel`，不经 console | `channels/<id>.log` |
| 扩展 throw（unhandled）| `process.on('uncaughtException')` → RPC `extension/uncaught` | `host.log`（Node 默认 stderr dump）+ `audit.log` |
| Node SecurityWarning / ExperimentalWarning | `--no-warnings` 抑制；平台 spawn 时**直接**写一次 `audit.log {event:"flags.high_risk", flags:[...]}`（已知信息）| `audit.log` |
| Node host 进程崩 | wait exit | `audit.log {event:"crashed"}` |

**spec §6.1 "0 行 require 劫持"守得住**：bootstrap.js 替换的是 `console.*`（global 对象方法）和注册 `process.on('uncaughtException')`（公开 event）——这两个都不是 `require` 函数本身。

---

## 9. v1 alpha 范围 / M1+ 范围

### v1 alpha（必须）

- ✅ §3 文件落地（4 类 log + channel 子目录）
- ✅ §2 host.log / output.log / channels/*.log / audit.log
- ✅ §4 `createOutputChannel` + `LogOutputChannel` SDK
- ✅ §5.1 设置面板"日志" tab 简版（channel 下拉 + 文本滚动 + 时段过滤）
- ✅ §5.2 命令面板 Show Logs / Open Log Folder
- ✅ §5.3 dev 模式终端透传
- ✅ §6 `cronymax diagnostic-bundle` CLI
- ✅ §7 6 层错误处理 A-F 全实现
- ✅ §8 console.* 拦截 + process.on hook（bootstrap.js）
- ✅ IDL `cep-idl/v1/logging.ts` 新增（freeze 例外，见 §12-L5）

### M1+（推迟）

- 结构化 channel（table / tree）
- 设置面板内嵌完整 viewer（搜索 / 正则 / 多 channel 合并）
- OTLP / 远程上报
- `audit.log` 加更多事件类型（permission grant 历史 / RPC 详细 trace）
- 跨扩展依赖图日志

---

## 10. Phase 计划影响

### 加任务

| 任务 | Phase | 估时 |
|---|---|---|
| `cep-idl/v1/logging.ts` + window.ts patch + tsc 重过 | P0 补 | 0.5d |
| `extensions/logging.rs`：4 类 log 文件 writer + rotate + diagnostic-bundle | P2 末 | 2d |
| `bootstrap.js` console.* + process.on + activate try/catch + ping/pong | P2 末 | 1d |
| 6 层错误处理 § A-F 平台侧实现 + 单测 | P2 末 + P10 | 1.5d |
| audit log writer（活/崩/flag/uncaught/hung 全部）| P2 + P3 | 1d |
| SDK `createOutputChannel` 实现 + RPC `log/channel` | P3 | 1d |
| `cronymax ext dev --watch` 透传（已在 P9-T05） | P9 | +0.5d 增量 |
| 设置面板"日志" tab UI（简版） | P9 | 2d |
| 命令面板 "Show Logs" / "Open Log Folder" | P9 | 0.5d |
| 文档：扩展开发者日志最佳实践 + 6 层错误手册 | P10 | 1d |

**净增工程量**：约 **~11 天**。10-12 周计划尾部仍可吸收。

### 改任务

- **P0-T02 IDL freeze 状态变为"v1 发布前增量"**：logging.ts 是 freeze 之后的补丁。需 2 人 review 后**正式补进 v1 freeze**，不算破规则（因为 v1 还没 ship）。同步加进 IDL README 的"freeze 例外史" 段。
- **P2 性能基准**：增加"日志写入不阻塞 RPC 路径"的 sanity（异步 tokio 后台 writer）
- **P9 设置面板**：增加"日志" tab（spec §11 设置面板里已有"权限"tab，加一个）

---

## 11. VS Code 对照

| 维度 | VS Code | cronymax v1 |
|---|---|---|
| 进程模型 | 共享 Extension Host | **每扩展独立 host**（精度更高） |
| OutputChannel API | `vscode.window.createOutputChannel(name, {log:true}?)` | **完全对齐** |
| Output panel | 底部独立 panel + channel 下拉 | 设置 → 扩展 → 日志 tab + channel 下拉 |
| console.* 兜底 | 自动到 DevTools console + auto channel | stdout/stderr → output.log/host.log + 设置面板自动 "stdout"/"stderr" channel |
| activate 失败 | 红 badge + notification | **同** + audit.log 结构化 |
| uncaughtException | toast + 不死 EH | **同** + audit.log + 平台事件 |
| EH 卡住 | heartbeat → 杀**整个 EH** | ping/pong → 仅杀**单扩展** |
| EH 崩 | 全 EH 重启，**所有扩展** | 单扩展 auto-restart ≤ 3，超后单扩展 disable |
| `process.exit()` | 跟 EH 崩混 | **可识别**（没收到 deactivate 握手） |
| 日志目录 | `~/Library/.../Code/logs/<session>/exthost1/<ext>/` | `~/.cronymax/logs/<session>/extensions/<ext>/` |
| 远程拿日志 | "Open Logs Folder" 让用户 zip | **`cronymax diagnostic-bundle`** 自动脱敏 + zip |

**关键优势**：per-ext host 让 D（卡住）/E（崩）/F（自杀）的精度从"哪个扩展？基本靠猜"变成"就是这个扩展，毫无歧义"。客服场景显著省力。

---

## 12. 风险

| # | 风险 | 缓解 |
|---|---|---|
| L1 | 扩展疯狂 `console.log`，日志爆盘 | per-channel rate limit (1MB/min 软限) + 500MB total 拦；超量截断写"...truncated..." |
| L2 | 敏感数据漏到 audit log | 脱敏白名单（默认 `args` 字段在 diagnostic-bundle 里整列删）；开发者文档警告"audit 是给平台读的，别把 secret 塞 args" |
| L3 | 日志写入阻塞 Node host 主线程 | 平台 Rust 端异步 tokio task；bootstrap.js 用 `setImmediate` 投递 RPC notify |
| L4 | Session 切换 in-flight 丢失 | 关 cronymax 时 flush + tokio shutdown 等 5s；崩溃靠 OS page cache（接受 < 1s 丢失） |
| **L5** | **logging.ts 加进 v1 freeze 破坏 freeze 政策** | **单次例外，写进 README**："v1 freeze 政策 = v1 ship 后只增不删；v1 ship 前允许补丁。本次补丁 = logging.ts。需 2 人 review。" |
| L6 | bootstrap.js 拦 console.* 跟 spec §6.1 "0 行 require 劫持" 冲突 | **不冲突**：console.* 是 global 对象方法替换，process.on 是公开 event 注册——都不动 require。spec §6.1 注释加澄清。 |
| L7 | 6 层错误处理 ping/pong overhead | 5s 间隔 × 单包 ~10 bytes × N 扩展 = 忽略不计；改可配 |
| L8 | 扩展挂 `process.on('uncaughtException')` 把平台 handler 覆盖了 | bootstrap.js 在 main.js 加载**之前**注册（保证是 listener[0]）；扩展挂的是 listener[1+]；扩展即使 swallow 也不影响平台 RPC 通知 |

---

## 13. 决议落档

Phase 0 评议结论：

| 项 | 决议 |
|---|---|
| 日志系统纳入 v1 alpha | ✅ 接受 |
| `createOutputChannel` SDK 进 v1 | ✅ 接受 |
| `cep-idl/v1/logging.ts` 作 v1 freeze 单次例外 | ✅ 接受（2 人 review） |
| bootstrap.js 替换 console.* + process.on hook 不违反 §6.1 | ✅ 接受（write 进 spec §6.1 澄清注释） |
| §B 决策：永远 `--no-warnings` + audit log | ✅ 接受（与本设计 §7+§8 一致） |
| 净工程量 ~11 天纳入 10-12 周计划 | ✅ 接受 |

---

**文档版本**：v0.2 · 2026-05-19 · 作者：Phase 0 评议
**前版**：v0.1（极简 stdout-only，推 OutputChannel 至 M1）—— 评议否决
