# `@cronymax/extension` API 参考

`cronymax` namespace 的 v1 surface 速查。**类型与精确签名以 `sdk/extension/src/*.ts` 为准**(本文件是导览,源码是真相)。IDL v1 已 freeze:只增不改。

入口在 [`sdk/extension/src/index.ts`](../../sdk/extension/src/index.ts) 的 `interface Cronymax`。

```ts
import * as cronymax from "@cronymax/extension";
// 或按需:import { window, commands, events } from "@cronymax/extension";

export async function activate(ctx: cronymax.ExtensionContext) { /* ... */ }
export function deactivate() {}                                  // 可选
```

运行时由 host(`bundled/extension-host-bootstrap.js`)注入到 `globalThis.cronymax`;SDK 在 import 时读取该全局。

---

## ExtensionContext(`lifecycle.ts`)

`activate(ctx)` 收到的对象:

- `ctx.subscriptions: Disposable[]` — push 进去的会在停用时自动 dispose。**所有注册都挂这里**。
- `ctx.extensionMode: ExtensionMode` — `production` | `development`。
- `ctx.storagePath` / `ctx.globalStoragePath` — 每扩展私有目录(canonical 绝对路径)。
- `ctx.workspaceFolders: WorkspaceFolder[]` / `ctx.rootUri` — 当前打开的 workspace 根(零个时为 `[]`)。

---

## L1 — kernel 原语

### `env`(`index.ts`)
`appName` · `platform`(`darwin`/`linux`/`win32`)· `machineId`(每安装稳定)· `homedir`。

### `commands`(`commands.ts`)
- `register(id, handler): Disposable` — id 必须在 manifest `cronymax.command` 申报过。
- `execute(id, ...args): Promise<unknown>`。

### `events`(`events.ts`)— 平台事件总线(L1.5)
- `on(topic, handler): Disposable` — 订阅平台 `cronymax.*` 事件(需在 `capabilities.events.subscribe` 白名单)。
- `emit(topic, payload): Promise<void>` — 发自有 topic(需在 `capabilities.events.emit`,且为 publisher 前缀)。
- 平台 topic 见 `PlatformTopic` + `PlatformTopicPayloads`:`session.*` / `message.*`(user.sent / assistant.delta / assistant.done)/ `tool.*`(invoked / completed)/ `permission.requested`。从扩展 agent 路径 emit(见 spec §5)。

### `workspace`(`workspace.ts`)
- `fs: WorkspaceFileSystem` — `readFile` / `writeFile` / `stat` / ...(`FileStat`,错误 `WorkspaceFsError`)。
- `getConfiguration(section): Configuration` + `onDidChangeConfiguration`。
- `workspaceFolders` / `WorkspaceFolder`。

> 注:平台 `fs` API 之外,你的扩展进程本就有完整 Node `fs`(信任模型)。`workspace.fs` 是带 workspace 语义的便捷层。

### `window`(`window.ts`)— UI
- `showInformationMessage / showWarningMessage / showErrorMessage(text, ...items): Promise<MessageItem | undefined>`。
- `showInputBox(InputBoxOptions)` · `showQuickPick(items, QuickPickOptions)`。
- `createOutputChannel(name, opts?): OutputChannel | LogOutputChannel` — 见下「logging」。
- `createWebviewPanel(opts): WebviewPanel` — 主动开一个 webview 面板(sidebar/tab,见 `WebviewSlot`);`panel.webview.postMessage` / `onDidReceiveMessage` 双向。
- `registerWebviewViewProvider(viewId, provider): Disposable` — 平台驱动打开的操作视图(rail → 主区/dock)。`provider.resolveWebviewView(view)` 里拿 `view.webview` 收发消息;`onDidChangeVisibility` / `onDidDispose` 生命周期。

### `secrets`(`secrets.ts`)
- `get(key)` / `store(key, value)` / `delete(key)`(macOS Keychain;Linux/Win 见 spec)。每扩展按 namespace 隔离;`SecretsError`。

### `auth`(`auth.ts`)
- `getSession(providerId, scopes, GetSessionOptions): Promise<AuthSession>` — 内置 OAuth / PKCE / device-flow 数据层。

### `extensions`(`extensions.ts`)
- `getExtension(id): Extension | undefined` + `.exports` — 跨扩展取 API(VS Code 同款)。

---

## L2 — 扩展点客户端 API

### `agents`(`agents.ts`)
- `registerProvider(id, provider: AgentProvider): Disposable` — id 须在 manifest `cronymax.agents.provider` 申报。
- `AgentProvider`:`enumerate(): Promise<ModelInfo[]>` · `createSession(SessionOptions): Promise<AgentSession>`。
- `AgentSession`:`prompt(msg): AsyncIterable<AgentEvent>` · `resolvePermission(...)` · `cancel()` · `dispose()`。
- `AgentEvent` 闭合枚举:`text` / `thinking` / `toolCall` / `toolCallUpdate` / `permissionRequest` / `done`。
- 平台把这些事件翻成聊天/flow 的流式 token、tool 卡片、权限弹窗。

### `renderers`(content renderer,**iframe 侧**)
content renderer **不走 Node host**。`entry` HTML 里(在 iframe 内)用全局:
```ts
acquireCronymaxRendererApi().activate((ctx) => ({
  renderItem(req) { /* req.content 渲染进 DOM;ctx.setHeight(px) 撑开高度 */ },
  updateItem?(req) {}, disposeItem?() {},
}));
```
- `RenderRequest`:`rendererId` · `content: string` · `complete: boolean`(流式)· `version: number`(stale drop)· `metadata { language, source, messageId }`。
- iframe 是独立 origin(`cronymax-webview://<extId>`);CSP 由平台注入,`csp.connect_src` 可在 manifest 放开。
- 类型见 `cep-idl/v1/renderer-host.ts`。

---

## logging(`logging.ts`)

```ts
const ch = cronymax.window.createOutputChannel("Acme", { log: true }); // LogOutputChannel
ch.info("started"); ch.warn("..."); ch.error("...");                   // 带 level
// 或普通 OutputChannel:ch.appendLine("...")
```
每行落进 `channels/<name>.log`(NDJSON 带时间戳),在设置面板「Logs」tab 与 stdout/stderr 合并时间排序展示。`console.log/error` 也会被 host 捕获到 output.log / host.log。

---

## primitives(`primitives.ts`)
`Disposable`(值 + 类型)· `CancellationToken` / `CancellationTokenSource` / `CancellationError` · `Event<T>` / `Listener` · `URI` · `Thenable`。

## contributions(`contributions.ts`)
`ContributionKind`(值)+ `ContributionDescriptor` / `ContributionItem` / `ContributionOwner` — 平台侧贡献目录的形状(扩展一般不直接用,UI 用)。

---

## 冻结策略

`interface Cronymax` 的形状在 v1 **不得变更,除非增量增长**(新可选字段 / 新 namespace)。Rust → TS codegen 以 `cep-idl/v1/*.ts` 为源;`sdk/extension/src/*.ts` 是其镜像,两者保持字节一致。
