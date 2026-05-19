# Cronymax 扩展平台 — 设计文档 v0.3

状态：**Phase 0 评议通过（2026-05-20），Phase 1 启动中**

替换关系：**本文档替代 spec-v0.2**。v0.2 的"Node 22 LTS + 网络软门控 + child_process 平台 wrap"已废弃。v0.3 目标 Node 26（Permission Model 完整版），所有 capability 全靠 Node VM 层强制，平台仅写极少量 bootstrap.js hook（console.* 拦截 + EH 错误处理）；不写 require 劫持 / 命令白名单 / 网络包装 / 命名审计 hook。

**Phase 0 评议引入的修订**：详见 §16.1 速查表 + [`phase-0-review.md`](phase-0-review.md)。配套文档：[`extension-logs.md`](extension-logs.md)（日志系统设计）、[`node26-permission-spike.md`](node26-permission-spike.md)、[`msgpack-rpc-spike.md`](msgpack-rpc-spike.md)、[`legacy-agent-step.md`](legacy-agent-step.md)。

---

## 0. 设计目标与非目标

cronymax 是 **agent workflow 应用**。

### 目标

- 第三方在**不修改 cronymax 核心代码**的前提下，向平台贡献 agent provider / tool / content renderer / view / settings page / theme / channel / fs provider 等
- **完整 Node 生态兼容**：扩展是普通 Node 包，npm 直接可用
- **极简安全实施**：Node 26 Permission Model 一条龙处理 fs / network / process / worker / addons / ffi / inspector，平台不写包装层
- **DX 贴近 VS Code 习惯**
- **平台可演化**：加新扩展点走 RFC 流程
- **α→γ 升级路径丝滑**：v1 上 Node Permission，M1 加 OS sandbox 外层

### 非目标（v1）

- 代码编辑器扩展性（LSP / DAP / grammar / formatter / linter / code lens）
- WASM runtime
- 跨扩展正式契约系统（schema + semver 版本管理）—— 用 VS Code 同款 `extension.exports`
- 远程开发模式（SSH / WSL / dev container）
- Marketplace 数字签名
- OS-level sandbox（M1+）
- 共享 Node host
- 内置 LLM provider 迁移到扩展模型
- per-command spawn 白名单 / 审计 / 网络包装（v0.2 设计的，现在确认不需要）

---

## 1. 总体架构

### 三层 + 进程模型

```
┌──────────────────────────────────────────────────────────────────┐
│ CEF Renderer Process                                             │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐ │
│  │ cronymax UI (React)  │  │ 扩展 Webview iframes (sandbox)    │ │
│  │ chat / flows / inbox │  │ 设置页 / 侧栏 view / 渲染器        │ │
│  └──────────────────────┘  └──────────────────────────────────┘ │
└──────────────────────────┬───────────────────────────────────────┘
                           │ CEF browser query / event
                           ▼
┌──────────────────────────────────────────────────────────────────┐
│ Rust Core (cronymax 主进程)                                       │
│  Extension Manager · L2 Registry · L1.5 Event Bus · RPC Server   │
└─────┬──────────────────────┬───────────────────────┬─────────────┘
      │ 每扩展独立 Unix socket（Win Named Pipe）+ MessagePack-RPC  │
      ▼                      ▼                       ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│ Node host #1     │  │ Node host #2     │  │ Node host #N     │
│ bytedance.coco   │  │ acme.mermaid     │  │ ...              │
│                  │  │                  │  │                  │
│ node 26 \        │  │ node 26 \        │  │ node 26 \        │
│  --permission \  │  │  --permission \  │  │  --permission \  │
│  --allow-fs-     │  │  --allow-fs-     │  │  ...             │
│   read=ws \      │  │   read=ws \      │  │                  │
│  --allow-fs-     │  │                  │  │                  │
│   write=ws \     │  │                  │  │                  │
│  --allow-net=    │  │                  │  │                  │
│   api.foo.com \  │  │                  │  │                  │
│  --allow-child-  │  │                  │  │                  │
│   process \      │  │                  │  │                  │
│  bootstrap.js    │  │  bootstrap.js    │  │  bootstrap.js    │
│                  │  │                  │  │                  │
│ ↓ vm.Context     │  │ ↓ vm.Context     │  │ ↓ vm.Context     │
│ main.ts + deps   │  │ main.ts + deps   │  │ main.ts + deps   │
└────────┬─────────┘  └──────────────────┘  └──────────────────┘
         │
         ▼
   ┌──────────────────┐
   │ coco acp serve   │
   └──────────────────┘
```

### 安全模型分层

| 层 | 实现 | 何时上 |
|---|---|---|
| **L0 · 进程隔离** | Node host 独立 OS 进程，扩展崩不拖死 cronymax | v1 |
| **L1 · 静态契约 + 安装期授权** | manifest 申报 capabilities；安装时用户审 | v1 |
| **L2 · VM 强制（Node Permission）** | `--permission --allow-*` 由 Rust 根据 manifest 拼出，**Node 全管**：fs / network / child_process / worker / addons / ffi / inspector | **v1** |
| **L3 · OS Sandbox（外层）** | sandbox-exec / bubblewrap / Job Object，kernel 级 | **M1** |
| **L4 · Marketplace + 签名 + 行为监控** | 社会化机制 | M2+ |

α(v1) → γ(M1) 是叠加式升级，不破坏扩展兼容。

---

## 2. L1 Kernel — 14 个原语

| 原语 | 实现 | Capability flag |
|---|---|---|
| `lifecycle` | `activate(ctx)` / `deactivate()` | n/a |
| `capabilities` | manifest 静态声明 | n/a |
| `commands` | 注册具名可调用 | 自家命名空间永远允许 |
| `events` | pub/sub topic | `events.subscribe` / `events.emit` 白名单 |
| `config` | get/update + onDidChange | `config.schema` 申报 |
| `secrets` | Keychain / DPAPI / secret-service | `secrets.namespace` 锁前缀 |
| `storage` | per-extension state KV | n/a |
| `process` | 真 Node child_process | **Node `--allow-child-process`** |
| `fs` | 真 Node fs | **Node `--allow-fs-read/-write`** |
| `network` | 真 Node fetch / WebSocket / net | **Node `--allow-net`**（Node 26+）|
| `ui-slots` | 标识贡献槽位 | `ui-slots` 列表 |
| `webview` | CEF iframe + postMessage 中转 | n/a |
| `auth` | 内置 OAuth / PKCE / device-flow | `auth.providers` 白名单 |
| `extensions` | getExtension + exports | n/a |

**关键**：fs / network / process 这三大块**全部由 Node Permission Model 在 VM 层强制**。平台代码不写任何 require 劫持、命令白名单或网络包装层。

### SDK 形状

```ts
declare namespace cronymax {
  function activate(handler: (ctx: ExtensionContext) => Promise<void>): void;

  const commands: { register; execute; };
  const events:   { on; emit; };
  const workspace: {
    rootUri: URI | undefined;
    fs: WorkspaceFileSystem;            // 包装真 Node fs，提供 URI 抽象
    getConfiguration(section?): Configuration;
    onDidChangeConfiguration(handler): Disposable;
  };
  const process:  { spawn(opts): Promise<ChildProcess>; };  // 直接调真 child_process
  const network:  { fetch; websocket; };                    // 真 Node fetch
  const secrets:  { get; set; delete; };
  const window:   { showInformationMessage; createWebviewPanel; openConfigPage; };

  const agents:  { registerProvider; getProvider; };
  const chat:    { ... };
  const renderers: { ... };

  const extensions: { getExtension; readonly all; };
  const env:        { appName; platform; homedir; machineId; };
}
```

扩展可直接 `import { spawn } from "child_process"` 或 `fetch(...)`，**Node 自己拦截不合规调用**。

---

## 3. L1.5 平台事件主题

平台命名空间 `cronymax.*` 下的事件。扩展通过 `cronymax.events.on(topic, handler)` 订阅，需 manifest `capabilities.events.subscribe` 显式声明。

### v1 入门 8 条

| 主题 | payload |
|---|---|
| `cronymax.session.started` | `{ sessionId, providerId, agentId?, model }` |
| `cronymax.session.ended` | `{ sessionId, reason }` |
| `cronymax.message.user.sent` | `{ sessionId, turnId, text }` |
| `cronymax.message.assistant.delta` | `{ sessionId, turnId, textDelta }` |
| `cronymax.message.assistant.done` | `{ sessionId, turnId, fullText, finishReason }` |
| `cronymax.tool.invoked` | `{ sessionId, turnId, toolCallId, name, input, source }` |
| `cronymax.tool.completed` | `{ sessionId, toolCallId, status, output }` |
| `cronymax.permission.requested` | `{ sessionId, requestId, target, options }` |

---

## 4. L2 Extension Points

12 候选；v1 起步 6 个。

| EP | M0 |
|---|---|
| `cronymax.command` | ✅ |
| `cronymax.keybinding` | M1 |
| `cronymax.menu.item` | M1 |
| `cronymax.config.schema` | ✅ |
| `cronymax.config.page` | ✅ |
| `cronymax.ui.activitybar.item` | M1 |
| `cronymax.ui.sidebar.view` | ✅ |
| `cronymax.ui.statusbar.item` | M1 |
| `cronymax.content.renderer` (block) | ✅ |
| **`cronymax.agents.provider`** | ✅ |
| `cronymax.chat.tool` | M1 |
| `cronymax.workspace.fs-provider` | M1 |

### `cronymax.agents.provider` 完整规范

**清单声明**：
```jsonc
"contributes": {
  "cronymax.agents.provider": [{
    "id": "coco",
    "label": "Coco",
    "icon": "$(coco)",
    "description": "ByteDance Coco via ACP",
    "supportsModels": true,
    "supportsModes": true,
    "supportsMcp": true
  }]
}
```

**Runtime 注册**：
```ts
cronymax.agents.registerProvider(id: string, impl: AgentProvider): Disposable;

interface AgentProvider {
  listModels(): Promise<ModelInfo[]>;
  modes?: ModeInfo[];
  createSession(opts: SessionOptions): Promise<AgentSession>;
}

interface SessionOptions {
  cwd: string;
  model?: string;
  mode?: string;
  mcpServers?: McpServerSpec[];
  systemPrompt?: string;
  allowedTools?: string[];
}

interface AgentSession {
  readonly id: string;
  prompt(message: PromptMessage, token: CancellationToken): AsyncIterable<AgentEvent>;
  cancel(): Promise<void>;
  dispose(): Promise<void>;
}

type AgentEvent =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "toolCall"; id; name; input; source; status: "in_progress" }
  | { kind: "toolCallUpdate"; id; status: "completed" | "failed"; output }
  | { kind: "permissionRequest"; requestId; tool; options }
  | { kind: "done"; stopReason };
```

**平台消费方**：聊天面板 + flow runtime（共享同一份 registry）。

---

## 5. Node Host 进程模型

| 维度 | 选择 |
|---|---|
| Node 版本 | **Node 26**（Permission Model 完整版含 `--allow-net` / `--allow-ffi` / `--allow-inspector`）|
| Node 来源 | 打包进 cronymax build |
| 隔离粒度 | **每扩展独立 OS 进程** |
| 启动 | Lazy activate（被需要时才起）|
| 回收 | 不主动 deactivate（cronymax 退出才退）|
| IPC | Unix socket（Win Named Pipe）+ MessagePack-RPC |
| 模块系统 | CJS 主推；ESM M1 完善 |

### 资源占用

- 单 host 稳态 ~50-80MB
- 首次激活 80-200ms
- 典型重度用户（8 扩展）~500MB

VS Code 等量场景 EH 一进程 ~500-1000MB；cronymax 独立 host 模型 RAM 近似但隔离更强。

### 启动延迟缓解

1. cronymax 启动时按 "上次会话用过" 列表静默预热常用扩展
2. 激活时 UI spinner（200ms 内可接受）
3. 不主动 deactivate
4. M1 评估 host 池预热 / `--snapshot-blob`

---

## 6. 安全模型 · α 阶段（v1）

### 6.1 一份代码：build_node_flags

```rust
// crates/cronymax/src/extensions/capability.rs
fn build_node_flags(
    manifest: &Manifest,
    ctx: &ExpansionCtx,   // {workspace, ext_dir, ext_storage, ext_global_storage, home, tmp}
) -> Vec<String> {
    // 平台基础设施 —— 永远 emit，不归用户 capability：
    let mut flags = vec![
        "--permission".to_string(),
        "--no-warnings".to_string(),  // 抑制 SecurityWarning / ExperimentalWarning
    ];

    // 平台必给的 fs（扩展私有存储），永远 rw
    for path in [&ctx.ext_dir, &ctx.ext_storage, &ctx.ext_global_storage] {
        emit_fs(&mut flags, path, "rw");
    }

    // 用户 manifest 申报的 fs（数组形式 + 平台变量）
    for fs_spec in &manifest.capabilities.fs {
        let expanded = expand_vars(&fs_spec.path, ctx)?;         // {WORKSPACE}/x → /Users/.../x
        // 路径 traversal 检查：expanded canonicalize 后必须仍在变量根下
        let canonical = std::fs::canonicalize(&expanded)
            .unwrap_or_else(|_| expanded.clone());                // 路径不存在时退回 expanded
        emit_fs(&mut flags, &canonical, &fs_spec.mode);
        if canonical != expanded {                                // symlink: 双填
            emit_fs(&mut flags, &expanded, &fs_spec.mode);
        }
    }

    // network: v1 boolean，按需 emit（不带 =host —— Node 26.1.0 不支持主机过滤）。
    // manifest.network.allow 仅作安装期人话授权 UI 用。
    if manifest.capabilities.network.is_some() {
        flags.push("--allow-net".to_string());
    }

    // 其余 boolean flag
    if matches!(manifest.capabilities.process, Some(true)) {
        flags.push("--allow-child-process".to_string());
    }
    if matches!(manifest.capabilities.workers, Some(true)) {
        flags.push("--allow-worker".to_string());
    }
    if matches!(manifest.capabilities.native_addons, Some(true)) {
        flags.push("--allow-addons".to_string());
    }
    // ffi / inspector / wasi v1 一律不开

    flags
}

fn emit_fs(flags: &mut Vec<String>, path: &Path, mode: &str) {
    flags.push(format!("--allow-fs-read={}", path.display()));
    if mode == "rw" {
        flags.push(format!("--allow-fs-write={}", path.display()));
    }
}
```

**这就是全部 flag 部分**。bootstrap 不写任何 require 劫持、命令白名单、网络包装、审计 hook。Node 26 Permission Model 全管。

### 6.1.1 RPC 通道：inherited fd 3，不走 Unix socket

Spawn Node host 时 stdio 配置：

```rust
Command::new(&node_bin)
    .args(&flags)
    .arg(bootstrap_js)
    // stdio: [stdin, stdout, stderr, fd3]
    //         ╲ pipe ╲ pipe → output.log ╲ pipe → host.log ╲ pipe ← RPC 通道
    .stdio_with_extras([Pipe, Pipe, Pipe, Pipe])
    .spawn()
```

子进程 Node 端 bootstrap.js：

```js
const net = require("node:net");
const rpc = new net.Socket({ fd: 3 });    // wrap inherited fd; 不需要 --allow-net
```

**关键**：早期设计假设 RPC 走 Unix socket，但 Node 26 把 socket `connect` 也算 network ACL（实测 spike P0-T04），强迫平台 emit `--allow-net`，跟"网络是用户 capability"语义冲突。改 RPC 走 stdio fd 3 后：

- RPC 通道不触发 Node permission（inherited fd）
- `--allow-net` 真正回归为用户 capability（manifest `network` 申报才 emit）
- 安装期人话授权 UI 上"网络"那条变诚实

### 6.1.2 平台变量集

manifest `fs.path` 必须用以下变量（白名单封闭集）：

| 变量 | 展开值 | 永远授权 |
|---|---|---|
| `{WORKSPACE}` | 当前 cronymax 工作区根（未开工作区时变量不可用）| ❌ 需扩展申报 |
| `{HOME}/<subpath>` | `$HOME` / `%USERPROFILE%`；裸 `{HOME}` 拒收 | ❌ |
| `{EXT_DIR}` | `~/.cronymax/extensions/<id>/` | ✅ 平台必给（只读） |
| `{EXT_STORAGE}` | `~/.cronymax/extensions/<id>/storage/` | ✅ 平台必给（读写） |
| `{EXT_GLOBAL_STORAGE}` | `~/.cronymax/global-state/<id>/` | ✅ 平台必给（读写） |
| `{TMP}/<subpath>` | `os.tmpdir()` | ❌ |
| `{CRONYMAX_CONFIG}/<subpath>` | `~/.cronymax/` | ❌ |

校验规则：
- 变量名不在白名单 → 安装期拒
- 绝对路径硬写（`/Users/...`）→ 安装期拒
- Path traversal（`..` 跳出变量根）→ 校验时 canonicalize 后比较根，跳出则拒
- 裸 `{HOME}` / 裸 `{TMP}` → 拒（必须有子路径）；`{WORKSPACE}` 可裸用

### 6.2 安装期人话授权

manifest capabilities 译成清单（all-or-nothing；取消 = 不安装；post-install 可在设置面板撤销单项）：

```
┌───────────────────────────────────────────────────────────┐
│ 安装 Coco                                                  │
│ by bytedance · v0.1.0                                      │
│                                                            │
│ 此扩展将能够：                                              │
│  📁 读写当前工作区（{WORKSPACE}）                            │
│  📁 读写 ~/.coco                                            │
│  ⚙ 启动子进程（任意命令）                                   │
│  🌐 访问网络（声明的具体域：api.openai.com；v1 不区分主机） │
│  🔑 存储 bytedance.coco.* 命名空间下的密钥                  │
│  💬 注册为聊天 agent provider                               │
│                                                            │
│ [详情]                          [取消]  [安装]              │
└───────────────────────────────────────────────────────────┘
```

UX 约定：
- 弹窗 all-or-nothing；【取消】= 不解压、不留痕迹；【安装】= 解压 + 写 registry + enabled
- 不显示"平台必给"项（`{EXT_DIR}` / `{EXT_STORAGE}` / `{EXT_GLOBAL_STORAGE}` / 平台 RPC 通道）—— 是基础设施而非用户授权
- 网络项**诚实标注**"v1 不区分主机"（Node 26.1.0 限制；M1+ host-scoped 落地后改回）
- post-install 在"设置 → 扩展 → \<ext\> → 权限"撤销单项（撤销 → 下次 spawn 不 emit 对应 flag → 扩展运行时拿 `ERR_ACCESS_DENIED`）
- 平台强制 flag（`--permission` / `--no-warnings` / `--allow-net` for RPC / 扩展私有存储 fs）UI 上不出现撤销勾选

### 6.3 命名空间锁定

| 命名空间 | 谁拥有 | 第三方写 |
|---|---|---|
| `cronymax.*` | 平台 | 安装期校验拒绝 |
| `<publisher>.*` | 该 publisher | 仅自家扩展 |

### 6.4 防绕过路径（Node 26 实测）

| 绕路尝试 | 结果 |
|---|---|
| `fs.readFileSync('/etc/passwd')` | ✅ `ERR_ACCESS_DENIED` |
| `eval("require('fs').readFileSync('/etc/passwd')")` | ✅ 拦 |
| `process.binding('fs').open(...)` | ✅ 拦 |
| `Function('return require')()` | ✅ require 不可达 |
| `vm.runInThisContext('require...')` | ✅ require 不可达 |
| `import('fs')` 动态导入 | ✅ 拦 |
| `fetch('https://evil.com')` — 扩展未声明 `network` | ✅ `--allow-net` 不 emit → 拦 |
| `fetch('https://evil.com')` — 扩展已声明 `network: ["api.openai.com"]` | ⚠️ v1 通（Node 26.1.0 `--allow-net` 是 boolean，不分主机；M1+ Node host-scoped 落地后才能拦） |
| `net.createConnection(...)` 不在白名单 | ✅/⚠️ 同上 |
| `process.dlopen` 加载 .so | ✅ `--allow-ffi` 默认禁 |
| 扩展自挂 inspector | ✅ `--allow-inspector` 默认禁 |
| Native addon | ✅ `--allow-addons` 默认禁 |

**剩余可能的攻击**（α 拦不住，γ OS sandbox 兜底）：
- 资源耗尽（无限循环 / 内存爆掉）—— 靠 Rust 监控 + 限额
- 时间侧信道 —— 几乎无法防
- 滥用授权范围内的能力（fs.workspace 给了 rw，扩展真写工作区）—— 这是用户授权的责任

---

## 7. 安全模型 · γ 阶段（M1）

OS sandbox 外层叠加，对扩展代码、manifest、SDK 零侵入：

```
Rust core spawn:
  sandbox-exec -f /tmp/<ext-id>.sb -- \
    node 26 --permission --allow-* bootstrap.js
                                                        ↑
                                              v1 已有的 Node host
```

| 平台 | 实现 | 工程量 |
|---|---|---|
| macOS | SBPL 模板 | ~1 周 |
| Linux | bubblewrap + seccomp BPF | ~1 周 |
| Windows | Job Object + AppContainer | ~2 周 |
| 共用：profile 生成 + 测试 | ~1 周 |
| **小计** | **4-5 周** |

---

## 8. 扩展清单 schema

```jsonc
{
  // 标识
  "id":        "publisher.name",     // 必须 publisher 前缀
  "name":      "Display Name",
  "version":   "1.0.0",
  "publisher": "publisher",
  "engines":   { "cronymax": "^1.0" },
  "main":      "./dist/main.js",

  // 描述
  "description": "...",
  "icon":      "./icon.png",
  "repository": "...",
  "license":   "MIT",

  // 激活事件
  "activationEvents": [
    "onAgentProvider:coco",
    "onCommand:coco.openSettings"
  ],

  // L2 EP 贡献
  "contributes": {
    "cronymax.agents.provider": [...],
    "cronymax.command": [...],
    "cronymax.config.schema": {...},
    "cronymax.config.page": [...]
  },

  // 能力（极简；fs 用平台变量数组）
  "capabilities": {
    "fs": [
      { "path": "{WORKSPACE}",          "mode": "rw" },
      { "path": "{HOME}/.coco",         "mode": "rw" },
      { "path": "{HOME}/.config/coco",  "mode": "r"  }
    ],
    "network": { "allow": ["api.openai.com", "*.openai.com"] },  // v1 仅人话授权用，不分主机
    "process": true,                              // boolean
    "workers": false,
    "native_addons": false,
    "secrets": { "namespace": "publisher.name.*" },
    "events.subscribe": ["cronymax.message.assistant.done"],
    "events.emit":      ["publisher.name.*"],
    "ui-slots":         ["sidebar", "settings"],
    "extension-points": ["cronymax.agents.provider", "cronymax.command"],
    "auth.providers":   ["oauth-generic"]
  },

  // 跨扩展依赖
  "extensionDependencies": ["other.extension"]
}
```

### 校验规则

- `id` 必须 `<publisher>.<name>`；`<publisher>` 必须等于 `publisher` 字段
- `contributes` key 必须以 `cronymax.` 开头且在 `capabilities.extension-points` 申报
- 任何往 `cronymax.*` 命名空间写入安装期拒
- `capabilities.fs[].path` 必须用平台变量（见 §6.1.2 表）；绝对路径硬写 / 未知变量 / path traversal 一律拒
- 路径在 Rust 侧展开变量 + canonicalize 后传给 Node（每路径 emit 两条 `--allow-fs-*`：canonical + 原 expanded，覆盖 symlink）

---

## 9. Webview 通信链路

扩展 webview iframe 跟扩展 main.ts 跨进程（webview 在 CEF Renderer，main.ts 在 Node host）：

```
扩展 webview iframe (CEF Renderer)
   │ panel.postMessage({type, payload})
   ▼
CEF 拦截 → cronymax-webview://<ext-id>/<panel-id>
   │ 经现有 browser query 上报 Rust
   ▼
Rust core 鉴权 + 转 Node host: rpc.notify("webview/postMessage", ...)
   ▼
Node host 路由到 vm.Context 内 panel.onDidReceiveMessage handler
```

反向同理。4 跳 IPC，配置页类低频交互无所谓。

---

## 10. 扩展生命周期

```
未安装 ──install──► 已安装/未启用 ──enable──► 已启用/未激活
                                                      │ activationEvent
                                                      ▼
                                              已激活/运行中
                                                      │ deactivate
                                                      ▼
                                              已启用/未激活
                                                      │ disable
                                                      ▼
                                              已安装/未启用
                                                      │ uninstall
                                                      ▼
                                                  未安装
```

- **install**：解压 .crx 到 `~/.cronymax/extensions/<id>/`，校验 manifest，弹授权（all-or-nothing；见 §6.2）
- **activate**：
  1. Rust 检查 host 池
  2. spawn Node 26 with `--permission --no-warnings --allow-* ... bootstrap.js`
     · stdio: `[pipe, pipe, pipe, pipe]`（fd 0 stdin / fd 1 stdout→output.log / fd 2 stderr→host.log / fd 3 RPC）
  3. bootstrap.js wrap fd 3 为 net.Socket，握手，调 `extension/activate`
  4. require main.js（运行在 Node 主 vm context；不另开 vm.Context）
  5. catch activate 抛出的异常 → audit.log + 通知用户激活失败
- **crash 与错误处理**：详见 [`extension-logs.md`](extension-logs.md) §7 6 层错误处理 A-F；exit code 非零或 ping/pong 超时 → 自动重启 ≤ 3 次 → 超后禁用并通知用户

---

## 11. 端到端走查：coco-acp

### 项目结构

```
coco-extension/
├── package.json
├── cronymax-extension.json
├── tsconfig.json
├── src/
│   ├── main.ts
│   ├── acp-client.ts          ← 直接 import { spawn } from "child_process"
│   ├── coco-session.ts
│   └── settings/
│       ├── index.html
│       └── index.ts
└── dist/                       ← esbuild CJS 输出
```

### cronymax-extension.json

```jsonc
{
  "id": "bytedance.coco",
  "name": "Coco",
  "version": "0.1.0",
  "publisher": "bytedance",
  "engines": { "cronymax": "^1.0" },
  "main": "./dist/main.js",

  "activationEvents": [
    "onAgentProvider:coco",
    "onCommand:coco.openSettings"
  ],

  "contributes": {
    "cronymax.agents.provider": [{
      "id": "coco",
      "label": "Coco",
      "icon": "$(coco)",
      "description": "ByteDance Coco via ACP",
      "supportsModels": true,
      "supportsModes": true,
      "supportsMcp": true
    }],
    "cronymax.command": [
      { "id": "coco.openSettings", "title": "Coco: Open Settings" }
    ],
    "cronymax.config.schema": {
      "title": "Coco",
      "properties": {
        "coco.binaryPath":   { "type": "string", "default": "coco" },
        "coco.defaultModel": { "type": "string", "default": "GPT-5.4" },
        "coco.defaultMode":  { "type": "string", "enum": ["default","plan","bypass_permissions"], "default": "default" },
        "coco.yolo":         { "type": "boolean", "default": false }
      }
    },
    "cronymax.config.page": [{
      "id": "coco.advanced",
      "title": "Coco · Advanced",
      "entry": "./dist/settings/index.html"
    }]
  },

  "capabilities": {
    "fs": [
      { "path": "{WORKSPACE}",  "mode": "rw" },
      { "path": "{HOME}/.coco", "mode": "rw" }
    ],
    "process": true,
    "network": { "allow": ["api.openai.com"] },
    "secrets": { "namespace": "bytedance.coco.*" },
    "ui-slots": ["settings"],
    "extension-points": [
      "cronymax.agents.provider",
      "cronymax.command",
      "cronymax.config.schema",
      "cronymax.config.page"
    ]
  }
}
```

### src/main.ts

```ts
import * as cronymax from "@cronymax/extension";
import { AcpClient } from "./acp-client";
import { CocoSession } from "./coco-session";

export async function activate(ctx: cronymax.ExtensionContext) {
  ctx.subscriptions.push(
    cronymax.agents.registerProvider("coco", {
      async listModels() {
        const client = await AcpClient.spawnEphemeral();
        try {
          await client.initialize();
          const probe = await client.newSession({
            cwd: cronymax.workspace.rootUri?.fsPath ?? process.cwd(),
            mcpServers: []
          });
          return probe.models.availableModels.map(m => ({
            id: m.modelId, label: m.name, description: m.description
          }));
        } finally {
          await client.shutdown();
        }
      },
      modes: [
        { id: "default", label: "Default" },
        { id: "plan", label: "Plan" },
        { id: "bypass_permissions", label: "Accept All Tools" }
      ],
      async createSession(opts) {
        const cfg = cronymax.workspace.getConfiguration("coco");
        const client = await AcpClient.spawn({
          binaryPath: cfg.get<string>("binaryPath"),
          yolo: cfg.get<boolean>("yolo")
        });
        await client.initialize();
        const acp = await client.newSession({
          cwd: opts.cwd, mcpServers: opts.mcpServers ?? []
        });
        if (opts.mode)  await client.setMode(acp.sessionId, opts.mode);
        if (opts.model) await client.setModel(acp.sessionId, opts.model);
        return new CocoSession(client, acp);
      }
    }),
    cronymax.commands.register("coco.openSettings", () =>
      cronymax.window.openConfigPage("coco.advanced"))
  );
}

export async function deactivate() {}
```

### src/acp-client.ts

```ts
import { spawn, ChildProcess } from "child_process";   // 真 Node 标准 API
import { createInterface } from "readline";

export class AcpClient {
  static async spawn(opts: { binaryPath?: string; yolo?: boolean }) {
    const args = ["acp", "serve"];
    if (opts.yolo) args.push("--yolo");
    const proc = spawn(opts.binaryPath ?? "coco", args, {
      stdio: ["pipe","pipe","pipe"]
    });
    return new AcpClient(proc);
  }
  // ... JSON-RPC framing
}
```

**注意**：直接 `import { spawn } from "child_process"`，没经任何 cronymax wrap。Node Permission Model 在 VM 层兜底（如果 manifest 没声明 process 就 throw）。

### 用户流程

```
1. cronymax ext install bytedance.coco-0.1.0.crx
   → 平台解压、校验、弹授权窗
   → 用户 Allow → 写入 registry.json

2. 用户开聊天面板
   → 平台扫已装扩展 manifest，agent picker 列出 "Coco"（未激活）

3. 用户点选 "Coco"
   → 触发 onAgentProvider:coco
   → Rust：spawn Node 26 host (stdio: [pipe,pipe,pipe,pipe]，fd 3 = RPC)：
     node --permission --no-warnings \
          --allow-fs-read=/Users/alice/work/cronymax-ws         \  ← {WORKSPACE} canonical
          --allow-fs-write=/Users/alice/work/cronymax-ws        \
          --allow-fs-read=/Users/alice/.coco                    \  ← {HOME}/.coco canonical
          --allow-fs-write=/Users/alice/.coco                   \
          --allow-fs-read=/Users/alice/.cronymax/extensions/bytedance.coco \  ← {EXT_DIR}（平台必给）
          --allow-fs-read=/Users/alice/.cronymax/extensions/bytedance.coco/storage \  ← {EXT_STORAGE}
          --allow-fs-write=/Users/alice/.cronymax/extensions/bytedance.coco/storage \
          --allow-fs-read=/Users/alice/.cronymax/global-state/bytedance.coco \
          --allow-fs-write=/Users/alice/.cronymax/global-state/bytedance.coco \
          --allow-net               \  ← v1 boolean；用户声明了 network capability
          --allow-child-process     \
          extension-host-bootstrap.js --ext=bytedance.coco
   → bootstrap.js wrap fd 3 为 net.Socket，握手
   → require main.js + 调 activate(ctx)
   → register agents.provider("coco", ...)

4. 用户配置 Coco / GPT-5.4 / plan，发消息
   → 聊天面板：provider.createSession()
   → main.ts: spawn("coco", ["acp","serve"]) ← Node 验过 --allow-child-process 允许
   → ACP initialize / session/new
   → 返回 CocoSession

5. session.prompt(msg)：
   → RPC 流式回事件
   → 聊天面板按 event.kind 渲染
```

### flows 里用 Coco

```yaml
id: code-reviewer
provider: coco                       # 引用 agents.provider 的 id
provider_config:
  model: GPT-5.4
  mode: plan
system_prompt: |
  You are a senior code reviewer...
allowed_tools:
  - cronymax.tool.shell
```

flow runtime 走到 `type: agent, agent: code-reviewer` 时，从 registry 拿 Coco provider 调 createSession，事件流回。**跟聊天面板路径等价**。

---

## 12. 平台代码改动清单

### `crates/cronymax/src/extensions/`（新增）

- `manifest.rs` — schema 解析 + 校验 + canonicalize
- `registry.rs` — 元数据扫描 + 状态持久化
- `activation.rs` — activationEvents 匹配
- `host/`
  - `node.rs` — Node 26 进程池 + spawn + 监控
  - `bootstrap/` — bootstrap.js + SDK 注入
- `capability.rs` — manifest → Node flags 转换
- `contributions/` — L2 EP 注册中心
- `events.rs` — L1.5 平台事件总线
- `rpc/` — MessagePack-RPC 服务端
- `api/` — cronymax/v1 Rust 实现
  - `commands.rs` / `events.rs` / `secrets.rs` / `agents.rs` / `window.rs` / `workspace.rs` / `extensions.rs`
  - **不需要** `fs.rs` / `process.rs` / `network.rs`（这些 Node 自己管）

### `web/src/panels/chat/`（修改）

- 替换硬编码 LLM provider 列表为 `registry.consume("cronymax.agents.provider")`
- session 创建/路由统一走 AgentProvider 接口
- 事件流转翻译统一格式

### `web/src/panels/flows/agents/`（新增）

- AgentList.tsx / NewAgentWizard.tsx / AgentEditor.tsx

### `crates/cronymax/src/flow/`（修改）

- agent step → 通过 AgentProvider registry 调度
- 加载 `.cronymax/agents/<id>.agent.yaml` 解析 provider + provider_config

### `crates/cronymax/src/capability/agent_loader.rs`（修改）

- AgentDef 加 provider + provider_config 字段

### `web/src/panels/settings/extensions/`（新增）

- 扩展列表 / 安装 / 卸载 / 启用 / 禁用 / 配置 / 撤销 capability

### `web/src/extensions/sdk/`（新增）`@cronymax/extension`

- TypeScript SDK，从 Rust IDL codegen 类型声明
- npm 包发布

### Node 26 二进制打包

- macOS arm64 / x64 / Linux x64 / Windows x64
- 总增量 ~50-75MB per platform

---

## 13. 决策表 v0.3（最终）

| # | 决策 | 终值 |
|---|---|---|
| 1 | 架构层数 | L1 Kernel + L1.5 Events + L2 EPs |
| 2 | 命名 | AgentProvider / AgentSession |
| 3 | 命名空间锁 | `cronymax.*` reserved；publisher 前缀强制 |
| 4 | 主要 runtime | **Node 26**（含 `--allow-net` / `--allow-ffi` / `--allow-inspector`）|
| 4a | Node 来源 | 打包进 cronymax |
| 4b | 进程模型 | **每扩展独立 host** + lazy activate + 不主动 deactivate |
| 4c | IPC | **Inherited fd 3** (stdio pipe) + MessagePack-RPC。**改自 Unix socket** —— Phase 0 spike 发现 Node 26 把 socket connect 算 network ACL，会强制平台 emit `--allow-net`，破坏"网络是用户 capability"语义。fd 3 wrap 不触发 ACL。 |
| 4d | Capability 实施 α | **纯 Node Permission Model + 极少量 bootstrap.js hook**（console.* 拦截 + process.on('uncaughtException') + activate try/catch；均为 global API 替换 / 公开 event 注册，**不**包含 require 劫持、命令白名单、网络包装）。Node 26.1.0 `--allow-net` 是 boolean（无主机过滤）；manifest `network.allow` 仅作安装期人话授权用。M1 等 Node 27/28 host-scoped 落地后升级。 |
| 4e | Capability 实施 γ | M1 加 OS sandbox 外层 |
| 5 | API 真相之源 | TS interface IDL → codegen .d.ts |
| 6 | RPC 编码 | MessagePack-RPC（`rmp-serde` ↔ `@msgpack/msgpack`；裸数组帧；P99 实测 18-30μs）|
| 7 | 清单文件名 | `cronymax-extension.json` |
| 8 | 跨扩展通信 | VS Code 同款 `extension.exports`（无 schema / semver）|
| 9 | UI 沙箱 | CEF iframe 独立 origin；跨进程经 Rust 中转 |
| 10 | v1 L2 EP 数 | 6 个 |
| 11 | 内容渲染器 | block 入 v1；inline 推 M1 |
| 12 | 模块系统 | v1 主推 CJS；ESM 完善推 M1 |
| 13 | 调试 | 标准 Node Inspector（启动 host 加 `--inspect`）|

---

## 14. 风险登记 v0.3

| 风险 | 缓解 | 决策节点 |
|---|---|---|
| R1 · Node host RAM 8 扩展 ~500MB | 接受；M1 评估 host 池 | M1 |
| R2 · 首次激活 80-200ms 延迟 | 预热常用扩展 + spinner | Phase 7 末 |
| R3 · Node 26 当前 vs LTS | Phase 0 spike 实测 --allow-net 等新 flag 行为；评估 ship 时 LTS 状态 | Phase 0 末 |
| R4 · 资源耗尽 / 死循环 | Rust 监控 + 限额 + 强杀 | Phase 10 |
| R5 · L1 API 设计错 | Phase 0 freeze IDL + peer review | Phase 0 末 |
| R6 · flow runtime 现有 agent step 耦合 | Phase 0 摸底；Phase 8 重构 | Phase 0 末 |
| R7 · 内置 LLM provider 迁移 | v1 不迁；M1 评估 | 已决 |
| R8 · v1 无 marketplace 审核 | v1 仅本地 .crx；用户群可控 | M1 上 marketplace |

---

## 15. M0 范围（v1 alpha）

### 必须

- L1 Kernel 14 原语
- L1.5 平台事件 8 条入门
- L2 EP 6 个（command / config.schema / config.page / agents.provider / content.renderer / ui.sidebar.view）
- **Node 26 打包**（macOS arm64/x64 + Win x64 + Linux x64）
- **Node host 进程池 + 每扩展独立 host**
- **build_node_flags：manifest → Node Permission flags**（含平台变量展开 + canonicalize）
- **RPC 走 inherited fd 3**（不走 Unix socket / Named Pipe；详见 §6.1.1）
- Webview 沙箱 iframe
- 安装期人话授权 UX（all-or-nothing + post-install 单项撤销）
- `@cronymax/extension` TS SDK + IDL codegen
- `cronymax ext` CLI
- 设置面板 - Extensions 标签
- **`bytedance.coco` 内置 dogfood**
- **flows agent 系统适配 AgentProvider**
- 第二个 dogfood（推荐 `acme.mermaid-renderer`）
- **扩展日志系统**：4 类 log 文件 + `createOutputChannel` SDK + 设置面板"日志" tab + `diagnostic-bundle` CLI + 6 层 EH 错误处理（详见 [`extension-logs.md`](extension-logs.md)）

### 不在 v1（推 M1+）

- OS sandbox 外层（γ）
- Marketplace + 数字签名
- 远程开发模式
- 共享 Node host / 池预热
- Process runtime（非 JS 扩展）
- 正式 L3 服务 registry
- WASM runtime
- 其余 L2 EPs
- inline content renderers
- 内置 LLM provider 迁移
- ESM 主推
- 跨扩展依赖图 UI

---

## 16. 跟 v0.2 的差异速查

| 项 | v0.2 | v0.3 |
|---|---|---|
| Node 版本 | Node 22 LTS | **Node 26** |
| 网络 ACL | "manifest 信息披露不强制" | **Node `--allow-net` boolean**（Node 26.1.0 未落地 host-scoped）；manifest 仅人话授权 |
| child_process 平台 wrap | "command + argsPattern 白名单" | **砍掉，纯 Node `--allow-child-process` boolean** |
| 网络包装层 | "M1 bootstrap 包装 net/tls/http" | **砍掉** |
| 审计 hook | "bootstrap 记 spawn 日志" | **审计走平台 audit.log 结构化**（不在 bootstrap 写）|
| FFI / inspector / addons | 未控 | **Node `--allow-ffi/-inspector/-addons` 默认禁** |
| manifest capabilities | `process: { allow: [...] }` 复杂结构 | **`process: true` boolean**；`fs` 改 `[{path, mode}]` + 平台变量 |
| α 安全实施工程量 | ~5 天 | **~3 天**（Phase 0 评议后含 RPC fd 3 改 + 平台变量展开）|
| IPC | Unix socket / Named Pipe | **Inherited fd 3**（Phase 0 评议改） |
| 扩展日志系统 | 未设计 | **入 v1**（详 `extension-logs.md`）|

## 16.1 Phase 0 评议引入的修订（v0.3 → v0.3-patched）

详见 [`phase-0-review.md`](phase-0-review.md) §6 决议汇总。要点：

| 项 | 改动 |
|---|---|
| §6.1 `build_node_flags` | 永远 emit `--no-warnings`；fs 改平台变量展开 + canonicalize 双填；network 改 boolean |
| §6.1.1 **新增** | RPC 走 inherited fd 3，不走 Unix socket |
| §6.1.2 **新增** | 平台变量集（`{WORKSPACE}` / `{HOME}` / `{EXT_STORAGE}` 等） |
| §6.2 安装期授权 | all-or-nothing + post-install 撤销；网络项标注"v1 不区分主机" |
| §6.4 防绕过表 | 网络一行改"v1 boolean，M1+ host-scoped" |
| §8 manifest schema | `fs` 改 `[{path, mode}]` 数组 + 平台变量 |
| §10 lifecycle | activate spawn 加 stdio fd 3 描述；crash 详转 `extension-logs.md` §7 |
| §13 决策 4c | IPC = inherited fd 3（替 Unix socket）|
| §13 决策 4d | 加 "bootstrap 极少量 hook 不算 require 劫持" 澄清 |
| §15 M0 范围 | 加扩展日志系统 / RPC fd 3 / 平台变量 |
| **新增** §16.1 | 本表 |
| 总 v1 alpha 估时 | 12-14 周 | **10-12 周** |

---

## 17. 附录 · 词汇

- **Kernel API** — `cronymax.*` namespace 暴露的能力 API
- **L2 EP** — Extension Point，平台 UI/逻辑消费的具名贡献槽位
- **AgentProvider** — 实现聊天会话能力的扩展接口
- **AgentSession** — 一次具体对话实例
- **Agent** — `.cronymax/agents/<id>.agent.yaml` 配置的 persona + provider 引用
- **Node host** — 跑扩展代码的 Node 26 子进程；每扩展独立
- **Node Permission Model** — Node 24+ 稳定的 VM 层 capability enforcement，Node 26 完整版含网络/FFI/Inspector
- **α 阶段** — v1：Node Permission 一条龙
- **γ 阶段** — M1：α + OS sandbox 双层

---

文档版本：v0.3 · 2026-05-19
DRI：待定
评审：待
