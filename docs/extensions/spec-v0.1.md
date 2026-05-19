# Cronymax 扩展平台 — 设计文档 v0.1（M0 草案）

状态：内部评审中 / 未实现

## 目录

0. [设计目标与非目标](#0-设计目标与非目标)
1. [总体架构（三层）](#1-总体架构三层)
2. [L1 Kernel — 14 个原语](#2-l1-kernel--14-个原语)
3. [L1.5 平台事件主题](#3-l15-平台事件主题)
4. [L2 Extension Points](#4-l2-extension-points)
5. [扩展清单 schema](#5-扩展清单-schema)
6. [安全模型](#6-安全模型)
7. [扩展生命周期与 Runtime](#7-扩展生命周期与-runtime)
8. [端到端走查：coco-acp](#8-端到端走查coco-acp)
9. [平台代码改动清单](#9-平台代码改动清单)
10. [M0 范围与节奏](#10-m0-范围与节奏)

---

## 0. 设计目标与非目标

cronymax 是 **agent workflow 应用**，不是代码编辑器。

### 目标

- 第三方在**不修改 cronymax 核心代码**的前提下，向平台贡献 agent provider / tool / content renderer / view / settings page / theme / channel / fs provider 等
- **安全可控**：命名空间锁定 + capability 白名单 + 子进程沙箱
- **DX 贴近 VS Code 习惯**：常见路径熟悉，写第一个扩展不超过半天
- **平台可演化**：加新扩展点走 RFC 流程，不破坏已有扩展

### 非目标（v1）

- 代码编辑器扩展性（LSP / DAP / grammar / formatter / linter / code lens / custom code editor）
- WASM runtime
- 跨扩展正式契约系统（schema + semver 版本管理）—— 暂用 VS Code 同款 `extension.exports` 替代
- 远程开发模式（SSH / WSL / dev container）
- 数字签名 / marketplace 上架（先靠本地 .crx + sandbox + 授权弹窗）

### 设计原则

平台开放性的定义：**在平台代码不变、不发新版的前提下，第三方能完成多少种连接。**

任何让平台必须"知道某种新插件类型存在"的设计，都违反这条原则。但**平台 UI 显然要消费它的扩展点**（聊天面板要知道 agent 是什么）—— 这种"平台 UI 参与"是不可避免的，且明确划在 L2 EP 这一层做，不掩盖。

---

## 1. 总体架构（三层）

```
┌──────────────────────────────────────────────────────────────┐
│ L2 Extension Points —— 12 个具名契约                          │
│ 平台 UI / 核心逻辑消费扩展贡献的"显示在平台某处的能力"        │
│ 命名空间 cronymax.* 保留，capability 强制                     │
│ 加新 EP 走 RFC 流程                                          │
└──────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────┴────────────────────────────────┐
│ L1.5 平台事件主题 —— ~15 个                                   │
│ 平台关键时刻（chat 消息、tool 调用、flow 步骤）发的事件        │
│ 扩展通过 L1.events 订阅，做观察/统计/审计/middleware         │
│ 加新主题走 RFC 流程                                          │
└──────────────────────────────────────────────────────────────┘
                              ▲
┌─────────────────────────────┴────────────────────────────────┐
│ L1 Kernel —— 14 个原语                                       │
│ 所有扩展能用的最底层 API，每个原语 capability 门控            │
│ 含简易跨扩展通信 extensions.getExtension(id).exports          │
└──────────────────────────────────────────────────────────────┘
```

| 维度 | L1 Kernel | L1.5 事件 | L2 EP |
|---|---|---|---|
| 谁定义 | 平台（封闭 14 条） | 平台（可增长，走 RFC） | 平台（可增长，走 RFC） |
| 命名空间 | n/a | `cronymax.*` 锁定 | `cronymax.*` 锁定 |
| 扩展行为 | 调用 API | 订阅 / 发布 | 静态声明 + runtime 注册 |
| 平台行为 | 实现 API + 门控 | 平台代码 emit | 平台 UI/逻辑消费注册 |
| 加新一项 | 平台改代码发版 | 平台改代码 + 文档 | 平台改代码 + 文档 |

---

## 2. L1 Kernel — 14 个原语

每条 API 都受 capability 门控。manifest 不申报对应 capability，则该 API 不可调用（运行时 throw + 安装期 UI 提醒）。

### 2.1 lifecycle

扩展入口契约：

```ts
export function activate(ctx: cronymax.ExtensionContext): void | Promise<void>;
export function deactivate(): void | Promise<void>;
```

`ctx.subscriptions: Disposable[]` —— 反激活时自动 dispose。

### 2.2 capabilities

manifest 静态声明，平台运行时校验。详见 [安全模型](#6-安全模型)。

### 2.3 commands

```ts
cronymax.commands.register(id: string, handler: (...args) => any): Disposable
cronymax.commands.execute<T>(id: string, ...args): Promise<T>
cronymax.commands.getAll(): string[]
```

注册自家 publisher 命名空间下的 id 永远允许；执行他人 id 受被注册方约定。

### 2.4 events

```ts
cronymax.events.on(topic: string, handler: (payload) => void): Disposable
cronymax.events.emit(topic: string, payload: any): void
cronymax.events.listSubscribable(): string[]
```

Capability:
```jsonc
"events.subscribe": ["cronymax.message.assistant.done", "acme.scoring.*"]
"events.emit":      ["bytedance.coco.*"]   // 自家命名空间永远允许；cronymax.* 永禁
```

### 2.5 config

```ts
cronymax.workspace.getConfiguration(section?: string): Configuration
cronymax.workspace.onDidChangeConfiguration(handler): Disposable
```

Configuration 提供 `get<T>(key)` / `update(key, value, target)`。

Manifest `contributes.cronymax.config.schema` 声明结构。

### 2.6 secrets

```ts
cronymax.secrets.get(key: string): Promise<string | undefined>
cronymax.secrets.set(key: string, value: string): Promise<void>
cronymax.secrets.delete(key: string): Promise<void>
cronymax.secrets.onDidChange(handler): Disposable
```

Capability `secrets.namespace: "publisher.name.*"`：只能读写自家命名空间。

### 2.7 storage

```ts
ctx.globalState.get<T>(key, default?): T
ctx.globalState.update(key, value): Promise<void>
ctx.workspaceState.{get,update}(key, value)
```

明文 KV，per-extension 隔离。敏感数据用 secrets。

### 2.8 process

```ts
cronymax.process.spawn(opts: SpawnOptions): ChildProcess
ChildProcess: { stdin, stdout, stderr, kill, signal, exitCode, ... }
```

Capability:
```jsonc
"process": {
  "allow": [
    { "command": "${config:coco.binaryPath}", "argsPattern": ["^acp$", "^serve$", ".*"] }
  ]
}
```

每次 spawn 都校验白名单。

### 2.9 fs

```ts
cronymax.workspace.fs.readFile(uri): Promise<Uint8Array>
cronymax.workspace.fs.writeFile(uri, data): Promise<void>
cronymax.workspace.fs.stat(uri): Promise<FileStat>
cronymax.workspace.fs.readDirectory(uri): Promise<[name, type][]>
cronymax.workspace.fs.createDirectory(uri): Promise<void>
cronymax.workspace.fs.delete(uri, options?): Promise<void>
cronymax.workspace.fs.rename(src, dst, options?): Promise<void>
cronymax.workspace.fs.watch(uri, options): Watcher
```

Capability:
```jsonc
"fs": { "scope": "workspace" | "none", "mode": "ro" | "rw" }
```

`scope: workspace` 锁定工作区根；不允许逃逸。

### 2.10 network

```ts
cronymax.network.fetch(url, options?): Promise<Response>     // 浏览器 fetch 兼容
cronymax.network.websocket(url, protocols?): WebSocket
```

Capability:
```jsonc
"network": { "allow": ["api.foo.com", "*.bar.com"] }
```

host 白名单（支持通配）。每次请求校验。

### 2.11 ui-slots

不是直接 API；标识扩展能往哪些 UI 槽位贡献。具体贡献通过 L2 EP（如 `cronymax.ui.sidebar.view`）声明。

```jsonc
"ui-slots": ["activitybar", "sidebar", "statusbar", "panel", "modal", "inline", "settings"]
```

### 2.12 webview

```ts
cronymax.window.createWebviewPanel(opts): WebviewPanel
WebviewPanel: { webview: { html, postMessage, onDidReceiveMessage }, onDidDispose, ... }
```

iframe sandbox + 独立 origin `cronymax-webview://<ext-id>/<resource>`。
postMessage 桥由平台中转，可审计。

### 2.13 auth

```ts
cronymax.authentication.getSession(
  providerId: string,         // 内置 "oauth-generic" / "pkce" / "device-flow"，或扩展贡献的 provider
  scopes: string[],
  options?: { createIfNone, forceNewSession }
): Promise<AuthSession>
```

内置三种通用流；扩展通过 `cronymax.auth.provider` EP 贡献新的认证方式。

Capability `auth.providers: string[]`。

### 2.14 extensions

```ts
cronymax.extensions.getExtension(id): Extension | undefined
cronymax.extensions.all: readonly Extension[]
Extension: { id, manifest, isActive, exports, activate(): Promise }
```

简易跨扩展 API（VS Code 同款）：扩展 `activate()` 的返回值放到 `extension.exports`。**无 schema、无 semver、无安装期校验**。`extensionDependencies` 申报后能强保证依赖顺序激活。

正式版（带 schema + semver）推 v2。

---

## 3. L1.5 平台事件主题

平台命名空间 `cronymax.*` 下的事件，由 cronymax 核心代码在关键时刻发布。扩展通过 `cronymax.events.on` 订阅。

订阅 `cronymax.*` 主题**必须**在 manifest `capabilities.events.subscribe` 显式声明，否则运行时 throw。

### Session / Chat
| 主题 | payload | 何时发 |
|---|---|---|
| `cronymax.session.started` | `{ sessionId, providerId, agentId?, model }` | 用户开会话 |
| `cronymax.session.ended` | `{ sessionId, reason }` | 会话关闭 |
| `cronymax.message.user.sent` | `{ sessionId, turnId, text }` | 用户发消息 |
| `cronymax.message.assistant.start` | `{ sessionId, turnId, providerId }` | agent 开始响应 |
| `cronymax.message.assistant.delta` | `{ sessionId, turnId, textDelta }` | 流式 token |
| `cronymax.message.assistant.done` | `{ sessionId, turnId, fullText, finishReason }` | 当前 turn 收尾 |

### Tools / Permissions
| 主题 | payload |
|---|---|
| `cronymax.tool.invoked` | `{ sessionId, turnId, toolCallId, name, input, source }` |
| `cronymax.tool.completed` | `{ sessionId, toolCallId, status, output }` |
| `cronymax.permission.requested` | `{ sessionId, requestId, target, options }` |
| `cronymax.permission.decided` | `{ requestId, decision, by }` |

### Flow
| 主题 | payload |
|---|---|
| `cronymax.flow.started` | `{ flowId, runId }` |
| `cronymax.flow.step.started` | `{ runId, stepId, type }` |
| `cronymax.flow.step.completed` | `{ runId, stepId, output }` |
| `cronymax.flow.step.failed` | `{ runId, stepId, error }` |
| `cronymax.flow.ended` | `{ runId, status }` |

### Config / Workspace
| 主题 | payload |
|---|---|
| `cronymax.config.changed` | `{ section, affected }` |
| `cronymax.workspace.folders.changed` | `{ added, removed }` |

加新主题 = 平台改代码 + 更新本文档。

---

## 4. L2 Extension Points

12 个。每个由平台特定 UI / 核心逻辑模块消费。命名空间 `cronymax.*` 锁定。

| EP | 谁消费 | 干什么 | M0 |
|---|---|---|---|
| `cronymax.command` | 命令面板 / 快捷键引擎 | 注册具名命令 | ✅ |
| `cronymax.keybinding` | 快捷键引擎 | 绑定命令到按键 | ✅ |
| `cronymax.menu.item` | 菜单系统 | 把命令塞进菜单/右键 | M1 |
| `cronymax.config.schema` | 设置面板 | 声明式 JSON Schema 设置 | ✅ |
| `cronymax.config.page` | 设置面板 | 自定义 webview 设置页 | ✅ |
| `cronymax.ui.activitybar.item` | 活动栏 | 注册图标入口 | M1 |
| `cronymax.ui.sidebar.view` | 侧栏 | 注册侧栏面板 | ✅ |
| `cronymax.ui.statusbar.item` | 状态栏 | 注册状态栏 item | M1 |
| `cronymax.content.renderer` | 内容渲染管道 | 注册 block / inline 渲染器 | ✅ |
| `cronymax.chat.session-provider` | 聊天面板 / flow runtime | **注册 chat 会话提供者（coco-acp 在这）** | ✅ |
| `cronymax.chat.tool` | 工具调度器 | 注册 agent 可调的 tool | M1 |
| `cronymax.workspace.fs-provider` | 工作区 fs | 注册虚拟文件系统 scheme | M1 |

每个 EP 的正式规范包含：
- 命名（`cronymax.*` reserved）
- 清单 schema（`contributes` 字段格式）
- runtime 注册 API（`cronymax.X.register(...)`）
- 平台消费方代码（哪个模块读这个 EP 的注册）
- capability 要求

详细 schema 见 `docs/extensions/ep-schemas/`（M0 期间起草）。

### 4.1 `cronymax.chat.session-provider` 完整规范（重点）

**清单声明**：
```jsonc
"contributes": {
  "cronymax.chat.session-provider": [{
    "id": "coco",                           // 在所有已装扩展里唯一
    "label": "Coco",
    "icon": "$(coco)",                      // codicon 引用
    "description": "ByteDance Coco via ACP",
    "supportsModels": true,                 // 是否需要二级模型选择
    "supportsModes": true,                  // 是否有 mode（plan / default 等）
    "supportsMcp": true                     // 是否接受 mcpServers 注入
  }]
}
```

**Runtime 注册**：
```ts
cronymax.chat.registerSessionProvider(id: string, impl: SessionProvider): Disposable

interface SessionProvider {
  listModels(): Promise<ModelInfo[]>;
  modes?: ModeInfo[];                       // 静态模式列表
  createSession(opts: SessionOptions): Promise<Session>;
}

interface SessionOptions {
  cwd: string;
  model?: string;
  mode?: string;
  mcpServers?: McpServerSpec[];             // 平台从其他扩展收集进来
  systemPrompt?: string;                    // flow runtime / agent yaml 注入
  allowedTools?: string[];
}

interface Session {
  readonly id: string;
  prompt(message: PromptMessage, token: CancellationToken): AsyncIterable<ChatEvent>;
  cancel(): Promise<void>;
  dispose(): Promise<void>;
}

type ChatEvent =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "toolCall"; id: string; name: string; input: any; source: string; status: "in_progress" }
  | { kind: "toolCallUpdate"; id: string; status: "completed" | "failed"; output: any }
  | { kind: "permissionRequest"; requestId: string; tool: string; options: any }
  | { kind: "done"; stopReason: "end_turn" | "max_tokens" | "tool_calls" | "cancelled" | "error" };
```

**平台消费方**：
- 聊天面板（`web/src/panels/chat/`）—— UI 显示
- flow runtime（`crates/cronymax/src/flow/`）—— agent 调度

两个消费方共享同一份 provider 注册数据。

**Capability**：
```jsonc
"extension-points": ["cronymax.chat.session-provider"]
```

---

## 5. 扩展清单 schema

```jsonc
{
  // ── 标识（必填）
  "id":        "publisher.name",     // 必须 publisher 前缀；唯一
  "name":      "Display Name",
  "version":   "1.0.0",              // semver
  "publisher": "publisher",          // 必须存在；对应 marketplace 账号
  "engines":   { "cronymax": "^1.0" },
  "main":      "./dist/main.js",     // 可省，纯声明式扩展无需
  
  // ── 描述（建议）
  "description": "...",
  "icon":      "./icon.png",
  "homepage":  "https://...",
  "repository": "...",
  "license":   "MIT",
  
  // ── 激活
  "activationEvents": [
    "onSessionProvider:coco",
    "onCommand:coco.openSettings",
    "onView:coco.history",
    "onStartup",                     // 慎用
    "*"                              // 禁用，安装期警告
  ],
  
  // ── 静态贡献到 L2 EPs
  "contributes": {
    "cronymax.chat.session-provider": [...],
    "cronymax.command": [...],
    "cronymax.config.schema": {...},
    "cronymax.config.page": [...],
    // ...其他 EP
  },
  
  // ── 能力声明（capability 门控）
  "capabilities": {
    "process":           { "allow": [{ "command": "...", "argsPattern": [...] }] },
    "fs":                { "scope": "workspace", "mode": "rw" },
    "network":           { "allow": ["api.foo.com"] },
    "secrets":           { "namespace": "publisher.name.*" },
    "events.subscribe":  ["cronymax.message.assistant.done"],
    "events.emit":       ["publisher.name.*"],
    "ui-slots":          ["sidebar", "settings"],
    "extension-points":  ["cronymax.chat.session-provider", "cronymax.command"],
    "auth.providers":    ["oauth-generic"]
  },
  
  // ── 显式跨扩展依赖（用 extension.exports 时申报）
  "extensionDependencies": ["other.extension"]
}
```

校验规则：
- `id` 必须 `<publisher>.<name>` 格式，`<publisher>` 必须等于 `publisher` 字段
- `contributes` 的 key 必须以 `cronymax.` 开头（属于平台 EP），且必须在 `capabilities.extension-points` 申报过
- 任何往 `cronymax.*` 命名空间的写入（emit、provide）一律拒绝
- `activationEvents` `*` 通配在安装期警告，不强禁

---

## 6. 安全模型

### 6.1 命名空间

| 命名空间 | 谁拥有 | 第三方能写吗 |
|---|---|---|
| `cronymax.*` | 平台 | **否**，安装期拒绝 |
| `<publisher>.*` | 该 publisher | 仅自家扩展 |
| 其他 | 任何 | 容忍（但建议遵循 publisher 前缀） |

冒充 `cronymax.secrets` 之类的攻击在静态校验阶段就死。

### 6.2 Capability 门控

每个 L1 API 都有对应的 capability：
- 安装期把 capabilities 译成**人话清单**给用户审，例子见 [8.2](#82-安装阶段)
- 运行时每次调用校验；失败 throw `CapabilityDeniedError`
- 用户随时可在"设置 → 扩展 → <ext> → 权限"撤销单项

特殊规则：
- `secrets.namespace` 必须等于 `<publisher>.*` 或其子集，不允许跨 publisher 读写
- `process.allow.command` 可引用 config 值 `${config:foo.bar}`，运行时解析
- `network.allow` 支持 host 通配（`*.foo.com`），不支持 path
- `events.emit` 不能含 `cronymax.*`

### 6.3 沙箱

- **JS runtime**：CEF helper process + V8。多个扩展共享一个 helper（受信任时）；不受信任的扩展开独立 helper
- **Webview**：独立 origin sandbox iframe `cronymax-webview://<ext-id>/<path>`
- **Process runtime（v2）**：subprocess + Unix socket

资源额度（运行时硬限）：
- 单扩展 CPU 时间 / 内存上限
- iframe 数量上限
- 子进程数量上限
- 网络请求并发 / QPS 上限

### 6.4 信任分级

| 来源 | 默认能力 |
|---|---|
| 内置扩展（cronymax 自带） | 全开（平台代码自身） |
| Marketplace 已签名（v2） | manifest 声明全允许 |
| 本地未签名（开发期） | 敏感 capability 默认禁用，需用户每次手动启用 |

v1 阶段无 marketplace，只支持本地 .crx + cronymax 内置扩展。

---

## 7. 扩展生命周期与 Runtime

### 7.1 生命周期

```
未安装 → install → 已安装/未启用 → enable → 已启用/未激活
                                                   ↓ activationEvent
                                              已激活/运行中
                                                   ↓ deactivate
                                              已启用/未激活
                                                   ↓ disable
                                              已安装/未启用
                                                   ↓ uninstall
                                              未安装
```

- **install**：解压 .crx 到 `~/.cronymax/extensions/<id>/`，校验 manifest，弹 capability 授权对话框
- **enable**：默认启用；用户可禁用（不卸载）
- **activate**：触发 activationEvent 时，host runtime 加载 `main.js`，调 `activate(ctx)`
- **deactivate**：用户禁用/撤回/卸载时，host 调 `deactivate()`，`ctx.subscriptions` 全 dispose
- **crash**：host 进程崩了，平台自动重启 N 次（默认 3），超过则退而禁用并通知用户

### 7.2 JS Runtime（v1 唯一）

CEF helper process 跑 V8：
- 加载多个扩展的 `main.js` 到同一个 V8 隔离（受信扩展共享）
- 暴露 `cronymax/v1` 给扩展（每个扩展只能看自家 ctx 范围内 API）
- 通过现有 CEF browser/renderer IPC 把 API 调用转给 Rust core
- 不受信扩展开独立 helper

不提供 Node API。只有：
- `cronymax.*` namespace
- `fetch` / `WebSocket`（受 capability.network 门控）
- `URL` / `TextEncoder` / `crypto` 等 Web 标准
- ES2022+ JavaScript

要 Node 的扩展走 Process runtime（v2）。

---

## 8. 端到端走查：coco-acp

这是 dogfood 用例。验证整个系统能撑住实际场景。

### 8.1 扩展包结构

```
coco-extension/
├── cronymax-extension.json     # 清单
├── src/
│   ├── main.ts                 # activate / deactivate
│   ├── acp-client.ts           # ACP stdio 客户端（移植自我们已验过的 Python POC）
│   ├── coco-session.ts         # 实现 cronymax.chat.Session
│   └── settings/
│       ├── index.html          # 自定义设置页
│       └── index.ts
├── dist/                       # tsc 输出
├── package.json                # 仅开发期 / npm 用，cronymax 不读
└── README.md
```

### 8.1.1 cronymax-extension.json

```jsonc
{
  "id": "bytedance.coco",
  "name": "Coco",
  "version": "0.1.0",
  "publisher": "bytedance",
  "engines": { "cronymax": "^1.0" },
  "main": "./dist/main.js",

  "activationEvents": [
    "onSessionProvider:coco",
    "onCommand:coco.openSettings"
  ],

  "contributes": {
    "cronymax.chat.session-provider": [{
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
        "coco.binaryPath": {
          "type": "string", "default": "coco",
          "description": "Path to the coco binary"
        },
        "coco.defaultModel": {
          "type": "string", "default": "GPT-5.4"
        },
        "coco.defaultMode": {
          "type": "string",
          "enum": ["default", "plan", "bypass_permissions"],
          "default": "default"
        },
        "coco.yolo": {
          "type": "boolean", "default": false,
          "description": "Bypass tool permission checks (not recommended)"
        }
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
      "cronymax.chat.session-provider",
      "cronymax.command",
      "cronymax.config.schema",
      "cronymax.config.page"
    ]
  }
}
```

### 8.1.2 src/main.ts

```typescript
import * as cronymax from "@cronymax/extension";
import { AcpClient } from "./acp-client";
import { CocoSession } from "./coco-session";

export async function activate(ctx: cronymax.ExtensionContext) {
  ctx.subscriptions.push(
    cronymax.chat.registerSessionProvider("coco", {

      async listModels(): Promise<cronymax.chat.ModelInfo[]> {
        const client = await AcpClient.spawnEphemeral(ctx);
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

      async createSession(opts: cronymax.chat.SessionOptions) {
        const cfg = cronymax.workspace.getConfiguration("coco");
        const client = await AcpClient.spawn(ctx, {
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
        return new CocoSession(client, acp, opts);
      }
    }),

    cronymax.commands.register("coco.openSettings", () =>
      cronymax.window.openConfigPage("coco.advanced"))
  );
}

export async function deactivate() { /* subscriptions auto-cleaned */ }
```

### 8.1.3 src/coco-session.ts

```typescript
import * as cronymax from "@cronymax/extension";
import type { AcpClient, AcpEvent } from "./acp-client";

export class CocoSession implements cronymax.chat.Session {
  readonly id: string;

  constructor(
    private client: AcpClient,
    private acp: { sessionId: string },
    private opts: cronymax.chat.SessionOptions
  ) {
    this.id = `coco:${acp.sessionId}`;
  }

  async *prompt(
    message: cronymax.chat.PromptMessage,
    token: cronymax.CancellationToken
  ): AsyncIterable<cronymax.chat.ChatEvent> {
    const stream = this.client.streamPrompt({
      sessionId: this.acp.sessionId,
      prompt: [{ type: "text", text: message.text }],
      cancellationToken: token
    });

    for await (const event of stream) {
      const translated = translate(event);
      if (translated) yield translated;
    }
  }

  async cancel()  { await this.client.cancel(this.acp.sessionId); }
  async dispose() { await this.client.shutdown(); }
}

function translate(e: AcpEvent): cronymax.chat.ChatEvent | null {
  switch (e.type) {
    case "agent_message_chunk":
      return { kind: "text", text: e.text };
    case "agent_thought_chunk":
      return { kind: "thinking", text: e.text };
    case "tool_call":
      return {
        kind: "toolCall",
        id: e.toolCallId,
        name: e.title,
        input: e.rawInput,
        source: e._meta?.mcpServerName ?? "coco",
        status: "in_progress"
      };
    case "tool_call_update":
      return {
        kind: "toolCallUpdate",
        id: e.toolCallId,
        status: e.status === "completed" ? "completed" : "failed",
        output: e.rawOutput
      };
    case "permission_request":
      return {
        kind: "permissionRequest",
        requestId: e.requestId,
        tool: e.tool,
        options: e.options
      };
    case "done":
      return { kind: "done", stopReason: e.stopReason };
    default:
      return null;
  }
}
```

### 8.1.4 src/acp-client.ts

JSON-RPC over stdio。结构跟我们已验过的 `/tmp/acp_mcp_client.py` 同构，移植到 TS。

```typescript
import * as cronymax from "@cronymax/extension";

export class AcpClient {
  private nextId = 1;
  private pending = new Map<number, { resolve, reject }>();
  private notifySubs = new Map<string, Set<(payload) => void>>();

  static async spawn(ctx, opts) {
    const proc = await cronymax.process.spawn({
      command: opts.binaryPath,
      args: ["acp", "serve", ...(opts.yolo ? ["--yolo"] : [])],
      cwd: cronymax.workspace.rootUri?.fsPath
    });
    return new AcpClient(proc);
  }

  static async spawnEphemeral(ctx) { /* 类似，跑完即杀 */ }

  constructor(private proc: cronymax.process.ChildProcess) {
    this.readLoop();
  }

  private async readLoop() { /* 按行解 JSON，dispatch response/notification */ }
  
  async initialize() { return this.request("initialize", { protocolVersion: 1, clientCapabilities: {...} }); }
  async newSession(params) { return this.request("session/new", params); }
  async setMode(sessionId, mode) { return this.request("session/setMode", { sessionId, mode }); }
  async setModel(sessionId, model) { return this.request("session/setModel", { sessionId, model }); }
  async cancel(sessionId) { return this.request("session/cancel", { sessionId }); }
  async respondPermission(requestId, decision) { return this.request("session/respondPermission", { requestId, decision }); }
  async shutdown() { this.proc.kill(); }

  async *streamPrompt(params) {
    const id = this.nextId++;
    const sub = new Set<(e) => void>();
    // 关键：session/update notification 按 sessionId 路由进 sub
    // 收到 response 后流结束
    // 见 acp_mcp_client.py 的同构实现
  }

  private request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }
  
  private send(msg) {
    this.proc.stdin.write(JSON.stringify(msg) + "\n");
  }
}
```

### 8.2 安装阶段

```
1. 用户在 cronymax 设置 > Extensions 里点 "Install from File"，选 bytedance.coco-0.1.0.crx
   或 CLI: cronymax ext install ./bytedance.coco-0.1.0.crx

2. 平台扩展管理器：
   a. 解压 .crx 到 ~/.cronymax/extensions/bytedance.coco/
   b. 读 cronymax-extension.json
   c. 校验 manifest schema
   d. 校验：
      - id "bytedance.coco" 格式合法（publisher.name）
      - publisher "bytedance" 不在保留列表
      - contributes 所有 key 都在 capabilities.extension-points 申报过
      - 没有往 cronymax.* 命名空间写入

3. 平台弹安装授权对话框：

   ┌─────────────────────────────────────────────────────────────┐
   │ 安装 Coco                                                   │
   │ by bytedance · v0.1.0                                       │
   │                                                             │
   │ ByteDance Coco via ACP                                      │
   │                                                             │
   │ 此扩展将能够：                                                │
   │  ⚙ 启动进程：${coco.binaryPath} acp serve ...                │
   │  📁 在工作区读写文件                                          │
   │  🔑 存储 bytedance.coco.* 命名空间下的密钥                    │
   │  💬 作为聊天会话提供者出现在 agent 选择器                      │
   │  ⌨ 注册命令：Coco: Open Settings                            │
   │  ⚙ 贡献设置项与设置页面                                       │
   │                                                             │
   │ [详情]                          [取消]  [安装]               │
   └─────────────────────────────────────────────────────────────┘

4. 用户点 [安装]
   a. 平台写入 ~/.cronymax/extensions/registry.json：
      {
        "bytedance.coco": {
          "enabled": true,
          "version": "0.1.0",
          "installedAt": "2026-05-18T...",
          "grantedCapabilities": ["process", "fs", "secrets", ...]
        }
      }
   b. 状态：已安装、已启用、未激活
   c. 平台扫一遍 manifest activationEvents，登记到激活事件索引
```

### 8.3 激活阶段

```
1. 用户某时刻打开聊天面板
   → 聊天面板初始化时静态扫描所有"已安装且已启用"的扩展 manifest
   → 拼出 session-provider 选项列表（这一步无需激活，仅读 manifest 静态数据）
   → 用户看到下拉里多了 "Coco" 选项（图标灰色，标 "Not yet started"）

2. 用户点选 "Coco"
   → 聊天面板触发激活事件 onSessionProvider:coco
   → 平台扩展管理器查激活事件索引 → bytedance.coco
   → 扩展尚未激活，开始激活流程：
     a. 检查信任级别（内置/已签名/未签名）
     b. 决定用共享 helper 还是独立 helper（v1 默认共享）
     c. CEF helper process 起 V8（如还没起）
     d. 加载 ~/.cronymax/extensions/bytedance.coco/dist/main.js
     e. 调用导出的 activate(ctx)，ctx 注入按 capabilities 限制过的 cronymax/v1
   → main.ts 跑：
     - cronymax.chat.registerSessionProvider("coco", impl)
     - cronymax.commands.register("coco.openSettings", ...)
   → 平台注册中心存入这些 Disposable

3. 聊天面板感知 registry 更新，"Coco" 选项变为 "Ready"
```

### 8.4 聊天面板使用

```
1. 用户配置 "Coco / GPT-5.4 / Plan mode"，按发送

2. 聊天面板：
   const provider = registry.get("cronymax.chat.session-provider", "coco");
   const session = await provider.createSession({
     cwd: workspace.rootUri.fsPath,
     model: "GPT-5.4",
     mode: "plan",
     mcpServers: collectMcpServersFromAllExtensions()
   });

3. main.ts 的 createSession() 跑：
   - cronymax.process.spawn("coco", ["acp", "serve"])   ← 受 capability.process 门控
   - ACP initialize 握手
   - ACP session/new
   - setMode("plan") / setModel("GPT-5.4")
   - 返回 CocoSession

4. 聊天面板：
   cronymax.events.emit("cronymax.session.started", {sessionId, providerId: "coco", model})
   cronymax.events.emit("cronymax.message.user.sent", {sessionId, turnId, text})
   
   for await (const event of session.prompt(msg, cancelToken)) {
     switch (event.kind) {
       case "text":
         renderTextDelta(event.text);
         cronymax.events.emit("cronymax.message.assistant.delta", {...});
         break;
       case "thinking":
         renderThinking(event.text);
         break;
       case "toolCall":
         renderToolCard(event);
         cronymax.events.emit("cronymax.tool.invoked", {...});
         break;
       case "toolCallUpdate":
         updateToolCard(event);
         cronymax.events.emit("cronymax.tool.completed", {...});
         break;
       case "permissionRequest":
         const decision = await cronymax.permissions.askUser({tool, options});
         await provider.respondPermission(event.requestId, decision);
         break;
       case "done":
         finalizeMessage();
         cronymax.events.emit("cronymax.message.assistant.done", {...});
         return;
     }
   }
```

任何订阅 `cronymax.message.*` 的扩展（评分 / 审计 / 统计）此时都能收到事件。

### 8.5 flows agent 配置

cronymax 已有概念：`.cronymax/agents/<id>.agent.yaml` 定义一个"持久化人格"。v1 扩展该 schema 让它能引用任意 session-provider。

#### 8.5.1 agent yaml schema 扩展

```yaml
id: code-reviewer
name: Code Reviewer
description: Reviews PRs for code quality
icon: $(check)

# 必填：底层 session-provider 的 id
# 引用 cronymax.chat.session-provider EP 的 id 字段
provider: coco

# Provider 配置（schema 由 provider 决定，平台不解析内容）
provider_config:
  model: GPT-5.4
  mode: plan

# 平台级字段（与 provider 无关）
system_prompt: |
  You are a senior code reviewer. Be terse and direct.
  Focus on correctness, security, and performance.

# 工具白名单（cronymax 在调度时按此过滤 tool 调用）
allowed_tools:
  - "cronymax.tool.shell"
  - "cronymax.tool.fs.read"
  - "github.review.*"     # 来自其他扩展贡献的 tool

# MCP servers 注入（如果 provider supportsMcp）
mcp_servers:
  - id: github
    config:
      token_secret_ref: bytedance.github.token

# 偏好
auto_compact: true
default_max_turns: 30
```

`provider` 字段引用一个**已注册的 session-provider id**。可以是：
- coco（来自 bytedance.coco 扩展）
- native（cronymax 内置）
- 任何其他贡献了 `cronymax.chat.session-provider` 的扩展

#### 8.5.2 flows panel · Agents 标签

```
┌────────────────────────────────────────────────────────────────┐
│ Flows                                              [+ New Agent]│
│ ┌──────────┬─────────────┬────────────────┬──────────┐         │
│ │ Flows    │ Agents      │ History        │ Settings │         │
│ └──────────┴─────────────┴────────────────┴──────────┘         │
│                                                                │
│  ┌────────────────────────────────────────────────────────────┐│
│  │ ICON  Code Reviewer                          coco · GPT-5.4││
│  │       Reviews PRs for code quality            edit · delete││
│  ├────────────────────────────────────────────────────────────┤│
│  │ ICON  Meeting Notes                       native · Claude-O││
│  │       Summarizes meeting transcripts                       ││
│  ├────────────────────────────────────────────────────────────┤│
│  │ ICON  Research Assistant                 coco · Doubao-Code││
│  │       Deep research with web tools                         ││
│  └────────────────────────────────────────────────────────────┘│
└────────────────────────────────────────────────────────────────┘
```

#### 8.5.3 [+ New Agent] 向导

**Step 1/4 · 选择会话提供者**

```
┌────────────────────────────────────────────────────────────────┐
│ 创建 Agent — 选择会话提供者                                      │
│                                                                │
│ ○ Native       cronymax 内置（Anthropic / OpenAI / ...）         │
│ ●  Coco        bytedance.coco                                  │
│ ○ 其他...                                                       │
│                                                                │
│                                                  [取消] [下一步]│
└────────────────────────────────────────────────────────────────┘

实现：
- flows panel 读 registry：
  const providers = await registry.list("cronymax.chat.session-provider");
- 列出所有已安装 + 已启用扩展贡献的 provider
- 跟聊天面板的 agent picker 用同一份数据
```

**Step 2/4 · 选择模型与模式**

用户选 "Coco" 后：

```
┌────────────────────────────────────────────────────────────────┐
│ 创建 Agent — 模型与模式                                          │
│                                                                │
│ 模型: [GPT-5.4                            ▾]                   │
│       Context window: 240k, Max tool turns: 200                │
│       ╾━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━╼          │
│       GPT-5.4      Context 240k                                │
│       Doubao-Code  Context 184k                                │
│       Kimi-K2.6                                                │
│       DeepSeek-V4-Pro                                          │
│       ... (从 provider.listModels() 拿)                         │
│                                                                │
│ 默认模式: [Plan ▾]                                              │
│       - Default                                                │
│       - Plan                                                   │
│       - Bypass Permissions                                     │
│       (从 provider.modes 拿)                                    │
│                                                                │
│                                            [上一步]  [下一步]   │
└────────────────────────────────────────────────────────────────┘

实现：
- 触发激活 onSessionProvider:coco（让扩展启动）
- 调 provider.listModels() 拿模型列表
- 读 provider.modes 拿模式静态列表
```

**Step 3/4 · 配置人格**

```
┌────────────────────────────────────────────────────────────────┐
│ 创建 Agent — 配置人格                                            │
│                                                                │
│ 名称*:        [Code Reviewer                                ]  │
│ 描述:         [Reviews PRs for code quality                 ]  │
│ 图标:         [$(check)                                     ]  │
│                                                                │
│ System Prompt*:                                                │
│ ┌────────────────────────────────────────────────────────────┐│
│ │ You are a senior code reviewer. Be terse and direct.       ││
│ │ Focus on correctness, security, and performance.           ││
│ │                                                            ││
│ │                                              支持模板变量   ││
│ └────────────────────────────────────────────────────────────┘│
│                                                                │
│                                            [上一步]  [下一步]   │
└────────────────────────────────────────────────────────────────┘
```

**Step 4/4 · 工具与 MCP**

```
┌────────────────────────────────────────────────────────────────┐
│ 创建 Agent — 工具与 MCP                                          │
│                                                                │
│ 允许的工具:                                                      │
│  ☑ cronymax.tool.shell    (cronymax 内置)                       │
│  ☑ cronymax.tool.fs.read  (cronymax 内置)                       │
│  ☐ cronymax.tool.fs.write (cronymax 内置)                       │
│  ☑ github.review.*        (来自 acme.github 扩展)               │
│  ☐ python.run             (来自 acme.python-tool 扩展)          │
│                                                                │
│ MCP Servers:                                                   │
│  [+ Add MCP Server]                                            │
│  ─ github   ✕                                                  │
│      Config: { "token_secret_ref": "bytedance.github.token" }  │
│                                                                │
│ 选项:                                                           │
│  ☑ Auto compact                                                │
│  Max turns: [30]                                               │
│                                                                │
│                                            [上一步]  [完成]     │
└────────────────────────────────────────────────────────────────┘

实现：
- "允许的工具" 列出所有已注册 cronymax.chat.tool EP 的 id
- 用户保存 → 写入 .cronymax/agents/<auto-id>.agent.yaml
- 触发 cronymax.events.emit("cronymax.flow.agent.created", {agentId})
```

### 8.6 flows 运行时调度

flow yaml（cronymax 已有 schema）：

```yaml
id: review-pr
name: Review PR
inputs:
  pr_url:
    type: string

steps:
  - id: fetch-pr
    type: shell
    command: gh pr view ${inputs.pr_url} --json files,title,body

  - id: review
    type: agent
    agent: code-reviewer          # 引用 .cronymax/agents/code-reviewer.agent.yaml
    prompt: |
      Please review this PR:
      ${steps.fetch-pr.output}

  - id: post-comment
    type: shell
    command: |
      gh pr comment ${inputs.pr_url} -b "${steps.review.output}"
```

运行时流程：

```
1. 用户运行 flow review-pr，传 pr_url=https://github.com/foo/bar/pull/1

2. flow runtime 顺序执行 steps：

3. fetch-pr step（shell type）→ 跑 gh 命令拿到 PR 数据

4. review step（agent type）：
   a. 读 .cronymax/agents/code-reviewer.agent.yaml
   b. 解析 provider 字段 = "coco"
   c. 查 registry：
      const provider = await sessionProviderRegistry.get("coco");
      → 触发 onSessionProvider:coco 激活（若还未激活）
      → 返回 Coco 扩展贡献的 SessionProvider impl
   d. 调用 provider.createSession({
        cwd: workspace.rootUri.fsPath,
        model: agent.provider_config.model,       // "GPT-5.4"
        mode: agent.provider_config.mode,         // "plan"
        mcpServers: resolveMcpServers(agent.mcp_servers),
        systemPrompt: agent.system_prompt,
        allowedTools: agent.allowed_tools
      })
   e. 把 step.prompt 渲染（替换 ${steps.fetch-pr.output}）后送进 session.prompt()
   f. 收集事件流：
      - text 事件累积成 finalText
      - toolCall 事件经平台中转，按 agent.allowed_tools 门控：
          if (!matchAnyAllowlist(toolName, agent.allowed_tools)) {
            return { decision: "deny", reason: "tool not in allowlist" }
          }
      - permissionRequest 自动走 allowlist 决策（不弹窗，因为 flow 是无人值守运行）
      - done 事件结束当前 step
   g. step.output = finalText
   h. session.dispose() 关 coco 进程

5. post-comment step（shell type）→ 把 step.output 写回 PR

6. flow 结束：
   cronymax.events.emit("cronymax.flow.ended", {runId, status: "completed"})
```

**关键观察**：
- flow runtime（平台代码）和聊天面板（平台代码）**消费同一份 session-provider registry**
- Coco 扩展无需为 "在 flow 里用" 写任何额外代码 —— 它只需要实现 `SessionProvider` 接口
- agent yaml 是"persona 配置 + provider 引用"，跟 provider 实现细节完全解耦
- 同一个 Coco 扩展可以被多个 agent 用（Code Reviewer、Meeting Notes、Research Assistant 都引用 `provider: coco` 但 system_prompt / 模型不同）

---

## 9. 平台代码改动清单

### `crates/cronymax/src/extensions/`（新增）
- `manifest.rs` — schema 解析与校验
- `registry.rs` — 扩展元数据扫描 + 启用状态持久化
- `activation.rs` — activationEvents 匹配引擎
- `host/`
  - `js.rs` — CEF helper + V8 host
  - `process.rs`（v2）— subprocess + Unix socket host
- `capability.rs` — 能力门控
- `contributions/` — L2 EP 注册中心
  - `chat_session_provider.rs`
  - `content_renderer.rs`
  - `command.rs`
  - `config_schema.rs`
  - `config_page.rs`
  - `sidebar_view.rs`
- `events.rs` — L1.5 平台事件 emit/subscribe
- `api/` — `cronymax/v1` API 实现
  - `commands.rs`
  - `events.rs`
  - `fs.rs`
  - `process.rs`
  - `network.rs`
  - `secrets.rs`
  - `webview.rs`
  - `chat.rs`
  - `window.rs`
  - `workspace.rs`
  - `extensions.rs`

### `web/src/panels/chat/`（修改）
- 替换硬编码 LLM provider 列表为：
  ```ts
  const providers = await bridge.send("extensions/listContributions",
    "cronymax.chat.session-provider");
  ```
- session 创建/路由统一走扩展贡献的 provider
- 事件流转翻译统一格式

### `web/src/panels/flows/agents/`（新增）
- `AgentList.tsx`
- `NewAgentWizard.tsx`（4 步）
- `AgentEditor.tsx`
- 与 session-provider registry 集成

### `crates/cronymax/src/flow/`（修改）
- `agent` 类型 step：从硬编码 provider 改为通过 session-provider EP 调度
- 加载 .cronymax/agents/<id>.agent.yaml 时解析 `provider` 字段
- 把 `allowed_tools` 透传给 provider session，并门控 toolCall 事件

### `crates/cronymax/src/capability/agent_loader.rs`（修改）
- `AgentDef` 增加 `provider` + `provider_config` 字段
- 加载时校验 provider id 在 registry 存在

### `web/src/panels/settings/extensions/`（新增）
- 扩展列表 / 安装 / 卸载 / 启用 / 禁用 / 配置 / 撤销 capability

### `web/src/shells/`（新增 `extension.ts`）
- 暴露 cronymax/v1 API 给扩展 JS host
- 通过 bridge 转给 Rust core

### `web/src/extensions/sdk/`（新增）`@cronymax/extension`
- TypeScript SDK，提供 cronymax/v1 类型
- 编译产物发布到 npm（开发期用本地链接）

---

## 10. M0 范围与节奏

### 必须（v1 alpha）

- L1 Kernel 14 条全实现
- L1.5 事件主题：8 条入门（session / message / tool / permission / config）
- L2 EP 6 条入门：
  - `cronymax.command`
  - `cronymax.config.schema`
  - `cronymax.config.page`
  - `cronymax.chat.session-provider`
  - `cronymax.content.renderer`
  - `cronymax.ui.sidebar.view`
- 安装 / 卸载 / 启用 / 禁用 / capability gate UI
- `@cronymax/extension` TS SDK
- 扩展管理 UI
- **`bytedance.coco` 内置扩展作为 dogfood**
- **flows agent 系统适配 session-provider**（新建 agent 向导能选 Coco + Coco 在 flow 运行时被调度）

### 先不做（推 M1+）

- Marketplace（v1 用本地 .crx 安装）
- 数字签名（先靠 sandbox + capability + 用户授权）
- 跨扩展依赖图 UI
- WASM runtime
- Process runtime（v1 只有 JS runtime；外部 ACP/MCP 由 JS 扩展 spawn）
- 远程开发
- 正式 L3 服务 registry（schema + semver）

### 节奏估算

| 周 | 工作 |
|---|---|
| 1-2 | manifest / registry / activation 内核 |
| 3-5 | L1 Kernel API 实现（含 CEF helper V8 host） |
| 4 | L1.5 事件主题铺管线 + 头 4 条 |
| 5-6 | L2 EP × 6 实现 + 平台消费方代码改造 |
| 6-7 | 聊天面板 + flows agent 系统对齐 session-provider EP |
| 7-8 | `bytedance.coco` 内置扩展打通（含设置页） |
| 8-9 | `@cronymax/extension` TS SDK + 扩展管理 UI |
| 9-10 | 端到端测试 / 文档 / capability UX 打磨 |

总计 9-10 周到 alpha。

### M1 之后

- 第二个外部扩展（demo theme / demo content renderer）
- 更多 L2 EP（ui.activitybar.item / ui.statusbar.item / workspace.fs-provider / content.fenced-mime / chat.tool / lm.provider / auth.provider / keybinding / menu.item）
- Process runtime（让 Python / Go 扩展能写）
- 抽出 `community.acp-bridge` 让其他 ACP agent 零代码接入
- 抽出 `community.mcp-bridge` 让 MCP server 声明式接入

---

## 附录 A · 还未拍板的决策点

| # | 决策 | 选项 | 倾向 |
|---|---|---|---|
| 1 | TS SDK codegen 来源 | IDL（.ts interface 编译）/ JSON Schema 手写 / 手写 .d.ts | IDL |
| 2 | CEP 二进制编码 | 自卷 VS Code RPCProtocol / MessagePack-RPC / Cap'n Proto | MessagePack |
| 3 | 清单文件名 | `cronymax-extension.json` / `package.json` / `extension.toml` | `cronymax-extension.json` |
| 4 | 共享 V8 host vs 一扩展一进程 | 默认共享 + 不受信开独立 / 全独立 | 默认共享 |
| 5 | v1 是否含 webview content renderer | 含 / 推 M1 | 含（mermaid 等渲染 v1 就要） |

需要在 M0 启动前定稿。

---

## 附录 B · 词汇

- **Kernel API** — `cronymax.*` namespace 提供的能力 API
- **L2 EP** — Extension Point，平台 UI/逻辑消费的具名贡献槽位
- **Provider** — 一个扩展实现的、可被平台调度的能力提供方（如 session-provider, fs-provider）
- **Agent** — `.cronymax/agents/<id>.agent.yaml` 里定义的"persona + provider 引用 + 工具白名单"配置
- **Convention** — 文档级约定的字符串名（非强制），不是 L2 EP

---

文档版本：v0.1 草案 · 2026-05-18
评审：待
