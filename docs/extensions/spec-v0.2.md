# Cronymax 扩展平台 — 设计文档 v0.2

状态：内部评审 / 待启动开工

替换关系：**本文档替代 spec-v0.1**。v0.1 的"V8 host + require 劫持 + node-compat 兼容层"路线已废弃。

---

## 0. 设计目标与非目标

cronymax 是 **agent workflow 应用**。

### 目标

- 第三方在**不修改 cronymax 核心代码**的前提下，向平台贡献 agent provider / tool / content renderer / view / settings page / theme / channel / fs provider 等
- **完整 Node 生态兼容**：扩展是普通 Node 包，npm 直接可用
- **安全可控**：Node Permission Model 在 VM 层强制 capability；M1 加 OS sandbox 双层防护
- **DX 贴近 VS Code 习惯**：常见路径写法相同，迁移成本低
- **平台可演化**：加新扩展点走 RFC 流程，不破坏已有扩展
- **α→γ 升级路径丝滑**：v1 上 Node Permission，M1 加 OS sandbox 外层，零震动

### 非目标（v1）

- 代码编辑器扩展性（LSP / DAP / grammar / formatter / linter / code lens）
- WASM runtime
- 跨扩展正式契约系统（schema + semver 版本管理）—— 用 VS Code 同款 `extension.exports`
- 远程开发模式（SSH / WSL / dev container）
- Marketplace 数字签名（先靠本地 .crx + sandbox + 授权弹窗）
- OS-level sandbox（M1+）
- 多 Node 进程共享 host（v1 直接独立 host 起步）

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
                           │ CEF browser query / event（已有）
                           ▼
┌──────────────────────────────────────────────────────────────────┐
│ Rust Core (cronymax 主进程)                                       │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ Extension Manager                                           │ │
│  │ manifest / registry / activation / capability gate          │ │
│  │ Node host 进程池（每扩展独立 host）                          │ │
│  └────────────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ L2 Contribution Registry + L1.5 Event Bus                   │ │
│  └────────────────────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │ cronymax/v1 RPC Server                                      │ │
│  │ MessagePack-RPC over Unix socket（Win Named Pipe）          │ │
│  └────────────────────────────────────────────────────────────┘ │
└─────┬──────────────────────┬───────────────────────┬─────────────┘
      │ 每扩展独立 socket    │                       │
      ▼                      ▼                       ▼
┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
│ Node host #1     │  │ Node host #2     │  │ Node host #N     │
│ bytedance.coco   │  │ acme.mermaid     │  │ ...              │
│                  │  │                  │  │                  │
│ node \           │  │ node \           │  │ node \           │
│  --experimental- │  │  --experimental- │  │  --experimental- │
│   permission \   │  │   permission \   │  │   permission \   │
│  --allow-fs-     │  │  --allow-fs-     │  │  ...             │
│   read=ws \      │  │   read=ws \      │  │                  │
│  --allow-fs-     │  │                  │  │                  │
│   write=ws \     │  │                  │  │                  │
│  --allow-child-  │  │                  │  │                  │
│   process \      │  │                  │  │                  │
│  bootstrap.js    │  │  bootstrap.js    │  │  bootstrap.js    │
│                  │  │                  │  │                  │
│ ↓ vm.Context     │  │ ↓ vm.Context     │  │ ↓ vm.Context     │
│ main.ts + deps   │  │ main.ts + deps   │  │ main.ts + deps   │
└────────┬─────────┘  └──────────────────┘  └──────────────────┘
         │ Rust spawn 的子进程
         ▼
   ┌──────────────────┐
   │ coco acp serve   │
   └──────────────────┘
```

**三个进程类型**：
- CEF Renderer（cronymax UI + 扩展 webview iframe）
- Rust Core（capability + 协调 + 进程池）
- Node Host × N（每激活扩展一个）

### 安全模型分层

| 层 | 实现 | 何时上 |
|---|---|---|
| **L0 · 进程隔离** | Node host 是独立 OS 进程，扩展崩不拖死 cronymax | v1 |
| **L1 · 静态契约** | manifest 申报 capabilities；安装期用户授权 | v1 |
| **L2 · VM 强制（Node Permission）** | `--experimental-permission --allow-*` flags 由 Rust 根据 manifest 拼出，Node VM 层拦截 | **v1** |
| **L3 · OS Sandbox（外层）** | sandbox-exec / bubblewrap / Job Object，kernel 级拦截 syscall | **M1** |
| **L4 · Marketplace + 签名 + 行为监控** | 上传扫描、社区报告、吊销机制 | M2+ |

升级路径：α（L0+L1+L2）→ γ（α+L3）。**架构同构，对扩展开发者零震动**。

---

## 2. L1 Kernel — 14 个原语

| 原语 | 实现 | Capability |
|---|---|---|
| `lifecycle` | `activate(ctx)` / `deactivate()` / disposables | n/a（必需）|
| `capabilities` | manifest 静态声明，安装期校验 | n/a |
| `commands` | 注册具名可调用 | 自家 publisher 命名空间内永远允许 |
| `events` | pub/sub topic | `events.subscribe` / `events.emit` 白名单 |
| `config` | get/update + onDidChange | `config.schema` 申报 |
| `secrets` | macOS Keychain / Win DPAPI / Linux secret-service | `secrets.namespace` 锁 publisher 前缀 |
| `storage` | per-extension state KV | n/a |
| `process` | `spawn` 走 Rust（capability 校验 + tokio spawn）| `process.allow` 白名单 |
| `fs` | 真 Node fs，Node Permission Model 强制 scope | `fs.scope` + `fs.mode` |
| `network` | 真 Node fetch / WebSocket，network ACL 由 Node v22+ 强制 | `network.allow` host 白名单 |
| `ui-slots` | 标识能往哪些槽位贡献 | `ui-slots` 列表 |
| `webview` | CEF iframe + postMessage 中转 | n/a |
| `auth` | 内置 OAuth / PKCE / device-flow | `auth.providers` 白名单 |
| `extensions` | getExtension + exports（VS Code 同款） | n/a |

### 关键 API 形状

```ts
// @cronymax/extension SDK
declare namespace cronymax {
  function activate(handler: (ctx: ExtensionContext) => Promise<void>): void;

  const commands: {
    register(id: string, handler: (...args: unknown[]) => unknown): Disposable;
    execute<T>(id: string, ...args: unknown[]): Promise<T>;
  };

  const events: {
    on(topic: string, handler: (payload: unknown) => void): Disposable;
    emit(topic: string, payload: unknown): void;
  };

  const workspace: {
    rootUri: URI | undefined;
    fs: WorkspaceFileSystem;            // 包装 Node fs，path 锁定 workspace
    getConfiguration(section?: string): Configuration;
    onDidChangeConfiguration(handler): Disposable;
  };

  const process: {
    spawn(opts: SpawnOptions): Promise<ChildProcess>;
  };

  const network: {
    fetch: typeof globalThis.fetch;     // Node v22+ 受 --allow-net 强制
    websocket(url: string): WebSocket;
  };

  const secrets: {
    get(key: string): Promise<string | undefined>;
    set(key: string, value: string): Promise<void>;
    delete(key: string): Promise<void>;
  };

  const window: {
    showInformationMessage(text, ...actions): Promise<string | undefined>;
    createWebviewPanel(opts): WebviewPanel;
    openConfigPage(id: string): Promise<void>;
  };

  // L2 EP 注册
  const agents: {
    registerProvider(id: string, impl: AgentProvider): Disposable;
    getProvider(id: string): AgentProvider | undefined;
  };
  const chat: { /* ... */ };
  const renderers: { /* ... */ };

  const extensions: {
    getExtension(id: string): Extension | undefined;
    readonly all: readonly Extension[];
  };

  const env: { appName, platform, homedir, machineId };
}
```

---

## 3. L1.5 平台事件主题

平台命名空间 `cronymax.*` 下的事件，Rust core 在关键时刻 emit。扩展通过 `cronymax.events.on(topic, handler)` 订阅。

订阅 `cronymax.*` 主题**必须**在 manifest `capabilities.events.subscribe` 显式声明。

### v1 入门 8 条

| 主题 | payload | 何时发 |
|---|---|---|
| `cronymax.session.started` | `{ sessionId, providerId, agentId?, model }` | 用户开会话 |
| `cronymax.session.ended` | `{ sessionId, reason }` | 会话关闭 |
| `cronymax.message.user.sent` | `{ sessionId, turnId, text }` | 用户发消息 |
| `cronymax.message.assistant.delta` | `{ sessionId, turnId, textDelta }` | 流式 token |
| `cronymax.message.assistant.done` | `{ sessionId, turnId, fullText, finishReason }` | turn 收尾 |
| `cronymax.tool.invoked` | `{ sessionId, turnId, toolCallId, name, input, source }` | tool 调用 |
| `cronymax.tool.completed` | `{ sessionId, toolCallId, status, output }` | tool 完成 |
| `cronymax.permission.requested` | `{ sessionId, requestId, target, options }` | 需要授权 |

M1 扩展：`cronymax.flow.*` / `cronymax.config.changed` / `cronymax.workspace.folders.changed` / `cronymax.permission.decided` / `cronymax.message.assistant.start` / `cronymax.message.assistant.thinking`。

---

## 4. L2 Extension Points

12 个候选；v1 起步 6 个。命名空间 `cronymax.*` 锁定。

| EP | 谁消费 | M0 |
|---|---|---|
| `cronymax.command` | 命令面板 / 快捷键 | ✅ |
| `cronymax.keybinding` | 快捷键引擎 | M1 |
| `cronymax.menu.item` | 菜单系统 | M1 |
| `cronymax.config.schema` | 设置面板 | ✅ |
| `cronymax.config.page` | 设置面板 webview | ✅ |
| `cronymax.ui.activitybar.item` | 活动栏 | M1 |
| `cronymax.ui.sidebar.view` | 侧栏 | ✅ |
| `cronymax.ui.statusbar.item` | 状态栏 | M1 |
| `cronymax.content.renderer` | 内容渲染管道（block 类）| ✅ |
| **`cronymax.agents.provider`** | 聊天面板 + flow runtime | ✅ |
| `cronymax.chat.tool` | 工具调度器 | M1 |
| `cronymax.workspace.fs-provider` | 工作区 fs | M1 |

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
  | { kind: "toolCall"; id: string; name: string; input: any; source: string; status: "in_progress" }
  | { kind: "toolCallUpdate"; id: string; status: "completed" | "failed"; output: any }
  | { kind: "permissionRequest"; requestId: string; tool: string; options: any }
  | { kind: "done"; stopReason: "end_turn" | "max_tokens" | "tool_calls" | "cancelled" | "error" };
```

**平台消费方**：
- 聊天面板（`web/src/panels/chat/`）
- flow runtime（`crates/cronymax/src/flow/`）

两个消费方共享同一份 provider 注册数据。

---

## 5. Node Host 进程模型

### 关键决策：每扩展独立 Node host

| 维度 | 选择 |
|---|---|
| Node 版本 | **Node 22 LTS**（含 stable permission model）|
| Node 来源 | **打包进 cronymax**（不要求用户预装）|
| 隔离粒度 | **每扩展独立 OS 进程** |
| 启动模型 | **Lazy activate**（被需要时才起）|
| 回收模型 | **不主动 deactivate**（cronymax 退出才退）|
| IPC | **Unix socket（macOS/Linux）/ Named Pipe（Windows）+ MessagePack-RPC** |
| 模块系统 | **CJS 主推**（与 VS Code 对齐）；ESM 兼容 M1 完善 |

### 资源占用预期

| 单 Node host | 数 |
|---|---|
| 基础 RSS | ~50-80MB |
| 启动时间 | 80-200ms（首次激活）|
| 跨扩展调用 | 50-100μs/call（仅 `extension.exports`，频率低）|

| 用户场景 | 总开销 |
|---|---|
| 闲置 cronymax | 0 |
| 跟 Coco 聊天 | ~60MB |
| Coco + 2 source + 1 渲染器 | ~250MB |
| 重度用户 8 扩展 | ~500MB |

VS Code 同等场景 EH ~500-1000MB（一进程含全部扩展），cronymax 独立 host 模型实际内存近似且**隔离更强**。

### 启动延迟缓解

1. **预热常用扩展**：cronymax 启动时按"上次会话用过"列表静默激活
2. **激活时给 spinner**：200ms 内可接受
3. **不主动 deactivate**：避免反复冷启
4. **未来 M1**：host 池预热 / `--snapshot-blob` 快照启动

---

## 6. 安全模型 · α 阶段（v1）

### 6.1 Node Permission Model 实现

Rust core 起 Node host 时根据 manifest 拼 flags：

```rust
// crates/cronymax/src/extensions/host/node.rs
fn build_node_flags(manifest: &Manifest, workspace: &Path, ext_dir: &Path) -> Vec<String> {
    let mut flags = vec!["--experimental-permission".to_string()];

    // fs
    if let Some(fs_cap) = &manifest.capabilities.fs {
        if fs_cap.scope == "workspace" {
            flags.push(format!("--allow-fs-read={}", workspace.display()));
            if fs_cap.mode == "rw" {
                flags.push(format!("--allow-fs-write={}", workspace.display()));
            }
        }
        // 扩展私有 storage dir 永远允许
        flags.push(format!("--allow-fs-read={}", ext_dir.display()));
        flags.push(format!("--allow-fs-write={}", ext_dir.display()));
    }

    // process
    if manifest.capabilities.process.is_some() {
        flags.push("--allow-child-process".to_string());
        // command 白名单在平台 RPC 层校验，Node 只管"能不能 spawn"
    }

    // worker
    if manifest.capabilities.workers.is_some() {
        flags.push("--allow-worker".to_string());
    }

    // network (Node 22+)
    if let Some(net_cap) = &manifest.capabilities.network {
        for host in &net_cap.allow {
            flags.push(format!("--allow-net={}", host));
        }
    }

    flags
}
```

扩展看到的就是**真 Node**，全 npm 生态可用。但越界 fs / process / network 直接 `ERR_ACCESS_DENIED`。

### 6.2 兜底的命令白名单（平台 RPC 层）

Node 的 `--allow-child-process` 是 boolean，无法说"只允许 git 不允许 curl"。所以 process spawn 在 SDK 层多一道：

```ts
// @cronymax/extension 内部
function spawn(opts) {
  // 1. 先送 RPC 给 Rust，Rust 按 manifest.capabilities.process.allow 校验 command + args
  // 2. 校验过了，Rust 用 tokio::process spawn，返回 ChildProcess proxy
  // 3. 扩展拿 proxy；stdin/stdout/stderr 经 RPC 流回
}
```

直接 `require('child_process')` 怎么办？也劫持一份转给 Rust：
```ts
// bootstrap.js（vm.Context 注入前）
function makeChildProcessShim(extId) {
  return {
    spawn: (cmd, args, opts) => rpcSpawn({extId, cmd, args, opts}),
    exec: () => { throw new Error('exec not supported; use spawn'); },
    execSync: () => { throw new Error('sync not allowed'); },
    fork: () => { throw new Error('fork not allowed'); },
  };
}
```

注意：这**不是**之前讨论的"完整 require 劫持"。只针对 `child_process`，且不做 fs/net 包装（那些由 Node Permission Model 处理）。

### 6.3 安装期人话授权

manifest capabilities 译成清单：

```
┌───────────────────────────────────────────────────────────┐
│ 安装 Coco                                                  │
│ by bytedance · v0.1.0                                      │
│                                                            │
│ 此扩展将能够：                                              │
│  📁 读写工作区文件                                          │
│  ⚙ 启动进程：${coco.binaryPath} acp serve ...               │
│  🌐 访问网络主机：api.openai.com                            │
│  🔑 存储 bytedance.coco.* 命名空间下的密钥                 │
│  💬 注册为聊天 agent provider                              │
│                                                            │
│ [详情]                          [取消]  [安装]              │
└───────────────────────────────────────────────────────────┘
```

用户可在"设置 → 扩展 → <ext> → 权限"随时撤销单项。

### 6.4 命名空间锁定

| 命名空间 | 谁拥有 | 第三方写 |
|---|---|---|
| `cronymax.*` | 平台 | **静态校验拒绝** |
| `<publisher>.*` | 该 publisher | 仅自家扩展 |

冒充 `cronymax.secrets` 之类的攻击安装期就死。

### 6.5 诚实的局限

Node Permission Model（v22）已经覆盖：
- fs read / write 路径白名单
- child_process 调用门控
- worker_threads 门控
- network host 白名单（v22+ 完整）

**绕过路径已经被 Node 拦死**：
- ❌ `eval("require('fs').readFileSync('/etc/passwd')")` → `ERR_ACCESS_DENIED`
- ❌ `process.binding('fs').open(...)` → `ERR_ACCESS_DENIED`
- ❌ `Function('return require')()('fs')` → 被拦
- ❌ 通过 Object.prototype 污染拿到 fs → 拦

**仍可能的攻击**（α 拦不住，γ 才拦得住）：
- ⚠️ 资源耗尽（无限循环、内存爆掉）—— α 靠进程隔离 + Rust 限额
- ⚠️ 时间侧信道（精确测时间推断系统状态）—— 几乎无法防
- ⚠️ 滥用授权范围内的能力（network.allow 给了 evil.com，扩展往那送数据）—— 这是用户授权的责任

---

## 7. 安全模型 · γ 阶段（M1）

外层加 OS sandbox。架构对 v1 无侵入：

```
Rust core
  ↓ spawn:
sandbox-exec -f /tmp/<ext-id>.sb -- \
  node --experimental-permission --allow-fs-read=... bootstrap.js
                                                              ↑
                                                       v1 已有的 Node host

Rust 根据 manifest capabilities 生成两份等价规则：
  1. Node --allow-* flags（v1 已做）
  2. OS sandbox profile（M1 新做）
两套规则镜像同一份 capability，OS 层兜底。
```

### 各平台 profile 生成

| 平台 | 实现 | 工程量 |
|---|---|---|
| macOS | SBPL 模板 + 参数替换 | ~1 周 |
| Linux | bubblewrap + seccomp BPF | ~1 周 |
| Windows | Job Object + AppContainer + Restricted Token | ~2 周 |
| 共用：profile 生成 + 测试 | ~1 周 |
| 小计 | **4-5 周** |

### α→γ 升级对扩展的影响

| 项 | 影响 |
|---|---|
| 扩展代码 | 0 改动 |
| manifest schema | 0 改动 |
| SDK | 0 改动 |
| 行为差异 | α 时能用的 `/tmp` 等"灰区"在 γ 时可能拒。文档已警告扩展只用 SDK 暴露的资源 |

---

## 8. 扩展清单 schema

```jsonc
{
  // 标识（必需）
  "id":        "publisher.name",     // 必须 publisher 前缀
  "name":      "Display Name",
  "version":   "1.0.0",
  "publisher": "publisher",
  "engines":   { "cronymax": "^1.0" },
  "main":      "./dist/main.js",     // 可省（纯声明式扩展）

  // 描述
  "description": "...",
  "icon":      "./icon.png",
  "repository": "...",
  "license":   "MIT",

  // 激活事件
  "activationEvents": [
    "onAgentProvider:coco",
    "onCommand:coco.openSettings",
    "onView:coco.history"
  ],

  // L2 EP 贡献
  "contributes": {
    "cronymax.agents.provider": [...],
    "cronymax.command": [...],
    "cronymax.config.schema": {...},
    // ...
  },

  // 能力声明（capability gate）
  "capabilities": {
    "fs":                { "scope": "workspace", "mode": "rw" },
    "process":           { "allow": [{ "command": "...", "argsPattern": [...] }] },
    "network":           { "allow": ["api.foo.com", "*.bar.com"] },
    "secrets":           { "namespace": "publisher.name.*" },
    "events.subscribe":  ["cronymax.message.assistant.done"],
    "events.emit":       ["publisher.name.*"],
    "ui-slots":          ["sidebar", "settings"],
    "extension-points":  ["cronymax.agents.provider", "cronymax.command"],
    "auth.providers":    ["oauth-generic"],
    "workers":           false                  // 默认禁
  },

  // 显式跨扩展依赖
  "extensionDependencies": ["other.extension"]
}
```

### 校验规则

- `id` 必须 `<publisher>.<name>` 格式，`<publisher>` 必须等于 `publisher` 字段
- `contributes` 的 key 必须以 `cronymax.` 开头且在 `capabilities.extension-points` 申报
- 任何往 `cronymax.*` 命名空间的写入一律拒
- `activationEvents` `*` 通配在安装期警告

---

## 9. Webview 通信链路

扩展 webview iframe 跟扩展 main.ts **跨进程**（webview 在 CEF Renderer，main.ts 在 Node host）。链路：

```
扩展 webview iframe (CEF Renderer)
   │ panel.postMessage({type: "saveToken", token})
   ▼
CEF 拦 postMessage → cronymax-webview://<ext-id>/<panel-id>
   │ 通过现有 browser query 上报 Rust
   ▼
Rust core
   │ 1. 鉴权（iframe 属于哪个扩展 / panel）
   │ 2. 转给对应 Node host：
   │    rpc.notify("webview/postMessage", {panelId, message})
   ▼
Node host（该扩展独占）
   │ 路由到 vm.Context 里 panel.onDidReceiveMessage handler
   ▼
扩展 main.ts:
   panel.onDidReceiveMessage(msg => {...});
```

反向同理。**4 跳 IPC**，对配置页这种低频交互无所谓。

---

## 10. 扩展生命周期

```
未安装 ──install──► 已安装/未启用 ──enable──► 已启用/未激活
                                                      │
                                                      │ activationEvent
                                                      ▼
                                              已激活/运行中
                                                      │
                                                      │ deactivate
                                                      ▼
                                              已启用/未激活
                                                      │
                                                      │ disable
                                                      ▼
                                              已安装/未启用
                                                      │
                                                      │ uninstall
                                                      ▼
                                                  未安装
```

- **install**：解压 .crx 到 `~/.cronymax/extensions/<id>/`，校验 manifest，弹授权
- **enable / disable**：用户控制；扩展状态持久化到 registry.json
- **activate**：
  1. Rust 检查 host 池：该扩展是否已有 host？没有则 spawn 新 Node host
  2. spawn 命令：`node --experimental-permission --allow-* ... bootstrap.js --ext-id=<id>`
  3. bootstrap.js 接 socket、握手、调 `extension/activate` RPC
  4. Node 加载 main.js（在 vm.Context 里），调 `activate(ctx)`
  5. 扩展 register*** 调用通过 RPC 上报 Rust 注册中心
- **deactivate**：
  - 用户 disable / uninstall / 撤回 capability：Rust 通知 Node 调 `deactivate()`，dispose subscriptions，然后 SIGTERM 进程
  - 平台主动不 deactivate（idle 不回收）
- **crash**：
  - Node host SIGKILL / OOM
  - Rust 检测到 socket 断 / exit code 异常
  - 自动重启 N 次（默认 3）
  - 仍崩 → 退而禁用，UI 提示用户

---

## 11. 端到端走查：coco-acp

### 项目结构

```
coco-extension/
├── package.json              ← 正经 Node 包
├── cronymax-extension.json
├── tsconfig.json
├── src/
│   ├── main.ts               ← activate / deactivate
│   ├── acp-client.ts         ← 直接用 child_process（被门控）
│   ├── coco-session.ts
│   └── settings/
│       ├── index.html
│       └── index.ts
└── dist/                     ← esbuild 打包产物（CJS）
    ├── main.js
    └── settings/...
```

### package.json

```json
{
  "name": "@bytedance/coco-cronymax",
  "version": "0.1.0",
  "main": "dist/main.js",
  "dependencies": {
    "@cronymax/extension": "^1.0.0"
  },
  "devDependencies": {
    "esbuild": "^0.20.0",
    "typescript": "^5.4.0"
  },
  "scripts": {
    "build": "esbuild src/main.ts --bundle --platform=node --target=node22 --format=cjs --external:@cronymax/extension --outfile=dist/main.js"
  }
}
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
        "coco.binaryPath":  { "type": "string", "default": "coco" },
        "coco.defaultModel":{ "type": "string", "default": "GPT-5.4" },
        "coco.defaultMode": { "type": "string", "enum": ["default","plan","bypass_permissions"], "default": "default" },
        "coco.yolo":        { "type": "boolean", "default": false }
      }
    },
    "cronymax.config.page": [{
      "id": "coco.advanced",
      "title": "Coco · Advanced",
      "entry": "./dist/settings/index.html"
    }]
  },

  "capabilities": {
    "process": {
      "allow": [{
        "command": "${config:coco.binaryPath}",
        "argsPattern": ["^acp$", "^serve$", ".*"]
      }]
    },
    "fs":      { "scope": "workspace", "mode": "rw" },
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

      async listModels(): Promise<cronymax.agents.ModelInfo[]> {
        const client = await AcpClient.spawnEphemeral();
        try {
          await client.initialize();
          const probe = await client.newSession({
            cwd: cronymax.workspace.rootUri?.fsPath ?? process.cwd(),
            mcpServers: []
          });
          return probe.models.availableModels.map(m => ({
            id: m.modelId,
            label: m.name,
            description: m.description
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

      async createSession(opts: cronymax.agents.SessionOptions) {
        const cfg = cronymax.workspace.getConfiguration("coco");
        const client = await AcpClient.spawn({
          binaryPath: cfg.get<string>("binaryPath"),
          yolo: cfg.get<boolean>("yolo")
        });
        await client.initialize();
        const acp = await client.newSession({
          cwd: opts.cwd,
          mcpServers: opts.mcpServers ?? []
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

export async function deactivate() { /* ctx.subscriptions auto-cleaned */ }
```

### src/acp-client.ts

```ts
import { spawn, ChildProcess } from "child_process";   // 经平台门控
import { createInterface } from "readline";

export class AcpClient {
  static async spawn(opts: { binaryPath?: string; yolo?: boolean }) {
    const args = ["acp", "serve"];
    if (opts.yolo) args.push("--yolo");
    const proc = spawn(opts.binaryPath ?? "coco", args, { stdio: ["pipe","pipe","pipe"] });
    return new AcpClient(proc);
  }

  // ... JSON-RPC framing + 流式响应处理（已验过的 Python POC 端口）
}
```

`spawn("coco", [...])` 实际：
1. SDK 拦截 child_process.spawn
2. RPC 送 Rust：`{extensionId: "bytedance.coco", command: "coco", args: ["acp","serve"]}`
3. Rust 按 manifest.capabilities.process.allow 校验
4. tokio::process spawn 真子进程
5. 返回 ChildProcess proxy（stdin/stdout 经 RPC 流回 Node）

扩展代码看着就是 Node 写法，**底下被门控**，扩展感知不到。

### 安装 → 激活 → 使用流程

```
1. cronymax ext install bytedance.coco-0.1.0.crx
   → 平台解压、校验、弹授权窗
   → 用户 Allow → 写入 registry.json

2. 用户开聊天面板
   → 平台扫已装扩展 manifest，agent picker 列出 "Coco"（未激活，状态灰）

3. 用户点选 "Coco"
   → 触发 onAgentProvider:coco
   → Rust：spawn Node host #1 with --experimental-permission --allow-fs-read=ws --allow-fs-write=ws --allow-child-process
   → Node host 接 socket, 握手, 加载 bytedance.coco/dist/main.js
   → activate() 调用：cronymax.agents.registerProvider("coco", ...)
   → Rust 收到 RPC，记入 contribution registry
   → 聊天面板 UI 更新："Coco" 选项 ready

4. 用户配置 Coco / GPT-5.4 / plan，发消息
   → 聊天面板：provider.createSession({...})
   → Rust 转给 Node host: rpc.call("agents/createSession", ...)
   → main.ts createSession()：spawn("coco","acp","serve") 经 Rust 校验通过
   → ACP initialize / session/new
   → 返回 CocoSession

5. session.prompt(msg)：
   → 经 RPC 流式回事件给聊天面板
   → 聊天面板按 event.kind 渲染
```

### flows 里用 Coco

`.cronymax/agents/code-reviewer.agent.yaml`：

```yaml
id: code-reviewer
name: Code Reviewer
provider: coco                       # ← 引用 cronymax.agents.provider 的 id
provider_config:
  model: GPT-5.4
  mode: plan
system_prompt: |
  You are a senior code reviewer...
allowed_tools:
  - cronymax.tool.shell
  - cronymax.tool.fs.read
```

flow 走到 `type: agent, agent: code-reviewer` 时，flow runtime 经同一份 agent provider registry 拿到 Coco provider，调 createSession 跑。**与聊天面板路径等价**。

---

## 12. 平台代码改动清单

### `crates/cronymax/src/extensions/`（新增）

- `manifest.rs` — schema 解析与校验
- `registry.rs` — 元数据扫描 + 状态持久化
- `activation.rs` — activationEvents 匹配
- `host/`
  - `node.rs` — Node 进程池 + spawn + 监控
  - `bootstrap/` — Node 侧 bootstrap.js + SDK 注入
- `capability.rs` — 能力门控
- `contributions/` — L2 EP 注册中心（通用，每 EP 一行注册声明）
- `events.rs` — L1.5 平台事件总线 + 订阅路由
- `rpc/` — MessagePack-RPC 服务端
- `api/` — cronymax/v1 在 Rust 侧的实现
  - `commands.rs` / `events.rs` / `fs.rs` / `process.rs` / `network.rs` / `secrets.rs` / `webview.rs` / `agents.rs` / `window.rs` / `workspace.rs` / `extensions.rs`

### `web/src/panels/chat/`（修改）

- 替换硬编码 LLM provider 列表为 `registry.consume("cronymax.agents.provider")`
- session 创建/路由统一走 AgentProvider 接口
- 事件流转翻译统一格式

### `web/src/panels/flows/agents/`（新增）

- AgentList.tsx / NewAgentWizard.tsx (4 步) / AgentEditor.tsx
- 与 AgentProvider registry 集成（与 chat 面板同源数据）

### `crates/cronymax/src/flow/`（修改）

- agent step：通过 AgentProvider registry 调度（不硬编码 native）
- 加载 `.cronymax/agents/<id>.agent.yaml` 解析 provider + provider_config
- allowed_tools 透传 + tool call 门控

### `crates/cronymax/src/capability/agent_loader.rs`（修改）

- AgentDef 加 provider + provider_config 字段
- 校验 provider id 在 registry 存在

### `web/src/panels/settings/extensions/`（新增）

- 扩展列表 / 安装 / 卸载 / 启用 / 禁用 / 配置 / 撤销 capability

### `web/src/shells/extension.ts`（新增）

- 暴露 cronymax/v1 客户端给 webview / 主 UI（CEF 侧）

### `web/src/extensions/sdk/`（新增）`@cronymax/extension`

- TypeScript SDK，提供 cronymax/v1 类型
- 从 Rust IDL codegen 类型声明
- npm 包发布（内网 npm 或 GitHub Packages）

### Node 二进制打包

- macOS arm64 / x64 / Linux x64 / Windows x64 都打包进 build
- Node 22 LTS
- 总增量 ~50-75MB per platform

---

## 13. 决策表 v0.2 (最终)

| # | 决策 | 终值 |
|---|---|---|
| 1 | 架构层数 | L1 Kernel + L1.5 Events + L2 EPs |
| 2 | 命名 | AgentProvider / AgentSession |
| 3 | 命名空间锁 | `cronymax.*` reserved；publisher 前缀强制 |
| 4 | 主要 runtime | **Node.js 22 LTS subprocess** |
| 4a | Node 来源 | 打包进 cronymax |
| 4b | 进程模型 | **每扩展独立 host** + lazy activate + 不主动 deactivate |
| 4c | IPC | Unix socket / Named Pipe + MessagePack-RPC |
| 4d | Capability 实施 α | **Node Permission Model**（`--experimental-permission --allow-*`）+ child_process 平台门控 |
| 4e | Capability 实施 γ | M1 加 OS sandbox（sandbox-exec / bubblewrap / Job Object）外层 |
| 5 | API 真相之源 | TS interface IDL → codegen .d.ts |
| 6 | RPC 编码 | MessagePack-RPC |
| 7 | 清单文件名 | `cronymax-extension.json` |
| 8 | 跨扩展通信 | VS Code 同款 `extension.exports`（无 schema/semver）|
| 9 | UI 沙箱 | CEF iframe 独立 origin；跨进程经 Rust 中转 |
| 10 | v1 L2 EP 数 | 6 个 |
| 11 | 内容渲染器 | block 入 v1；inline 推 M1 |
| 12 | 模块系统 | v1 主推 CJS；ESM 完善推 M1 |
| 13 | 调试 | 标准 Node Inspector，启动 host 加 `--inspect` |

---

## 14. 风险登记 v0.2

| 风险 | 影响 | 缓解 | 决策节点 |
|---|---|---|---|
| R1 · 单 Node host RAM 占用 | 重度用户 8 扩展 ~500MB | 接受；M1 上 host 池 / 共享内存 | M1 评估 |
| R2 · 首次激活 80-200ms 延迟 | UI 卡顿感 | 预热常用扩展 + spinner | Phase 7 末测 |
| R3 · Node Permission Model 实验性 | API 变化 | 锁 Node 22 LTS；监控 Node 23/24 变化 | Node 主版本节点 |
| R4 · Node Permission 不防资源耗尽 | 恶意扩展耗 CPU/RAM | Rust 监控 + 限额 + 强杀 | Phase 10 |
| R5 · child_process 平台门控有漏 | 扩展逃出白名单 | argsPattern 严格 + 审计 + M1 OS sandbox | M1 |
| R6 · 内置 LLM provider 迁移 | 工程量爆 | v1 不迁；老的留 core；新 agent 走 AgentProvider | 已决 |
| R7 · flow runtime 重构面 | 现有 agent step 改造 | Phase 0 摸清楚现状，Phase 8 重构 | Phase 0 末 |
| R8 · v1 没 marketplace 审核 | 恶意扩展上传无门槛 | v1 只支持本地 .crx；MVP 用户群可控 | M1 上 marketplace |

---

## 15. M0 范围（v1 alpha）

### 必须

- L1 Kernel 14 原语
- L1.5 平台事件 8 条入门
- L2 EP 6 个（command / config.schema / config.page / agents.provider / content.renderer / ui.sidebar.view）
- **Node 22 LTS 打包**（macOS arm64/x64 + Win x64 + Linux x64）
- **Node host 进程池 + 每扩展独立 host**
- **Node Permission Model 应用**（capabilities → --allow-* flags 生成）
- **child_process 平台门控**（command + args 白名单）
- Webview 沙箱 iframe
- 安装期人话授权 UX
- `@cronymax/extension` TS SDK + IDL codegen
- `cronymax ext` CLI
- 设置面板 - Extensions 标签
- **`bytedance.coco` 内置 dogfood 扩展**
- **flows agent 系统适配 AgentProvider**
- 第二个 dogfood 扩展（验通用性，推荐 `acme.mermaid-renderer`）

### 不在 v1（推 M1+）

- OS sandbox 外层（γ 阶段）
- Marketplace + 数字签名
- 远程开发模式
- 共享 Node host / host 池预热
- Process runtime（非 JS 扩展）
- 正式 L3 服务 registry（schema + semver）
- WASM runtime
- 其余 L2 EPs（keybinding / menu.item / activitybar / statusbar / fs-provider / chat.tool / auth.provider）
- inline content renderers
- 内置 LLM provider 迁移到扩展模型
- ESM 主推
- 跨扩展依赖图 UI
- host 池预热 / `--snapshot-blob` 启动优化

---

## 16. 跟 v0.1 的对照（差异速查）

| 项 | v0.1 | v0.2 |
|---|---|---|
| Runtime | CEF V8 host（含 require 劫持）| Node 22 subprocess（每扩展独立）|
| Capability 实施 | require 劫持 + 灰白黑名单 | Node Permission Model + child_process 平台门控 |
| Node 兼容层 | @cronymax/node-compat shim | 不需要（真 Node）|
| 进程模型 | 单 helper subtype 共享 | 每扩展独立进程 |
| 安全升级路径 | 没明确 | α(v1) → γ(M1) 双层叠加 |
| Webview 通信 | V8 同进程 postMessage | 跨进程经 Rust 中转 |
| npm 生态 | ~85% | ~100% |

---

## 17. 附录 · 词汇

- **Kernel API** — `cronymax.*` namespace 暴露的能力 API
- **L2 EP** — Extension Point，平台 UI/逻辑消费的具名贡献槽位
- **AgentProvider** — 实现聊天会话能力的扩展接口（包括 listModels / createSession / modes）
- **AgentSession** — 一次具体的对话实例
- **Agent** — `.cronymax/agents/<id>.agent.yaml` 配置的"persona + provider 引用 + 工具白名单"
- **Node host** — 跑扩展代码的 Node.js 子进程；每扩展独立
- **Node Permission Model** — Node 20+ 实验性 / 22 LTS 稳定的 VM 层 capability enforcement
- **α 阶段** — v1：Node Permission 单层
- **γ 阶段** — M1：α + OS sandbox 双层

---

文档版本：v0.2 · 2026-05-19
DRI：待定
评审：待
