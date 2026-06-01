# Cronymax 扩展平台 · 实施计划 v0.1

配套：[`spec-v0.1.md`](spec-v0.1.md)
状态：内部评审 / 待启动

---

## 0. 已锁定的决策

| # | 决策 | 选择 |
|---|---|---|
| 1 | 三层架构 | L1 Kernel（封闭 14 原语）+ L1.5 平台事件 + L2 Extension Points |
| 2 | 命名 | `AgentProvider` / `AgentSession`（不绑 chat 表面）；L2 EP = `cronymax.agents.provider` |
| 3 | 命名空间锁定 | `cronymax.*` reserved；第三方必须用 publisher 前缀 |
| 4 | 主要 runtime（v1） | **Node.js subprocess 作 Extension Host**（先搁置 CEF + V8 方案，v2 再切） |
| 5 | API 真相之源 | TypeScript interface 当 IDL，codegen 出 SDK / RPC schema |
| 6 | 二进制 RPC 编码 | MessagePack-RPC（跨语言、库现成） |
| 7 | 清单文件名 | `cronymax-extension.json` |
| 8 | 跨扩展通信 | VS Code 同款 `extension.exports`（v1 不做正式契约 registry） |
| 9 | UI 沙箱 | Webview = sandbox iframe，独立 origin |
| 10 | v1 L2 EP 起步数 | 6 个：command / config.schema / config.page / agents.provider / content.renderer / ui.sidebar.view |
| 11 | 内容渲染器 | block + inline 两类；v1 含 block，inline 推 M1 |
| 12 | 共享 V8 模式 | v1 用 Node subprocess 共享 host；不受信扩展独立 host 推 M1 |
| 13 | Capability 模型 | 全白名单，命名空间锁定，安装期人话授权 |

---

## 1. 通用性硬规则（每周自检一次）

**所有阶段的实现都必须满足这五条**，违反任何一条 = 设计跑偏：

1. **平台不知道任何具体扩展类型的语义**。"Coco"、"Slack"、"Mermaid" 这些字眼**只**出现在内置 dogfood 扩展代码里，不出现在 `crates/cronymax/src/extensions/` 任何文件
2. **L2 EP 走同一套注册机制**。聊天面板消费 `cronymax.agents.provider` 的代码模板，和设置面板消费 `cronymax.config.page` 的代码模板**必须等价**。不允许 per-EP 专用 handler 模块
3. **任何 L1 API 调用都过 Capability Gate**。安装期未声明的 capability，运行时直接 throw，没有"内置扩展豁免"快速路径
4. **AgentProvider 接口对 chat 面板和 flow runtime 完全对称**。两个消费方使用同一份 createSession / prompt 调用，AgentProvider impl 不需要知道自己在哪个上下文里被调
5. **每加一个内置扩展，先尝试用纯 SDK 写一遍**。若发现必须改核心代码，停下来加 L2 EP 或事件主题，**不要给内置扩展开后门**

---

## 2. 风险登记

| 风险 | 影响 | 概率 | 缓解 | 决策节点 |
|---|---|---|---|---|
| R1 · Node host ↔ Rust core RPC 性能不够 | 聊天流式响应卡顿 | 中 | Phase 2 内置基准测试；批量 + async-first；必要时改 Unix socket binary frame | Phase 2 末 |
| R2 · L1 API surface 设计错（事后改破坏兼容） | 生态崩 | 高 | Phase 0 写完整 IDL + peer review；先 freeze IDL 再写实现 | Phase 0 末 |
| R3 · CEF 渲染器 ↔ Node host 鉴权链路复杂 | webview 调不通 API | 中 | 复用现有 CEF browser/renderer 桥，Phase 6 spike | Phase 6 起步 |
| R4 · Capability 弹窗惹用户烦 | 用户关闭授权 / 装不动扩展 | 中 | Phase 10 专项打磨；v1 阶段先 stub | Phase 10 |
| R5 · Flow runtime 当前 agent step 实现耦合内置 provider | 改造范围扩散 | 中 | Phase 0 摸清楚现有代码，Phase 8 重构 | Phase 0 末 |
| R6 · ACP 协议演化破坏 coco 扩展 | dogfood 不稳定 | 低 | coco 扩展锁版本依赖，独立可升级 | 持续 |
| R7 · Extension Host 单点故障 | 一个扩展崩了影响全部 | 中 | v1 自动重启 N 次；M1 加独立 host 选项 | Phase 10 |
| R8 · 内置 llm provider 迁移到扩展模型 | 工程量爆 | 高 | **v1 不迁移**。Anthropic/OpenAI 等留在 core 走旧路径，新 agent 走 AgentProvider；M1 再渐进迁移 | 已决 |

---

## 3. 阶段分解（串行 ~18 周，2-3 人并行 10-13 周）

### Phase 0 · Foundation（1 周）

**目标**：拿到能开工的设计基线和工程脚手架

**任务**：
- [ ] spec-v0.1 + plan-v0.1 内部评审定稿
- [ ] 写 IDL v1（`crates/cronymax/src/extensions/cep-idl/v1/*.ts`）—— 接口先冻结，实现后跟
- [ ] 摸清现有 flow runtime 的 agent step 实现（R5 缓解）
- [ ] Node host 选型 spike：决定 Node 版本、bundler、IPC framing
- [ ] 起 `crates/cronymax/src/extensions/` 模块骨架（空文件 + 模块组织）
- [ ] 起 npm 仓库 `@cronymax/extension`（占位）

**验收**：
- IDL v1 freeze，签字
- 现有 flow agent step 代码路径有 1 页 brief
- `cargo build` 通过（空模块）

**风险节点**：R2 在这一阶段 freeze

### Phase 1 · Manifest + Registry + Activation（2 周）

**目标**：装/卸/启用/禁用流程通；激活事件能匹配

**任务**：
- [ ] `manifest.rs` —— 清单 schema 解析 + 校验（命名空间、capability 一致性、publisher 前缀）
- [ ] `registry.rs` —— 扫 `~/.cronymax/extensions/` + 持久化启用状态
- [ ] `activation.rs` —— activationEvents 匹配引擎（onCommand / onAgentProvider / onView / onStartup）
- [ ] CLI 命令：`cronymax ext install <path>` / `list` / `enable` / `disable` / `uninstall`
- [ ] **零扩展实际执行**，只是元数据管理

**验收**：
- 装一个 0 代码的纯声明扩展（只有 manifest），CLI 列得出来
- 禁用后 list 状态改变
- 卸载后从盘上消失
- manifest 校验错误能给出明确报错

### Phase 2 · Node Extension Host + L1 Kernel 第一切片（2-3 周）

**目标**："Hello world" 扩展能跑 activate；commands 能注册 + 调用

**任务**：
- [ ] 写 `cronymax-extension-host.js`（Node subprocess）
  - 加载所有启用扩展的 main.js
  - 跑各扩展的 activate(ctx)
  - 通过 Unix socket 跟 Rust core 通信（MessagePack-RPC）
- [ ] Rust 端 `extensions/host/node.rs` —— spawn + IPC + 健康检查
- [ ] `extensions/api/commands.rs` + `extensions/api/lifecycle.rs`
- [ ] `extensions/capability.rs` —— 基础门控（stub UX，能拒就行）
- [ ] @cronymax/extension SDK 第一版（只导出 commands + lifecycle）

**验收**：
- 写一个 hello-world 扩展（30 行 TS）
  - manifest contributes 一个 command
  - activate 里 register 这个 command 的 handler
- 平台 `cronymax ext run hello.greet --args '["world"]'` 能输出 "hello, world"
- 杀掉 Node host，平台自动重启
- 不申报 capability 的扩展，能注册 command；调底层 process API 时 throw

**Phase 2 末跑性能基准**：1000 次 commands.execute round-trip，看 P99。决策 R1。

### Phase 3 · 其余 L1 Kernel（2 周）

**目标**：14 个原语全开

**任务**（每条都含 capability gate + 单测）：
- [ ] `events.rs` —— 基础 pub/sub（先不接平台主题，下阶段）
- [ ] `config.rs` —— get/update + onDidChange
- [ ] `secrets.rs` —— 接 macOS Keychain / Windows DPAPI
- [ ] `storage.rs` —— global / workspace state
- [ ] `process.rs` —— spawn 含 argsPattern 校验
- [ ] `fs.rs` —— workspace scope 锁定
- [ ] `network.rs` —— host 白名单 fetch + websocket
- [ ] `webview.rs` —— stub 接口（实际渲染推 Phase 6）
- [ ] `auth.rs` —— 内置 oauth-generic / pkce / device-flow
- [ ] `extensions.rs` —— getExtension + exports

**验收**：
- 每个原语有 1 个 focused 测试扩展
- capability 白名单违反全部能拦下
- secrets 跨扩展不可读

### Phase 4 · L2 EP × 6（2 周，可并行 Phase 5/6）

**目标**：6 个起步 EP 在平台 UI 里被消费

**任务**：
- [ ] `contributions/` 框架：通用注册中心 + 通用 consumer hook（满足规则 #2）
- [ ] `cronymax.command`（已在 Phase 2 落地，整合到框架）
- [ ] `cronymax.config.schema` —— 设置面板从清单生成表单
- [ ] `cronymax.config.page` —— 设置面板嵌入 webview iframe（依赖 Phase 6）
- [ ] `cronymax.agents.provider` —— **本计划的核心**，详见 §3.4 子任务
- [ ] `cronymax.content.renderer` —— block 类，先支持一个 MIME（用 mermaid 试）
- [ ] `cronymax.ui.sidebar.view` —— 侧栏 webview 注册

**子任务 3.4 · AgentProvider**：
- [ ] IDL: `AgentProvider` / `AgentSession` / `AgentEvent` 完整冻结
- [ ] Rust trait `AgentProviderHandle`（对 Rust 内部使用方暴露）
- [ ] 聊天面板：替换硬编码 LLM provider 选择器为 `registry.consume("cronymax.agents.provider")`
- [ ] Flow runtime：agent step 改为通过 agents registry 调度
- [ ] permission_request 事件桥接平台权限弹窗

**通用性 checkpoint**：
- 在 Phase 4 末，能否用纯 SDK 写一个 "echo theme" 扩展（声明 `cronymax.ui.theme`）而不动核心？
  - 如果"theme" EP 还不在 v1 范围内 → OK
  - 但 6 个 v1 EP 的**任何一个**新增需求都必须能纯 SDK 完成

### Phase 5 · L1.5 平台事件主题（1 周，可并行 Phase 4/6）

**目标**：8 条平台事件能 emit；扩展能订阅

**任务**：
- [ ] `events.rs` 接平台事件总线
- [ ] 聊天面板代码加 emit：`cronymax.session.*` / `cronymax.message.*`
- [ ] Tool 调度器加 emit：`cronymax.tool.*` / `cronymax.permission.*`
- [ ] Flow runtime 加 emit：`cronymax.flow.*`
- [ ] Config 系统加 emit：`cronymax.config.changed`
- [ ] Capability `events.subscribe` 白名单校验

**验收**：
- 写一个 "logger" 测试扩展，订阅 `cronymax.message.assistant.done`，把 fullText 写到 ~/.cronymax/logs/agent-messages.jsonl
- 跑一次聊天，日志文件追加正确

### Phase 6 · Webview 基建（2 周，可并行 Phase 4/5）

**目标**：扩展 webview 能渲染、能 postMessage 跟扩展 main.js 通信

**任务**：
- [ ] CEF 协议处理器：`cronymax-webview://<ext-id>/<path>` 路由到 `~/.cronymax/extensions/<ext-id>/`
- [ ] iframe sandbox CSP 配置
- [ ] `acquireCronymaxApi()` 注入：postMessage 双向桥
- [ ] 路由 postMessage 到 Node host 对应扩展实例
- [ ] config.page / ui.sidebar.view 用这套通道渲染

**验收**：
- 写一个 "ping" webview 扩展：HTML 里点按钮 → postMessage → 扩展 activate 里收到 → 回 pong → HTML 更新
- 扩展 A 的 webview 不能 access 扩展 B 的 webview DOM

### Phase 7 · `bytedance.coco` Dogfood（2 周）

**目标**：完整闭环：装、选 Coco、开会话、流式消息、tool 调用、权限弹窗、设置页

**任务**：
- [ ] coco-extension/ 目录骨架（独立 git repo 或 cronymax monorepo 子目录）
- [ ] manifest（见 spec §8.1.1，相应改为 AgentProvider 命名）
- [ ] `src/acp-client.ts` —— 端口 `/tmp/acp_mcp_client.py` 到 TS（已验过的逻辑）
- [ ] `src/coco-session.ts` —— ACP event 翻译
- [ ] `src/main.ts` —— register agents.provider + commands
- [ ] `src/settings/index.html` + `index.ts` —— 自定义设置页（OAuth-like + 模型探查）
- [ ] 打包成 `.crx`
- [ ] 写测试：用 mock coco binary 跑端到端

**通用性 checkpoint**：
- coco 扩展代码**不引用任何 cronymax 内部模块**，只 `import * as cronymax from "@cronymax/extension"`
- 整个扩展是个独立 npm 包，可以脱离 cronymax repo 单独 build

**Phase 7 末 dogfood 第二个扩展**：再写一个 demo 扩展（推荐 `acme.mermaid-renderer`）验证通用性。这是规则 #5 的体现。

### Phase 8 · Flow Agent 系统接通（1 周，跟 Phase 7 末并行）

**目标**：`.cronymax/agents/*.yaml` 的 `provider` 字段能引用任何 AgentProvider；flow 运行时调度

**任务**：
- [ ] `capability/agent_loader.rs` 改造：`AgentDef` 加 provider + provider_config
- [ ] `flow/agent_step.rs` 改造：通过 registry 拿 AgentProvider 而不是硬编码
- [ ] `web/src/panels/flows/agents/` 新建：agent list + 4 步新建向导
- [ ] 兼容：老 agent yaml（没有 provider 字段）默认 `provider: native`

**验收**：
- 新建 agent code-reviewer，绑定 coco / GPT-5.4 / plan mode
- 写一个 flow，step type=agent 引用 code-reviewer，运行
- agent.allowed_tools 门控生效（试着调用未授权的 tool，看到 deny 决策）

### Phase 9 · SDK + 扩展管理 UI（2 周）

**目标**：第三方能在不读 cronymax 源码的情况下写扩展

**任务**：
- [ ] IDL → TS 类型 codegen 脚本
- [ ] @cronymax/extension npm 包发布到内网 npm（或 GitHub Packages）
- [ ] 模板仓 `cronymax-extension-template`：含 hello-world / theme / content-renderer / agent-provider 四个示例
- [ ] 设置面板 - Extensions 标签：列表 / 安装 / 卸载 / 启用 / 禁用 / 详情 / 撤销 capability
- [ ] CLI 完善：`cronymax ext package <dir>` / `cronymax ext dev <dir> --watch`（开发期 hot reload）

**验收**：
- 一个不知道 cronymax 内部实现的同事，用模板仓 + README，半天内能写出能用的扩展

### Phase 10 · 收尾 + Alpha（1 周）

**目标**：稳定性、UX、文档闭环

**任务**：
- [ ] Crash 恢复测试（host 进程 SIGKILL 后自动重启）
- [ ] 资源额度：CPU / 内存 / 子进程数限制
- [ ] Capability 弹窗 UX：人话清单、撤销路径
- [ ] 文档：spec / SDK API ref / 开发者指南 / 安全模型说明
- [ ] 性能基准 + 优化（基于 Phase 2 / Phase 7 数据）
- [ ] Alpha 发布

**验收**：
- 跑 1 小时聊天 + flow 混合负载，host 进程不泄漏不崩
- 装 / 卸 / 启 / 禁 流程跑通无残留

---

## 4. 并行机会图

```
Phase 0 (foundation)
    │
    ▼
Phase 1 (manifest+registry+activation)
    │
    ▼
Phase 2 (Node host + L1 第一切片)
    │
    ├──────────┬──────────┬──────────┐
    ▼          ▼          ▼          ▼
Phase 3    Phase 5    Phase 6    (start Phase 9 SDK)
(其余 L1)  (L1.5 events) (webview)
    │          │          │          │
    └──────────┴──────────┴──────────┘
                │
                ▼
            Phase 4 (L2 EPs × 6)  ← AgentProvider 在这里
                │
                ├──────────┐
                ▼          ▼
            Phase 7    Phase 8
            (coco)     (flow integration)
                │          │
                └──────────┘
                    │
                    ▼
                Phase 9 (SDK + UI 完工)
                    │
                    ▼
                Phase 10 (alpha)
```

**最大并行收益**：Phase 3 / 5 / 6 三者全可并，Phase 4 是合流点。

---

## 5. 团队配置建议

最低可行：**1 Rust 后端 + 1 全栈/TS 前端 + 0.5 review 资源**，10-13 周到 alpha
推荐：**2 Rust + 1 TS + 0.5 design** —— 8-10 周到 alpha

| 角色 | 主要负责 | 跨阶段 |
|---|---|---|
| Rust 后端 A | Phase 1 / 2 / 3（kernel + host + registry） | 全程 |
| Rust 后端 B | Phase 4（L2 EPs 框架 + AgentProvider）+ Phase 5（events）+ Phase 8（flow 接通） | Phase 4+ |
| TS 全栈 | @cronymax/extension SDK + IDL codegen + Phase 6 webview + Phase 7 coco extension + Phase 9 UI | Phase 0+ |
| Design 兼职 | capability 弹窗 + Extensions 管理 UI + agent wizard | Phase 4 起 |

---

## 6. v1 Alpha 范围（明确 IN）

- [ ] L1 Kernel 14 原语
- [ ] L1.5 平台事件 8 条入门
- [ ] L2 EP × 6（command / config.schema / config.page / agents.provider / content.renderer / ui.sidebar.view）
- [ ] Node.js Extension Host（共享 V8 不入 v1，subprocess + Node 就够）
- [ ] Webview 沙箱 iframe
- [ ] Capability 模型 + 安装期授权弹窗
- [ ] `@cronymax/extension` TS SDK + IDL codegen
- [ ] `cronymax ext` CLI（install / list / enable / disable / uninstall / package / dev）
- [ ] 设置面板 - Extensions 标签
- [ ] `bytedance.coco` 内置 dogfood 扩展
- [ ] Flow agent 系统适配（agent yaml 引用 provider id）
- [ ] 第二个 dogfood 扩展（验证通用性，推荐 mermaid renderer）

## 7. 明确**不**在 v1（推 M1+）

- Marketplace + 数字签名
- 远程开发模式（SSH / WSL）
- CEF + V8 host（v1 Node subprocess 替代）
- 不受信扩展独立 host 隔离（v1 全共享）
- Process runtime（v1 只有 Node runtime，第三方非 JS 扩展 spawn 子进程）
- 正式 L3 服务 registry（schema + semver；v1 用 extension.exports）
- WASM runtime
- 其余 L2 EPs：keybinding / menu.item / ui.activitybar.item / ui.statusbar.item / workspace.fs-provider / chat.tool / lm.provider / auth.provider
- inline content renderers（v1 只支持 block）
- 内置 LLM provider（Anthropic/OpenAI/Ollama）迁移到扩展模型
- `community.acp-bridge` / `community.mcp-bridge` 通用桥（M1 抽出）

---

## 8. 第一周行动清单

启动 Phase 0，5 个并行的小项：

1. **本周一**：spec-v0.1 + plan-v0.1 内部评审会，定稿。预计 90 分钟
2. **本周二-三**：写 IDL v1（`cep-idl/v1/*.ts`），重点是 AgentProvider / AgentSession 完整接口冻结
3. **本周二-四**：摸清现有 flow runtime / capability/agent_loader.rs 实现，写 1 页 brief 放 `docs/extensions/legacy-agent-step.md`
4. **本周三-四**：Node host spike，跑通 "Rust spawns Node, exchange MessagePack-RPC ping/pong"
5. **本周五**：Phase 0 评审；正式立项；起 Phase 1 任务卡

启动条件（all-of）：
- IDL v1 review 至少 2 个 +1
- Node host spike 通了（ping/pong + 1000 round-trip 性能数据）
- Flow legacy brief 完成
- 团队人员对齐 + 权限准备好

---

## 9. 通用性自检清单（每周回顾必过）

每周五五分钟：

- [ ] 这周代码里有没有出现 "coco" / "mermaid" / "slack" 字眼在 `crates/cronymax/src/extensions/`？（违反规则 #1）
- [ ] 这周新加的 L2 EP 有没有专属 handler 文件？（违反规则 #2 —— 应走通用注册中心）
- [ ] 这周有没有为某个特定扩展开 capability 后门？（违反规则 #3）
- [ ] 新写的 chat 面板代码和 flow runtime 代码调 AgentProvider 是不是用同一份接口？（违反规则 #4）
- [ ] 这周新增的能力，如果让外部第三方写扩展实现，能不能不改核心？（违反规则 #5）

任何一条 NO，下周第一件事修。

---

## 10. 关键 DRI 决策

下面这几个决策**需要在 Phase 0 末签字**：

1. **Node 版本**：Node 20 LTS / 22 LTS
2. **MessagePack 实现**：`msgpack-lite`（JS）+ `rmp-serde`（Rust）
3. **CLI 实现位置**：扩在现有 cronymax CLI 上，还是单独 `cronymax-ext` 二进制
4. **SDK npm 发布渠道**：内网 npm（推荐）/ GitHub Packages / 公网 npm
5. **测试 coco 二进制是否打入 cronymax build 还是要求用户单独装**：推荐用户单独装，cronymax 通过 PATH 找
6. **是否在 Phase 2 直接发 Phase 0 alpha 给少数内部开发者试**：能拿到早期反馈，但接口未稳

---

## 11. 跟 spec doc 的一致性

本计划与 [`spec-v0.1.md`](spec-v0.1.md) 同源。任何冲突以 spec 为准。

**计划同步更新触发点**：
- spec doc 任何决策修改 → 7 天内更新本计划
- 任何 Phase 验收失败 → 当周更新本计划相关 Phase 估时

---

文档版本：v0.1 · 2026-05-19
DRI：待
评审：待
