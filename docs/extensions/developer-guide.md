# Cronymax 扩展开发者指南

面向想为 cronymax 写扩展的开发者。配套:[`sdk-api-reference.md`](sdk-api-reference.md)(API 速查)· [`trust-model.md`](trust-model.md)(信任 / 安全模型)· [`spec-v0.3.md`](spec-v0.3.md)(平台设计)。

> v1 alpha。`@cronymax/extension` 的 IDL 已 freeze,只增不改(新增可选字段 / 新 namespace)。

---

## 1. 扩展是什么

一个 cronymax 扩展是一个目录,根部有 `cronymax-extension.json`(manifest),可选一个 Node 入口 `main`。扩展运行在**每扩展独立的 Node 26 子进程**里(host),通过 fd 3 上的 MessagePack-RPC 与平台通信。SDK(`@cronymax/extension`)是一层薄壳:真正的实现在 cronymax 主进程,SDK 把调用转成 RPC。

两种形态:

- **host-backed**:manifest 声明了 `main`。平台为它 spawn 一个 Node host,跑你的 `activate(ctx)`。命令、agent provider、侧栏视图的双向消息都需要 host。
- **declarative-only**:没有 `main`。平台不 spawn 任何进程,只读 manifest 里的贡献。content renderer(iframe 渲染)和纯静态侧栏视图可以这样做——零 Node 开销。

信任模型:**安装即信任作者**。扩展进程有完整的 Node 能力(fs / network / child_process / 原生模块全开),不沙箱。详见 [`trust-model.md`](trust-model.md)。

---

## 2. Quickstart

最小目录结构:

```
my-ext/
  cronymax-extension.json
  package.json
  tsconfig.json
  src/main.ts
  dist/main.js        # 构建产物
```

**`cronymax-extension.json`**:

```json
{
  "id": "acme.hello",
  "name": "Hello",
  "version": "0.1.0",
  "publisher": "acme",
  "engines": { "cronymax": "^1.0" },
  "main": "dist/main.js",
  "activationEvents": ["onCommand:acme.hello.greet"],
  "contributes": {
    "cronymax.command": [
      { "id": "acme.hello.greet", "title": "Hello: Greet" }
    ]
  }
}
```

- `id` 必须是 `<publisher>.<name>`,且第一段等于 `publisher`。`publisher` 为 `cronymax` 的命名空间被保留(平台占用)。
- `engines.cronymax` 是 semver range。

**`package.json`**(用 SDK 的本地包引用;v1 alpha 期 SDK 还没发到公网 npm):

```json
{
  "name": "acme-hello",
  "private": true,
  "devDependencies": { "@cronymax/extension": "file:../cronymax/sdk/extension" },
  "scripts": { "build": "esbuild src/main.ts --bundle --platform=node --format=cjs --outfile=dist/main.js" }
}
```

**`src/main.ts`**:

```ts
import * as cronymax from "@cronymax/extension";

export async function activate(ctx: cronymax.ExtensionContext) {
  ctx.subscriptions.push(
    cronymax.commands.register("acme.hello.greet", async () => {
      await cronymax.window.showInformationMessage("Hello from acme.hello!");
    }),
  );
}

export function deactivate() {}
```

`activate` 是必需的命名导出;`deactivate` 可选。`ctx.subscriptions` 里 push 的 `Disposable` 会在停用时自动 dispose——把所有注册都挂上去。

---

## 3. 激活事件(`activationEvents`)

控制扩展**何时**被激活(spawn host + 跑 `activate`):

| 事件 | 含义 |
|---|---|
| `onStartup` | cronymax 启动时 |
| `onCommand:<id>` | 该命令首次被调用时 |
| `onAgentProvider:<id>` | 该 agent provider 首次被选用时 |
| `onView:<id>` | 该侧栏视图首次打开时 |
| `*` | 立即(谨慎用——总开销) |

> v1 alpha:启用的扩展实际在启动时被 eager 激活(见 spec)。`activationEvents` 仍是必填且语义正确,后续会按事件懒激活。

---

## 4. 六个扩展点

都在 manifest 的 `contributes` 下声明。host-backed 的(命令 / agent / sidebar 双向)还要在 `activate` 里 register。

### `cronymax.command`
```json
{ "cronymax.command": [{ "id": "acme.x.run", "title": "Acme: Run", "category": "Acme" }] }
```
```ts
ctx.subscriptions.push(cronymax.commands.register("acme.x.run", () => { /* ... */ }));
```

### `cronymax.agents.provider`(聊天 / flow 的 LLM/agent 引擎)
```json
{ "cronymax.agents.provider": [
  { "id": "acme.coco", "label": "Coco", "supportsModels": true }
] }
```
```ts
cronymax.agents.registerProvider("acme.coco", {
  enumerate: async () => [{ id: "coco-1", label: "Coco v1" }],
  createSession: async (opts) => ({
    id: "s1",
    prompt: async function* (msg) { yield { kind: "text", text: "hi" }; },
    dispose: () => {},
  }),
});
```
用户在聊天顶部的 model 列表或在「agent 管理」里建命名 agent 绑定它。

### `cronymax.content.renderer`(把消息里的代码块渲染成富内容,**declarative**)
```json
{ "cronymax.content.renderer": [
  { "id": "acme.mermaid", "mimeTypes": ["text/vnd.mermaid"], "entry": "./renderer/index.html",
    "csp": { "connect_src": [] } }
] }
```
renderer 是 iframe 托管的,**不走 Node host**——`entry` HTML 里用 `acquireCronymaxRendererApi()`(见 API 参考)。可无 `main`。参考 `examples/mermaid-renderer`。

### `cronymax.ui.sidebar.view`(活动栏 / 主区 / 右 dock 的 webview 面板)
```json
{ "cronymax.ui.sidebar.view": [
  { "id": "acme.panel", "title": "Acme", "icon": "./icon.svg", "entry": "./view/index.html", "target": "main" }
] }
```
`target`: `"main"`(替换主内容区)或 `"right"`(右侧 dock)。要双向消息,在 host 里用 `window.registerWebviewViewProvider`(参考 `examples/view-messaging`);纯静态可 declarative(参考 `examples/panel-explorer`)。

### `cronymax.config.schema`(从 JSON Schema 自动生成设置表单)
```json
{ "cronymax.config.schema": { "title": "Acme", "properties": {
  "acme.endpoint": { "type": "string", "default": "https://..." } } } }
```

### `cronymax.config.page`(自定义设置页,嵌入扩展 webview)
```json
{ "cronymax.config.page": [{ "id": "acme.settings", "title": "Acme Settings", "entry": "./settings/index.html" }] }
```

---

## 5. 构建、安装、调试

### 构建
任意打包器,产出 CJS。esbuild 例:
```
esbuild src/main.ts --bundle --platform=node --format=cjs --outfile=dist/main.js
```

### 安装
- **目录直装**:`cronymax ext install ./my-ext`
- **打包再装**:`cronymax ext package ./my-ext -o my-ext.cmx`,然后 `cronymax ext install my-ext.cmx`
  - `.cmx` = 纯 ZIP(根部直接放 `cronymax-extension.json` + `dist/` 等),自动排除 `.git`/`node_modules`/`.DS_Store`。
- **设置面板**:Extensions tab → 「Install…」选目录或 `.cmx`。

管理:`cronymax ext list / enable / disable / uninstall`,或设置面板 Extensions tab。

### 开发循环(热重载)
```
cronymax ext dev ./my-ext --watch
```
在一次性临时 registry 里装 + 激活,实时把扩展日志打到终端;改文件自动重启 host。不污染你真实的 `~/.cronymax`。

### 日志与排错
- 扩展里用 `cronymax.window.createOutputChannel("Acme")` → `channel.info("...")`;`console.log/error` 也会被捕获。
- 看日志:设置面板 → 该扩展 → 「Logs」(合并 stdout/stderr + 所有 channel,时间排序,可按 level / 时段过滤),「Open folder」打开日志目录。
- 打包一份诊断:`cronymax diagnostic-bundle`(收集所有 session 日志 + 各 manifest + 版本,脱敏后产 zip)。

---

## 6. 崩溃恢复(你需要知道的行为)

平台为每个 host-backed 扩展独立监督进程(P10-T01):

- 你的 host **崩溃 / 卡死**(进程退出 / `$/ping` 超时)→ 平台**自动重启**它,带退避(立即 → 500ms → 2s)。重启会重新跑你的 `activate`。
- 连续崩溃**超过 3 次**→ 平台**停用**该扩展并弹一条 toast「disabled after repeated crashes」。设置面板里它显示为 **Disabled (crashed)**,点 **Enable** 即可重试(crash 计数清零)。
- 内存超阈值(默认 ~1.5 GB)→ 平台**只告警**(日志 + 一次 toast),**不杀**进程。
- 一个扩展崩溃**不影响**其他扩展或主程序(per-extension 进程隔离)。

含义:`activate` 要可重入(重启会再跑一遍);把启动副作用写成幂等的。

---

## 7. 参考示例(`examples/`)

| 示例 | 扩展点 | 形态 |
|---|---|---|
| `hello-world` | command | host-backed,最小 |
| `echo-agent` | agents.provider | host-backed,带日志 |
| `mermaid-renderer` | content.renderer | declarative(无 main) |
| `panel-explorer` | sidebar.view ×2(main + right) | declarative |
| `view-messaging` | sidebar.view(双向消息) | host-backed,`registerWebviewViewProvider` |

类型与精确签名以 `sdk/extension/src/*.ts`(IDL 镜像)为准。
