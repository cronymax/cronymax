# IDL v1 · AI Self-Review

> Phase 0 评议 §0.1 决议：单人项目 IDL review 政策 = AI 协助 self-review + 决策理由文档化（替代"2 人 +1 review"硬指标）。本文件是这一政策的第一版输出。

- 范围：`crates/cronymax/src/extensions/cep-idl/v1/` 14 个 .ts 文件
- 对照：`spec-v0.3.md`（含 Phase 0 评议后所有修订）、`extension-logs.md` v0.2、`phase-0-review.md`
- 验证工具：`npm run check` (tsc strict + isolatedModules + noUncheckedIndexedAccess) ✅
- 日期：2026-05-20

---

## 0. 一句话结论

**14 个 IDL 文件 strict 编译通过；覆盖 spec §2/3/4/6/8/11 所有需要 SDK 暴露的接口；存在 6 处有意识缺口（spec 说明 "Node 直接管"，不走 IDL）；2 处需在 Phase 2 实现期跟进**。无 blocker，进 Phase 1 可继续 IDL 增量演化。

---

## 1. 覆盖矩阵（spec → IDL）

| spec 节 / 主题 | 涉及 SDK 表面 | IDL 文件 | 状态 |
|---|---|---|---|
| §2 L1 Kernel · lifecycle | `activate / deactivate / ExtensionContext` | `lifecycle.ts` | ✅ |
| §2 L1 · commands | `register / execute / list` | `commands.ts` | ✅ |
| §2 L1 · events | `on / emit` + topics | `events.ts` | ✅ |
| §2 L1 · workspace.fs / config | `rootUri / fs / getConfiguration / onDidChangeConfiguration` | `workspace.ts` | ✅ |
| §2 L1 · window | messages / inputBox / quickPick / createWebviewPanel / openConfigPage / openExternal + **createOutputChannel** | `window.ts` + `logging.ts` | ✅（含评议加 createOutputChannel） |
| §2 L1 · secrets | get / set / delete + onDidChange | `secrets.ts` | ✅ |
| §2 L1 · auth | getSession / removeSession + onDidChangeSessions | `auth.ts` | ✅ |
| §2 L1 · extensions | getExtension / all + Extension.exports | `extensions.ts` | ✅ |
| §2 L1 · env | appName / platform / machineId / homedir | `index.ts` (EnvNamespace) | ✅ |
| §2 L1 · `process` 子进程 | （无）— **Node 标准 API 直用** | n/a | ✅ 有意识缺口（§4） |
| §2 L1 · `network` fetch/ws | （无）— **Node 标准 API 直用** | n/a | ✅ 有意识缺口（§4） |
| §2 L1 · `fs` 真文件读写 | （无）— **Node 标准 API 直用**（workspace.fs 是 URI 抽象层）| n/a | ✅ 有意识缺口（§4） |
| §3 L1.5 平台事件 8 条 | 8 个 topic 常量 + 8 个 payload 类型 + topic → payload 映射 | `events.ts` (`PlatformTopic` + `PlatformTopicPayloads`) | ✅ |
| §4 L2 EP · `cronymax.command` | CommandContribution | `manifest.ts` | ✅ |
| §4 L2 EP · `cronymax.config.schema` | ConfigSchemaContribution + JsonSchema | `manifest.ts` | ✅ |
| §4 L2 EP · `cronymax.config.page` | ConfigPageContribution | `manifest.ts` | ✅ |
| §4 L2 EP · `cronymax.agents.provider` | AgentProviderContribution + AgentProvider/AgentSession/AgentEvent/ModelInfo/ModeInfo/SessionOptions/PromptMessage/PromptAttachment/McpServerSpec/PermissionDecision + agents.registerProvider/getProvider | `manifest.ts` + `agents.ts` | ✅ |
| §4 L2 EP · `cronymax.content.renderer` | ContentRendererContribution + Renderers.registerRenderer + RenderRequest/RenderHandle/RenderHandler | `manifest.ts` + `renderers.ts` | ✅ |
| §4 L2 EP · `cronymax.ui.sidebar.view` | SidebarViewContribution | `manifest.ts` | ✅ |
| §6 安全模型 · capabilities | FsCapability(`[{path, mode}]` 数组 + 平台变量)、NetworkCapability、process / workers / native_addons booleans、secrets、events.subscribe/emit、ui-slots、extension-points、auth.providers | `manifest.ts` | ✅（含评议 §D 重设） |
| §8 manifest schema | Manifest 主结构 | `manifest.ts` | ✅ |
| §11 coco walkthrough（all imports）| `agents.registerProvider` / `commands.register` / `workspace.rootUri` / `workspace.getConfiguration` / `window.openConfigPage` | 多文件 | ✅ |
| `extension-logs.md` · 日志系统 | OutputChannel / LogOutputChannel / LogLevel / CreateOutputChannelOptions | `logging.ts`（评议新增） | ✅ |

---

## 2. 有意识缺口（spec 提到但 IDL 不写）

| 缺口 | 理由 | spec 引用 |
|---|---|---|
| `cronymax.process.spawn(opts)` API | 决策 4d "纯 Node Permission Model"：扩展直接 `import { spawn } from "child_process"`；ACL 由 `--allow-child-process` 强制 | §2 表 + §11 acp-client.ts 注释 |
| `cronymax.network.fetch / websocket` API | 同上：扩展用全局 `fetch` / Node `node:net` | §2 表 |
| `cronymax.fs.*` 真文件 API | 同上：扩展用 `node:fs/promises`；ACL `--allow-fs-*` 强制 | §2 表 |
| `cronymax.chat.*` 命名空间 | spec §2 提了但表里没绑 EP；目前 chat 面板交互**全靠 agents.provider 间接**完成（chat panel 调 registry.consume 路由到扩展的 createSession）；独立 `cronymax.chat.*` 推 M1 评估 | §2 表 |
| `cronymax.env.user` 等身份 API | 推 M1 | — |
| 进度条 / status bar item API | M1 EP（spec §4 列在 M1） | §4 |
| 跨扩展正式契约 schema | spec §13 决策 8："VS Code 同款 extension.exports，无 schema / semver" | §13 |

---

## 3. 已修复的不一致（评议后 patch）

| 问题 | 修复 |
|---|---|
| `manifest.ts` FsCapability 是 `{scope: "workspace", mode}` 单对象 | ✅ 改成 `[{path: string, mode}]` 数组 + 平台变量；详 §6.1.2 |
| `manifest.ts` NetworkCapability 注释暗示 Node `--allow-net=host` enforce | ✅ 注释改"v1 informational only" |
| `window.ts` 没有 createOutputChannel | ✅ 加 overloaded 签名 `createOutputChannel(name)` + `createOutputChannel(name, {log: true})` |
| `logging.ts` 不存在 | ✅ 新增；LogLevel / OutputChannel / LogOutputChannel / CreateOutputChannelOptions |
| `index.ts` 没 re-export logging 类型 | ✅ 加 `export type { LogLevel, OutputChannel, LogOutputChannel, CreateOutputChannelOptions }` |

---

## 4. 待 Phase 2/3 实现期跟进

| # | 项 | 跟进 |
|---|---|---|
| 1 | `agents.ts` 的 `AgentEvent.toolCallUpdate` 当前定义 `status: "completed" \| "failed"`，缺 `"cancelled"`——`legacy-agent-step.md` 指出当前 `LlmEvent` 也没对等 cancel 状态 | Phase 8 改造 flow agent step 时统一加 `"cancelled"`；v1 freeze 政策允许"增加新 enum 值"作 additive growth |
| 2 | `lifecycle.ts` `ExtensionContext.extensionMode` 暴露 "production" / "development" / "test"，但 spec 没明确语义；当前推 VS Code parity，留 Phase 2 实现期定义清楚 cronymax 何时给哪个值 | Phase 2 P2-T06 实现时落 |
| 3 | `events.ts` `PlatformTopicPayloads` 索引签名跟 `PlatformTopic` 常量没 type-level 绑死；只在 doc-level | 可加 mapped type；M1 |
| 4 | `manifest.ts` JsonSchema 是 draft-7 子集——v1 设置面板渲染哪些字段需 P9-T04 实现期细化 | Phase 9 落 |
| 5 | `agents.ts` `PromptAttachment` 只覆盖 file/image/blob；可能需要 `audio` / `video` / `tool-result` | M1 当真出现需求时加 enum 值 |
| 6 | `window.ts` `WebviewPanel.setHtml` 与 `entry: path` 两种装配模式并存——建议 Phase 6 实现期确认是否两条都要，还是 entry 唯一 | Phase 6 P6-T03 落 |

---

## 5. 没找到（明确 negative）

走完所有文件未发现以下：

- ❌ 命名冲突（每个 interface 在自身文件内唯一）
- ❌ 循环 import（依赖图 primitives → 其他；index 顶层）
- ❌ `any` 类型逃逸（全 `unknown` / typed）
- ❌ 隐式 `null` / `undefined` 处理（`strictNullChecks` + `noUncheckedIndexedAccess` 全开）
- ❌ 跟 spec 矛盾的接口签名（除 §3 已修复的几处）
- ❌ 测试代码混入（IDL 纯类型，无 runtime 代码）
- ❌ 已弃用接口残留（v1 IDL 是首版）

---

## 6. 风险

| # | 风险 | 处理 |
|---|---|---|
| R1 | AI self-review 不如 2 人 +1 严谨——可能漏 type-level 微妙问题 | Phase 1/2 实现期会暴露大部分；接受 |
| R2 | `extension-logs.md` 的 LogOutputChannel.error(message: string \| Error) 联合类型在跨进程 RPC 序列化时需统一编码 | Phase 2 RPC 实现时定 Error 序列化格式（栈 + name + message + cause） |
| R3 | `agents.ts` AgentEvent 的 `toolCall.input: unknown` / `toolCallUpdate.output: unknown`——跨进程序列化容忍度高但调用方运行时需自己 type check | 文档警告；M1 评估带 schema 的 toolCall variant |
| R4 | `manifest.ts` 平台变量名是字符串（`"{WORKSPACE}"`），TS 不能 type-level 限制——manifest 校验阶段 Rust 端必校 | Phase 1 P1-T02 manifest 校验单测覆盖 |

---

## 7. 进 Phase 1 的判定

**进**。

理由：
1. 全部 14 文件 tsc strict 通过
2. 覆盖 spec 所有需要 SDK 暴露的接口
3. 缺口都是有意识的（"Node 直接管"原则）或推到 M1（明确 list）
4. 评议后的 5 处修订全 patch 进入；包括 logging.ts / manifest fs schema / createOutputChannel
5. 6 处 Phase 2/3 跟进项不阻塞 Phase 1（Phase 1 主要做 manifest 校验 / registry / activation 引擎；IDL 改动等 Phase 2 才碰）

下次 review 触发：Phase 2 末，IDL 增量稳定前。

---

**Reviewer**：Claude (AI self-review per Phase 0 评议 §0.1 政策)
**审议范围**：14 IDL 文件 + 配套 tsconfig + README + spec § 2/3/4/6/8/11 + 评议纪要
**审议方式**：文件结构遍历 + spec 节-IDL 文件交叉对照 + tsc 编译检查 + 类型签名一致性检查
