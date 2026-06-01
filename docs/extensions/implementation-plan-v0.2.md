# Cronymax 扩展平台 · 实施计划 v0.2

配套：[`spec-v0.2.md`](spec-v0.2.md)
状态：待启动；替换 plan-v0.1

---

## 0. 已锁定的决策（最终）

详见 spec-v0.2 §13。要点：

- 三层：L1 Kernel + L1.5 Events + L2 EPs
- 命名：AgentProvider / AgentSession（不绑 chat 表面）
- 命名空间锁 `cronymax.*`
- Runtime：**Node 22 LTS subprocess，每扩展独立 host，lazy activate，不主动 deactivate**
- 安全 α：**Node Permission Model**（`--experimental-permission --allow-*`）+ child_process 平台门控
- 安全 γ：M1 加 OS sandbox 外层（sandbox-exec / bubblewrap / Job Object）
- IPC：Unix socket（Win Named Pipe）+ MessagePack-RPC
- 模块系统：CJS 主推（与 VS Code 对齐）；ESM M1
- 跨扩展通信：`extension.exports` VS Code 同款
- v1 L2 EP：6 个起步

---

## 1. 通用性硬规则（每周自检）

违反任何一条 = 设计跑偏：

1. **平台核心代码不出现具体扩展名**（coco / slack / mermaid 等只在内置 dogfood 扩展代码里）
2. **所有 L2 EP 走同一套注册机制**（不允许 per-EP 专用 handler）
3. **任何 L1 API 必经 capability gate**（无内置扩展豁免）
4. **AgentProvider 对 chat 面板和 flow runtime 完全对称**
5. **加内置扩展前先尝试用纯 SDK 写**，发现要改核心就停下加 EP

---

## 2. 风险登记 v0.2

| 风险 | 缓解 | 决策节点 |
|---|---|---|
| R1 · Node host RAM 8 扩展 ~500MB | 接受；M1 评估 host 池 | M1 |
| R2 · 首次激活 80-200ms 延迟 | 预热常用扩展 + spinner | Phase 7 末 |
| R3 · Node Permission Model 实验性 | 锁 Node 22 LTS；监控变化 | Node 版本节点 |
| R4 · Node Permission 不防资源耗尽 | Rust 限额 + 强杀 | Phase 10 |
| R5 · child_process 平台门控漏 | argsPattern 严 + 审计 + M1 OS sandbox | M1 |
| R6 · L1 API 设计错 | Phase 0 冻 IDL + peer review | Phase 0 末 |
| R7 · flow runtime 当前 agent step 耦合 | Phase 0 摸现状；Phase 8 重构 | Phase 0 末 |
| R8 · 内置 LLM provider 迁移 | v1 不迁；M1 评估 | 已决 |

---

## 3. 阶段分解（串行 ~14 周；2-3 人并行 10-12 周）

### Phase 0 · Foundation（1 周）

**目标**：拿到开工基线

- [ ] spec-v0.2 + plan-v0.2 内部评审定稿
- [ ] 写 IDL v1（`crates/cronymax/src/extensions/cep-idl/v1/*.ts`），冻结 AgentProvider / AgentSession / ChatEvent 完整签名
- [ ] 摸现有 flow runtime + capability/agent_loader.rs 实现，写 1 页 brief
- [ ] **Node 22 Permission Model 实测**：
  - 跑一个 spike，验证 `--experimental-permission --allow-fs-read=...` 行为
  - 试 `eval("require('fs').readFileSync('/etc/passwd')")` 是否拦
  - 试常见 npm 包（octokit / yaml / zod）在 permission 下能不能正常工作
  - 性能影响（permission check overhead）
- [ ] MessagePack-RPC 选型：`@msgpack/msgpack`（JS）+ `rmp-serde`（Rust）spike
- [ ] 起 `crates/cronymax/src/extensions/` 模块骨架
- [ ] 起 npm 仓库 `@cronymax/extension`

**验收**：
- IDL v1 freeze
- Node 22 permission spike 通了
- MessagePack-RPC ping/pong 通了
- 有 flow 现状 brief

### Phase 1 · Manifest + Registry + Activation（2 周）

**目标**：装/卸/启用/禁用流程通；激活事件能匹配

- [ ] `manifest.rs` —— 清单 schema 解析 + 校验（命名空间、capability、publisher 前缀）
- [ ] `registry.rs` —— 扫 `~/.cronymax/extensions/` + 启用状态持久化
- [ ] `activation.rs` —— activationEvents 匹配引擎
- [ ] CLI 命令：`cronymax ext install / list / enable / disable / uninstall`

**验收**：
- 装一个纯声明扩展（只有 manifest），CLI 能列、能启禁、能卸
- manifest 校验错误能明确报错
- 任何 `cronymax.*` 命名空间写入被拒

### Phase 2 · Node Host + 基础 L1（2-3 周）

**目标**：hello-world 扩展能跑 activate；commands 能调用

- [ ] Node 22 LTS 打包进 cronymax build（4 平台）
- [ ] `extensions/host/node.rs`：
  - 每扩展独立 spawn
  - 根据 manifest 拼 `--experimental-permission --allow-*` flags
  - Unix socket / Named Pipe 通信
  - 进程生命周期 + 健康检查 + 重启
- [ ] `extension-host-bootstrap.js`：
  - 接 socket 握手
  - 接收 activate/deactivate 指令
  - 加载 `~/.cronymax/extensions/<id>/main.js` 到 vm.Context
  - 注入 `@cronymax/extension`（per-extension instance）
  - child_process 包装（拦截后 RPC 给 Rust）
- [ ] `extensions/api/lifecycle.rs` + `commands.rs`
- [ ] `extensions/capability.rs` —— manifest → flags 转换 + child_process 校验
- [ ] `@cronymax/extension` SDK 第一版：导出 lifecycle + commands

**验收**：
- hello-world 扩展（30 行 TS）：activate 注册 command，命令面板能调用
- 故意写 `fs.readFileSync('/etc/passwd')` → Node throw `ERR_ACCESS_DENIED`
- 故意写 `spawn("curl")` 不在白名单 → 平台拒
- 杀掉 Node host，平台自动重启
- Phase 2 末跑性能基准：1000 commands.execute round-trip P99 < 5ms

### Phase 3 · 其余 L1 Kernel（2 周）

**目标**：14 个原语全开

每条含 capability gate + 单测：

- [ ] `events.rs` — pub/sub
- [ ] `config.rs` — get/update + onDidChange
- [ ] `secrets.rs` — Keychain / DPAPI / secret-service
- [ ] `storage.rs` — global / workspace state
- [ ] `process.rs` — spawn 走 Rust，stdin/stdout/stderr 经 RPC 流回
- [ ] `fs.rs` — 主要靠 Node Permission；SDK 暴露 workspace.fs 风格 helper
- [ ] `network.rs` — fetch / WebSocket 经 Node 22+ --allow-net
- [ ] `webview.rs` — stub 接口（实际渲染 Phase 6）
- [ ] `auth.rs` — OAuth / PKCE / device-flow
- [ ] `extensions.rs` — getExtension + exports

**验收**：
- 每原语 1 个 focused 测试扩展
- capability 越界全部被拦
- secrets 跨扩展不可读

### Phase 4 · L2 EP × 6（2 周，可并行 Phase 5/6）

**目标**：6 个起步 EP 在平台 UI 里被消费

- [ ] `contributions/` 通用注册中心（满足规则 #2）
- [ ] `cronymax.command`（已在 Phase 2，整合）
- [ ] `cronymax.config.schema` — 设置面板从清单生成表单
- [ ] `cronymax.config.page` — 设置面板嵌入 webview（依赖 Phase 6）
- [ ] `cronymax.agents.provider` — **核心**，详见子任务
- [ ] `cronymax.content.renderer` — block 类，先支持一个 MIME
- [ ] `cronymax.ui.sidebar.view` — 侧栏 webview

**子任务 4.1 · AgentProvider**：
- [ ] IDL 完整冻结
- [ ] Rust trait `AgentProviderHandle`
- [ ] 聊天面板：替换硬编码 LLM 选择器为 `registry.consume("cronymax.agents.provider")`
- [ ] flow runtime：agent step 改为通过 agents registry 调度
- [ ] permissionRequest 事件桥接平台权限弹窗

**通用性 checkpoint**：
- 能否用纯 SDK 写一个新 EP 类的扩展？6 个 v1 EP 任何一个都必须可纯 SDK 完成

### Phase 5 · L1.5 平台事件（1 周，可并行 Phase 4/6）

**目标**：8 条平台事件能 emit；扩展能订阅

- [ ] 平台事件总线
- [ ] 聊天面板加 emit：`cronymax.session.*` / `cronymax.message.*`
- [ ] Tool 调度加 emit：`cronymax.tool.*` / `cronymax.permission.*`
- [ ] Capability `events.subscribe` 白名单校验

**验收**：
- "logger" 测试扩展订阅 `cronymax.message.assistant.done`，把 fullText 写入文件
- 跑一次聊天，日志文件追加正确

### Phase 6 · Webview 基建（2 周，可并行 Phase 4/5）

**目标**：扩展 webview 能渲染、能 postMessage 跨进程通信

- [ ] CEF 协议处理器：`cronymax-webview://<ext-id>/<path>`
- [ ] iframe sandbox CSP 配置
- [ ] `acquireCronymaxApi()` 注入：postMessage 桥
- [ ] CEF → Rust → Node host 的 postMessage 中转链路
- [ ] config.page / ui.sidebar.view 用这套渲染

**验收**：
- "ping" webview 扩展：HTML 按钮点 → postMessage → 扩展 main.ts 收到 → 回 pong → HTML 更新
- 扩展 A 的 webview access 不到扩展 B 的 webview

### Phase 7 · `bytedance.coco` Dogfood（2 周）

**目标**：完整闭环

- [ ] coco-extension/ 项目骨架（独立 repo 或 monorepo 子目录）
- [ ] manifest 按 spec §11 写
- [ ] `src/acp-client.ts` — 端口 Python POC 到 TS（用 `child_process` 标准 API，受平台门控）
- [ ] `src/coco-session.ts` — ACP event 翻译
- [ ] `src/main.ts` — register agents.provider + commands
- [ ] `src/settings/index.html` — 自定义设置页
- [ ] esbuild 打包成单个 dist/main.js（CJS, target node22）
- [ ] 打包 .crx
- [ ] 端到端测试（mock coco binary）

**通用性 checkpoint**：
- coco 扩展代码**不引用任何 cronymax 内部模块**
- 整个扩展是独立 npm 包，可脱离 cronymax repo 单独 build

**Phase 7 末做第二个 dogfood**：`acme.mermaid-renderer` 验通用性。

### Phase 8 · Flow Agent 系统接通（1 周，与 Phase 7 末并行）

**目标**：`.cronymax/agents/*.yaml` 的 `provider` 字段引用任意 AgentProvider；flow runtime 调度

- [ ] `capability/agent_loader.rs`：AgentDef 加 provider + provider_config
- [ ] `flow/agent_step.rs`：通过 registry 拿 AgentProvider
- [ ] `web/src/panels/flows/agents/`：list + 4 步向导
- [ ] 兼容：老 agent yaml 默认 `provider: native`

**验收**：
- 新建 agent code-reviewer 绑定 coco / GPT-5.4 / plan
- flow step type=agent agent=code-reviewer 运行
- allowed_tools 门控生效

### Phase 9 · SDK + 扩展管理 UI（2 周）

**目标**：第三方能在不读 cronymax 源码的情况下写扩展

- [ ] IDL → TS 类型 codegen 脚本
- [ ] `@cronymax/extension` npm 包发布
- [ ] 模板仓 `cronymax-extension-template`：含 hello-world / theme / content-renderer / agent-provider 四个示例
- [ ] 设置面板 - Extensions 标签：列表 / 安装 / 卸载 / 启用 / 禁用 / 详情 / 撤销 capability
- [ ] CLI 完善：`cronymax ext package <dir>` / `cronymax ext dev <dir> --watch`

**验收**：
- 一个不知道 cronymax 内部实现的同事，用模板仓 + README，半天内能写出能用的扩展

### Phase 10 · 收尾 + Alpha（1 周）

**目标**：稳定性、UX、文档闭环

- [ ] Crash 恢复测试（SIGKILL → 自动重启）
- [ ] 资源额度：CPU / 内存 / 子进程数限制
- [ ] Capability 弹窗 UX：人话清单、撤销路径
- [ ] 预热常用扩展：减少首次激活延迟
- [ ] 文档：spec / SDK API ref / 开发者指南 / 安全模型说明
- [ ] 性能基准 + 优化
- [ ] Alpha 发布

**验收**：
- 跑 1 小时聊天 + flow 混合负载，Node host 不泄漏不崩
- 装 / 卸 / 启 / 禁 流程无残留

---

## 4. 并行机会图

```
Phase 0 (foundation + Node 22 spike)
    │
    ▼
Phase 1 (manifest+registry+activation)
    │
    ▼
Phase 2 (Node host + L1 切片)
    │
    ├──────────┬──────────┬──────────┐
    ▼          ▼          ▼          ▼
Phase 3    Phase 5    Phase 6    Phase 9 SDK start
(其余 L1)  (L1.5)    (webview)
    │          │          │          │
    └──────────┴──────────┴──────────┘
                │
                ▼
            Phase 4 (L2 EPs × 6)
                │
                ├──────────┐
                ▼          ▼
            Phase 7    Phase 8
            (coco)     (flow integration)
                │          │
                └──────────┘
                    │
                    ▼
                Phase 9 (UI + SDK 完工)
                    │
                    ▼
                Phase 10 (alpha)
```

---

## 5. 团队配置建议

- 最低：1 Rust + 1 TS 全栈 + 0.5 review → 10-12 周
- 推荐：2 Rust + 1 TS + 0.5 design → 8-10 周

| 角色 | 主要负责 |
|---|---|
| Rust A | Phase 1/2/3 (kernel + Node host + registry) |
| Rust B | Phase 4 (L2 EPs + AgentProvider) + Phase 5 (events) + Phase 8 (flow) |
| TS 全栈 | @cronymax/extension SDK + Phase 6 webview + Phase 7 coco extension + Phase 9 UI |
| Design 兼职 | capability UX + 扩展管理 UI + agent wizard |

---

## 6. M0 alpha 范围

详见 spec-v0.2 §15。

---

## 7. 第一周行动清单（Phase 0）

启动 5 个并行小项：

1. **本周一**：spec-v0.2 + plan-v0.2 内部评审会，定稿（90 分钟）
2. **本周二-三**：写 IDL v1 freeze AgentProvider 等
3. **本周二-四**：摸现有 flow runtime / agent_loader.rs，写 brief
4. **本周二-四**：**Node 22 Permission Model spike**
   - 验证 `--experimental-permission` 行为
   - 测试 eval / process.binding 绕过
   - 测试 npm 包兼容性（octokit / yaml / zod 等）
   - 测性能 overhead
5. **本周三-四**：MessagePack-RPC spike（Rust ↔ Node ping-pong + 1000 round-trip）
6. **本周五**：Phase 0 评审；正式立项；起 Phase 1 任务卡

启动条件（all-of）：
- IDL v1 review 至少 2 个 +1
- Node 22 permission spike 通过（性能、兼容、安全三项达标）
- MessagePack-RPC spike 通了
- flow legacy brief 完成
- 团队人员对齐

---

## 8. 通用性自检清单（每周回顾必过）

每周五五分钟：

- [ ] 这周代码里有没有出现 "coco" / "mermaid" / "slack" 字眼在 `crates/cronymax/src/extensions/`？
- [ ] 这周新加的 L2 EP 有没有专属 handler 文件？（应走通用注册中心）
- [ ] 这周有没有为某个特定扩展开 capability 后门？
- [ ] chat 面板代码和 flow runtime 代码调 AgentProvider 是不是用同一份接口？
- [ ] 这周新增能力，如果让外部第三方写扩展实现，能不能不改核心？

任何一条 NO，下周第一件事修。

---

## 9. 关键 DRI 决策（Phase 0 末签字）

1. Node 版本：**Node 22 LTS**
2. MessagePack 实现：`@msgpack/msgpack`（JS）+ `rmp-serde`（Rust）
3. CLI 实现位置：扩在现有 cronymax CLI 上
4. SDK npm 发布：内网 npm（推荐）/ GitHub Packages
5. 测试 coco binary：用户单独装，cronymax 通过 PATH 找
6. Phase 7 末是否在内部发 alpha：能拿到早期反馈，接口未稳

---

## 10. 跟 v0.1 的对照（差异）

| 项 | v0.1 | v0.2 |
|---|---|---|
| Runtime | CEF V8 host | Node 22 subprocess（每扩展独立）|
| Capability 实施 | require 劫持 | Node Permission Model |
| Node 兼容层 | @cronymax/node-compat shim | 不需要 |
| 进程模型 | 单 helper 共享 | 每扩展独立进程 |
| 安全升级路径 | 不明确 | α(v1) → γ(M1) 双层叠加 |
| Webview 通信 | V8 同进程 | 跨进程经 Rust 中转 |
| npm 生态 | ~85% | ~100% |
| 估时 | 14-18 周 | 12-14 周（少 compat 层 + 少 V8 binding）|

---

文档版本：v0.2 · 2026-05-19
DRI：待定
评审：待
