# Cronymax 扩展平台 · 实施计划 v0.3

配套：[`spec-v0.3.md`](spec-v0.3.md)
状态：**待启动**；替换 plan-v0.2

---

## 0. 一句话目标

10-12 周内把 cronymax 从"硬编码 LLM provider + 内置 agent 系统"升级成 **VS Code 级开放扩展平台**：

- Node 26 subprocess 每扩展独立
- Permission Model 一条龙强制（fs / network / process / worker / addons / ffi / inspector）
- 第一个 dogfood `bytedance.coco`（ACP agent）+ 第二个 `acme.mermaid-renderer`

---

## 1. 已锁定的决策（最终）

详见 spec-v0.3 §13。**关键 12 条**：

| # | 决策 |
|---|---|
| 1 | 三层架构 L1/L1.5/L2，6 个 v1 EP 起步 |
| 2 | AgentProvider / AgentSession 命名 |
| 3 | `cronymax.*` 命名空间锁，publisher 前缀强制 |
| 4 | **Runtime：Node 26**（含完整 Permission Model）|
| 5 | **每扩展独立 Node host，lazy activate，不主动 deactivate** |
| 6 | **Capability α：纯 Node Permission Model**，平台不写包装层 / 审计 / 命令白名单 |
| 7 | Capability γ（M1）：α + OS sandbox 外层 |
| 8 | IPC：Unix socket / Named Pipe + MessagePack-RPC |
| 9 | 模块系统：CJS 主推，ESM M1 |
| 10 | 跨扩展通信：VS Code 同款 `extension.exports` |
| 11 | IDL：TS interface freeze → codegen TS/Rust |
| 12 | v1 无 marketplace；本地 .crx + 安装期授权 |

---

## 2. 通用性硬规则（每周自检）

违反任何一条 = 设计跑偏：

1. **平台核心代码不出现具体扩展名**（coco / slack / mermaid 等只在 dogfood 扩展代码里）
2. **所有 L2 EP 走同一套注册机制**（不允许 per-EP 专用 handler）
3. **任何 L1 API 必经 Node Permission**（无内置扩展豁免）
4. **AgentProvider 对 chat 面板和 flow runtime 完全对称**
5. **加内置扩展前先尝试用纯 SDK 写**，发现要改核心就停下加 EP

---

## 3. 风险登记 v0.3

| # | 风险 | 缓解 | 决策点 |
|---|---|---|---|
| R1 | Node host RAM 8 扩展 ~500MB | 接受；M1 评估 host 池 | M1 |
| R2 | 首次激活 80-200ms 延迟 | 预热常用扩展 + spinner | Phase 7 末 |
| R3 | Node 26 当前 vs LTS 时间 | Phase 0 spike 实测；评估 ship 时状态 | Phase 0 末 |
| R4 | 资源耗尽 / 死循环 | Rust 限额 + 强杀 | Phase 10 |
| R5 | L1 API 设计错事后改破坏兼容 | Phase 0 冻 IDL + peer review | Phase 0 末 |
| R6 | flow runtime 现有 agent step 耦合 | Phase 0 摸底；Phase 8 重构 | Phase 0 末 |
| R7 | 内置 LLM provider 迁移工程量 | v1 不迁 | 已决 |
| R8 | v1 无 marketplace 审核 | v1 仅本地 .crx | M1 上 marketplace |

---

## 4. 10 个 Phase（串行 ~12 周；2-3 人并行 8-10 周）

### Phase 0 · Foundation（1 周）

**目标**：拿到开工基线

| 任务 | 谁 | 工时 |
|---|---|---|
| spec-v0.3 + plan-v0.3 评审定稿 | 全员 | 90min 会 |
| IDL v1 freeze（cep-idl/v1/*.ts），AgentProvider/AgentSession/AgentEvent 完整签名 | Rust A | 2 天 |
| 摸现有 flow runtime + capability/agent_loader.rs，写 1 页 brief | Rust B | 2 天 |
| **Node 26 真机 spike**（关键）：验证 8 个 --allow-* flag 行为 + 性能 + npm 兼容 | TS 全栈 | 2 天 |
| **MessagePack-RPC spike**（Rust ↔ Node 22 ping-pong）| Rust A | 1 天 |
| 起 `crates/cronymax/src/extensions/` 骨架 + npm 仓库 `@cronymax/extension` | Rust A | 1 天 |
| Phase 0 评审 + Phase 1 任务卡 | 全员 | 60min |

**Node 26 spike 验收清单**：
- ✅ `--permission` 启动正常
- ✅ `--allow-fs-read=/path` 限定有效
- ✅ `--allow-net=host` 限定有效（之前 Node 22 缺这条）
- ✅ `--allow-child-process` 开关
- ✅ `--allow-ffi` / `--allow-inspector` 默认禁验证
- ✅ 绕路尝试（eval / process.binding / Function / dynamic import）全部拦
- ✅ 常见 npm 包（octokit / yaml / zod / date-fns / lodash）在 permission 下正常工作
- ✅ permission check overhead < 5%（已知 readFileSync ~+15μs）
- ✅ 路径 canonicalize 处理 macOS /tmp → /private/tmp 等 symlink

### Phase 1 · Manifest + Registry + Activation（2 周）

**目标**：装/卸/启用/禁用流程通

| 模块 | 内容 |
|---|---|
| `manifest.rs` | cronymax-extension.json schema 解析 + 校验（命名空间、capability、publisher 前缀）|
| `registry.rs` | 扫 `~/.cronymax/extensions/` + 启用状态持久化 |
| `activation.rs` | activationEvents 匹配引擎 |
| CLI | `cronymax ext install / list / enable / disable / uninstall` |

**验收**：
- 装一个纯声明扩展 → 列得出 → 启禁状态变 → 卸载干净
- manifest 错误明确报错
- 任何 `cronymax.*` 命名空间写入安装期拒

### Phase 2 · Node Host + L1 第一切片（2 周）

**目标**：hello-world 扩展能跑 activate；commands 能调用

| 模块 | 内容 |
|---|---|
| Node 26 打包 | macOS arm64/x64 + Win x64 + Linux x64 入 build |
| `host/node.rs` | spawn + 进程生命周期 + 重启 + 健康检查 |
| `capability.rs` | `build_node_flags(manifest)` - 极简版（~50 行）|
| `bootstrap.js` | 接 socket / 握手 / 加载 main.js / 注入 @cronymax/extension。**0 行 require 劫持代码** |
| `api/lifecycle.rs` + `api/commands.rs` | 第一切片 |
| `@cronymax/extension` SDK v0 | 暴露 lifecycle + commands |

**验收**：
- hello-world 扩展（30 行 TS）能注册 command 被命令面板调
- 故意 `fs.readFileSync('/etc/passwd')` → Node throw `ERR_ACCESS_DENIED`
- 故意 `fetch('https://evil.com')` 不在白名单 → Node throw
- 杀掉 Node host → 自动重启
- 性能基准：1000 commands.execute round-trip P99 < 5ms

### Phase 3 · 其余 L1 Kernel（2 周）

**目标**：14 个原语全开

每条含 capability gate（Node 强制）+ 单测：

- [ ] `events.rs` — pub/sub topic 路由
- [ ] `config.rs` — get/update + onDidChange
- [ ] `secrets.rs` — Keychain / DPAPI / secret-service
- [ ] `storage.rs` — global / workspace state
- [ ] `webview.rs` — stub 接口（实际渲染 Phase 6）
- [ ] `auth.rs` — OAuth / PKCE / device-flow
- [ ] `extensions.rs` — getExtension + exports

**不需要写的**：`fs.rs` / `process.rs` / `network.rs` —— Node 直接管。

**验收**：每原语 focused 测试扩展；越界全拦；secrets 跨扩展不可读。

### Phase 4 · L2 EP × 6（2 周，可并行 Phase 5/6）

**目标**：6 个起步 EP 在平台 UI 里被消费

| EP | 说明 |
|---|---|
| `cronymax.command` | Phase 2 整合 |
| `cronymax.config.schema` | 设置面板自动生成表单 |
| `cronymax.config.page` | 设置面板嵌入 webview（依赖 Phase 6）|
| **`cronymax.agents.provider`** | **核心**，详见子任务 |
| `cronymax.content.renderer` | block 类，先支持 mermaid |
| `cronymax.ui.sidebar.view` | 侧栏 webview |

**子任务 4.1 · AgentProvider**：
- IDL 完整冻结
- Rust trait `AgentProviderHandle`
- 聊天面板硬编码 LLM 选择器 → `registry.consume(...)`
- flow runtime agent step → registry 调度
- permissionRequest 事件桥接平台权限弹窗

### Phase 5 · L1.5 平台事件（1 周，可并行 Phase 4/6）

**目标**：8 条平台事件 emit；扩展能订阅

- 平台事件总线
- 聊天面板 emit `cronymax.session.*` / `cronymax.message.*`
- Tool 调度 emit `cronymax.tool.*` / `cronymax.permission.*`
- Capability `events.subscribe` 白名单校验

**验收**：写 "logger" 测试扩展订阅 `cronymax.message.assistant.done`，跑聊天日志正常追加。

### Phase 6 · Webview 基建（2 周，可并行 Phase 4/5）

**目标**：扩展 webview 能渲染、能 postMessage 跨进程通信

- CEF 协议处理器 `cronymax-webview://<ext-id>/<path>`
- iframe sandbox CSP
- `acquireCronymaxApi()` postMessage 桥
- CEF → Rust → Node host 中转链路

**验收**：ping webview 扩展（HTML 按钮 → 扩展 main.ts → 回 pong → HTML 更新）；扩展 A webview access 不到扩展 B。

### Phase 7 · `bytedance.coco` Dogfood（2 周）

**目标**：完整闭环

- coco-extension/ 项目骨架（独立 repo，可发 npm）
- manifest 按 spec §11
- `src/acp-client.ts` — 直接 `import { spawn } from "child_process"`（Node 26 ACL 兜底）
- `src/coco-session.ts` — ACP event 翻译
- `src/main.ts` — register agents.provider + commands
- `src/settings/` — 自定义设置页
- esbuild 打包 CJS, target node22+
- 端到端测试

**通用性 checkpoint**：
- coco 扩展代码**不引用任何 cronymax 内部模块**
- 可脱离 cronymax repo 单独 build

**Phase 7 末做第二个 dogfood**：`acme.mermaid-renderer` 验证非 agent 类扩展走同一套机制。

### Phase 8 · Flow Agent 系统接通（1 周，与 Phase 7 末并行）

**目标**：`.cronymax/agents/*.yaml` 引用 AgentProvider；flow runtime 调度

- `agent_loader.rs`：AgentDef 加 provider + provider_config
- `flow/agent_step.rs`：通过 registry 拿 AgentProvider，不硬编码
- `web/src/panels/flows/agents/`：list + 4 步新建向导
- 兼容：老 agent yaml 默认 `provider: native`

**验收**：UI 新建 agent 绑 coco / GPT-5.4 / plan；flow step type=agent 运行通过；allowed_tools 门控生效。

### Phase 9 · SDK + 扩展管理 UI（2 周）

**目标**：第三方在不读 cronymax 源码情况下能写扩展

- IDL → TS 类型 codegen 脚本
- `@cronymax/extension` npm 包发布
- 模板仓 `cronymax-extension-template`：4 个示例（hello-world / theme / content-renderer / agent-provider）
- 设置面板 - Extensions 标签：列表 / 安装 / 卸载 / 启用 / 禁用 / 详情 / 撤销 capability
- CLI 完善：`cronymax ext package <dir>` / `cronymax ext dev <dir> --watch`

**验收**：一个不知道 cronymax 内部实现的同事，用模板仓 + README，**半天内**能写出能用的扩展。

### Phase 10 · 收尾 + Alpha（1 周）

**目标**：稳定性、UX、文档

- Crash 恢复测试（SIGKILL → 自动重启）
- 资源额度：CPU / 内存 / 子进程数 / iframe 数限制
- Capability 弹窗 UX：人话清单 + 撤销路径
- 预热常用扩展：减少首次激活延迟
- 文档：spec / SDK API ref / 开发者指南 / 安全模型说明
- 性能基准 + 优化
- Alpha 发布

**验收**：1 小时聊天 + flow 混合负载无泄漏不崩；装/卸/启/禁流程无残留。

---

## 5. 关键路径 + 并行机会

```
Phase 0 (foundation + Node 26 spike) ← 1w
    │
    ▼
Phase 1 (manifest+registry+activation) ← 2w
    │
    ▼
Phase 2 (Node host + L1 切片) ← 2w
    │
    ├────────────┬────────────┬─────────────────┐
    ▼            ▼            ▼                 ▼
Phase 3      Phase 5      Phase 6           Phase 9 SDK 起步
(其余 L1)    (L1.5 事件)  (webview 基建)
2w           1w           2w
    │            │            │                 │
    └────────────┴────────────┴─────────────────┘
                          │
                          ▼
                  Phase 4 (L2 EPs × 6) ← 2w
                          │
                          ├─────────────────┐
                          ▼                 ▼
                  Phase 7 (coco)        Phase 8 (flow 接通)
                  2w                    1w
                          │                 │
                          └─────────────────┘
                                   │
                                   ▼
                            Phase 9 完工 (SDK + UI) ← 2w
                                   │
                                   ▼
                            Phase 10 (alpha) ← 1w
```

**关键路径**（最长链）：Phase 0 → 1 → 2 → 4 → 7 → 9 → 10 = **12 周**
**并行加速后**：Phase 3/5/6 在 Phase 2 完成后跟 Phase 4 同时跑 → **省 3-4 周** → **8-10 周**

---

## 6. 团队配置

### 最低（1.5 人）：10-12 周

| 角色 | 主要负责 |
|---|---|
| Rust 后端 | Phase 1/2/3/4/5/8 全部 + Phase 10 |
| TS 全栈 | Phase 6 webview + Phase 7 coco + Phase 9 SDK & UI |

### 推荐（3 人）：8-10 周

| 角色 | 主要负责 |
|---|---|
| Rust A | Phase 1/2/3（kernel + Node host + registry）|
| Rust B | Phase 4（L2 EPs + AgentProvider）+ Phase 5（events）+ Phase 8（flow） |
| TS 全栈 | @cronymax/extension SDK + Phase 6 webview + Phase 7 coco + Phase 9 UI |

---

## 7. 第一个月详细计划（按周）

### Week 1 — Phase 0

| 天 | 内容 |
|---|---|
| Mon | spec-v0.3 + plan-v0.3 评审会（90min）；分工 |
| Mon-Tue | IDL v1 起草（Rust A） / **Node 26 spike** 启动（TS）|
| Tue-Wed | flow 现状摸底 + brief（Rust B） |
| Wed-Thu | Node 26 spike 跑完，输出验证报告 |
| Wed-Thu | MessagePack-RPC ping-pong spike |
| Thu | IDL v1 review |
| Fri | Phase 0 评审；Phase 1 任务卡 |

### Week 2-3 — Phase 1

| 周 | 内容 |
|---|---|
| W2 | manifest.rs schema + canonicalize + 校验 |
| W3 | registry.rs + activation.rs + CLI ext 子命令 + Phase 1 验收 |

### Week 4-5 — Phase 2

| 周 | 内容 |
|---|---|
| W4 | Node 26 打包多平台 + host/node.rs spawn + capability.rs build_node_flags |
| W5 | bootstrap.js + SDK v0 + lifecycle/commands API + 性能基准 |

### Week 6-7 — Phase 3 + 5 并行

| 周 | 内容 |
|---|---|
| W6 | Rust A: events/config/secrets/storage  ·  Rust B: L1.5 events 总线 + 头 4 主题 |
| W7 | Rust A: webview stub/auth/extensions  ·  Rust B: 剩余 4 主题 + 验收 |

### Week 7-8 — Phase 6 并行

| 周 | 内容 |
|---|---|
| W7 | CEF 协议处理器 + iframe sandbox CSP |
| W8 | acquireCronymaxApi 桥 + postMessage 中转 + ping demo |

后续 Phase 4 / 7 / 8 / 9 / 10 按计划走。

---

## 8. v1 Alpha 验收清单

完整跑通这一组场景就算 alpha：

- [ ] 装 `bytedance.coco` 扩展，授权对话框显示人话清单
- [ ] 聊天面板 agent picker 列出 Coco
- [ ] 选 Coco / GPT-5.4 / plan，开会话发消息
- [ ] agent 输出流式渲染、tool call 卡片显示、permission 弹窗
- [ ] 设置面板能改 coco.binaryPath，自定义设置页能开
- [ ] flows panel - Agents 标签建 agent code-reviewer 绑 coco
- [ ] 写 flow step type=agent agent=code-reviewer 运行通过
- [ ] 装 `acme.mermaid-renderer`，agent 输出 mermaid 块自动渲染图
- [ ] 杀掉 Node host → 自动重启
- [ ] 卸载扩展 → 设置/状态/子进程全清理
- [ ] 安全验证：扩展尝试 `fs.readFile('/etc/passwd')` 被 Node `ERR_ACCESS_DENIED` 拦
- [ ] 安全验证：扩展尝试 `fetch('https://evil.com')` 不在白名单 → 拦
- [ ] 安全验证：扩展尝试加载 .so / native addon → 拦
- [ ] 安全验证：扩展尝试 `eval("require('fs')...")` → 拦

---

## 9. v1 不做（明确推 M1+）

- Marketplace + 数字签名
- OS sandbox（γ 阶段）
- 远程开发模式
- 共享 Node host / host 池
- Process runtime（非 JS 扩展）
- 正式 L3 服务 registry
- WASM runtime
- 其余 L2 EPs（keybinding / menu / activitybar / statusbar / fs-provider / chat.tool / auth.provider）
- inline content renderers
- 内置 LLM provider 迁移到扩展模型
- ESM 主推
- 跨扩展依赖图 UI
- `--snapshot-blob` 启动优化

---

## 10. Phase 0 末 DRI 决策（必须签字）

1. ✅ Node 版本：**Node 26**
2. ✅ MessagePack 实现：`@msgpack/msgpack`（JS）+ `rmp-serde`（Rust）
3. ✅ CLI 实现位置：扩在现有 cronymax CLI 上
4. ⏳ SDK npm 发布渠道：内网 npm vs GitHub Packages
5. ⏳ 测试 coco binary：用户单独装通过 PATH 找（推荐）vs cronymax 内置
6. ⏳ Phase 7 末是否发内部 alpha：能拿早期反馈但接口未稳
7. ⏳ Node 26 ship 时间：current vs LTS（取决于 ship 日期）

---

## 11. 现有 / 待产出文件清单

### 已有

```
docs/extensions/
├── spec-v0.3.md                       ← 完整设计文档（当前）
├── implementation-plan-v0.3.md        ← 实施计划（当前）
├── spec-v0.2.md / plan-v0.2.md       ← 旧版本，保留对照
└── spec-v0.1.md / plan-v0.1.md       ← 旧版本，保留对照

diagrams/                              ← 已渲染画板源码
├── 2026-05-18T225000/                 整体架构
├── layers/                            三层模型
├── lifecycle/                         生命周期
├── flows-panel/                       Flows Agents UI
└── wizard/                            4 步向导
```

### Phase 0 要产出

```
docs/extensions/
├── ep-schemas/                        ← 每个 L2 EP 的 schema 规范
│   ├── agents-provider.md
│   ├── content-renderer.md
│   ├── config-schema.md
│   ├── config-page.md
│   ├── command.md
│   └── sidebar-view.md
├── legacy-agent-step.md               ← 现有 flow runtime brief
├── node26-permission-spike.md         ← Node 26 spike 报告
└── msgpack-rpc-spike.md               ← IPC spike 报告

crates/cronymax/src/extensions/
└── cep-idl/v1/                        ← IDL v1 freeze
    ├── lifecycle.ts
    ├── commands.ts
    ├── events.ts
    ├── workspace.ts
    ├── window.ts
    ├── secrets.ts
    ├── auth.ts
    ├── extensions.ts
    ├── agents.ts                      ← AgentProvider / AgentSession / AgentEvent
    └── renderers.ts
```

---

## 12. 通用性自检（每周五五分钟）

每周回顾，任何 NO 下周第一件事修：

1. 这周代码里有没有出现 "coco" / "mermaid" / "slack" 字眼在 `crates/cronymax/src/extensions/`？
2. 这周新加的 L2 EP 有没有专属 handler 文件？（应走通用注册中心）
3. 这周有没有为某个特定扩展开 capability 后门？
4. chat 面板代码和 flow runtime 代码调 AgentProvider 是不是用同一份接口？
5. 这周新增能力，如果让外部第三方写扩展实现，能不能不改核心？

---

## 13. 跟 v0.2 的差异（含工程量影响）

| 项 | v0.2 | v0.3 | 工程量差 |
|---|---|---|---|
| Node 版本 | 22 LTS | **26** | 0 |
| 网络 ACL | 软门控 | **Node 强制 --allow-net** | -3d（无包装层）|
| child_process per-command | 平台 wrap + argsPattern | **砍** | -3d |
| 审计 hook | 写 spawn 日志 | **砍** | -0.5d |
| ffi / inspector / addons | 未控 | **Node 默认禁** | 0（白送）|
| manifest capabilities | `process: {allow: [...]}` | **`process: true`** | -1d schema |
| α 安全实施总量 | ~5 天 | **~2 天** | -3 天 |
| 总 v1 alpha 估时 | 12-14 周 | **10-12 周** | -2 周 |

---

文档版本：v0.3 · 2026-05-19
DRI：待定
评审：待
