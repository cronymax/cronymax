# Cronymax 扩展平台 · 任务清单 v0.3

配套：[`spec-v0.3.md`](spec-v0.3.md) · [`implementation-plan-v0.3.md`](implementation-plan-v0.3.md)

格式：`P{phase}-T{task}` · `[角色] [估时] 任务描述 · 依赖`
角色：**RA** = Rust 后端 A · **RB** = Rust 后端 B · **TS** = TS 全栈 · **DS** = Design 兼职 · ***** = 全员

---

## P0 · Foundation（1 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P0-T01 | * | 1.5h | spec-v0.3 + plan-v0.3 评审会，定稿 | — | 会议纪要 + 决策签字 |
| P0-T02 | RA | 2d | 写 `crates/cronymax/src/extensions/cep-idl/v1/*.ts` IDL，freeze AgentProvider / AgentSession / AgentEvent / lifecycle / commands / events / workspace / window / secrets / auth / extensions / renderers | P0-T01 | ≥2 人 +1 review |
| P0-T03 | RB | 2d | 摸现有 flow runtime + `capability/agent_loader.rs` 实现，写 1 页 brief 到 `docs/extensions/legacy-agent-step.md` | — | brief 落地 |
| P0-T04 | TS | 2d | **Node 26 真机 spike**：验证 8 个 --allow-* flag 行为 + 性能 + npm 兼容 + 路径 canonicalize；产出 `docs/extensions/node26-permission-spike.md` | — | spike 报告全 ✅ |
| P0-T05 | RA | 1d | MessagePack-RPC spike：Rust ↔ Node ping-pong + 1000 round-trip P99；产出 `docs/extensions/msgpack-rpc-spike.md` | — | < 5ms P99 |
| P0-T06 | RA | 1d | 起 `crates/cronymax/src/extensions/` 模块骨架 + npm 仓库 `@cronymax/extension` 占位 | P0-T02 | `cargo build` 通过 |
| P0-T07 | * | 1h | Phase 0 评审 + Phase 1 任务卡 + DRI 决策签字 | P0-T02..T05 | Phase 1 开工 |

---

## P1 · Manifest + Registry + Activation（2 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P1-T01 | RA | 2d | `extensions/manifest.rs`：cronymax-extension.json schema 定义 + serde 反序列化 | P0-T06 | unit tests 通过 |
| P1-T02 | RA | 2d | manifest 校验：publisher 前缀匹配 / `cronymax.*` 命名空间拒 / contributes 必须在 extension-points 申报 / 路径 canonicalize | P1-T01 | 错误用例全部明确报错 |
| P1-T03 | RA | 2d | `extensions/registry.rs`：扫 `~/.cronymax/extensions/` + 启用状态持久化到 `registry.json` | P1-T01 | install/list/uninstall round-trip |
| P1-T04 | RA | 2d | `extensions/activation.rs`：activationEvents 匹配引擎（onCommand / onAgentProvider / onView / onStartup / `*` 警告） | P1-T03 | 各种 event 模式都能匹配 |
| P1-T05 | RA | 2d | CLI `cronymax ext install / list / enable / disable / uninstall` | P1-T01..T04 | 端到端可装可卸 |
| P1-T06 | * | 30min | Phase 1 验收 demo（装一个纯声明扩展走完整生命周期） | P1-T05 | demo 通过 |

---

## P2 · Node Host + L1 第一切片（2 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P2-T01 | RA | 1d | Node 26 多平台二进制打包（macOS arm64/x64 + Win x64 + Linux x64）入 cronymax build | P0-T04 | 4 平台 cronymax build 含 node |
| P2-T02 | RA | 2.5d | `extensions/capability.rs::build_node_flags`：manifest → Node `--allow-*` flags（含平台变量展开 + canonicalize 双填 + `--no-warnings` 永远开 + `--allow-net` 按 manifest network 申报决定 emit） | P1-T01 | unit tests 覆盖所有 capability 类型 + 各平台 symlink case |
| P2-T03 | RA | 3d | `extensions/host/node.rs`：每扩展 spawn Node 26 + **inherited fd 3** 通信（stdio `[pipe,pipe,pipe,pipe]`）+ 进程生命周期 + ping/pong 健康检查 + 重启 N 次 | P0-T05, P2-T01, P2-T02 | host spawn → 健康 → kill → 重启 |
| P2-T04 | RA | 2d | `extensions/rpc/`：MessagePack-RPC 服务端（请求/响应/通知/取消 token；走 fd 3）| P0-T05 | round-trip + cancellation 通 |
| P2-T05 | TS | 3d | `extension-host-bootstrap.js`：wrap fd 3 为 net.Socket / 握手 / 加载 main.js / 注入 @cronymax/extension / `process.on('uncaughtException')` / activate try-catch / console.* 拦截转 RPC | P2-T03 | hello-world activate() 跑通；activate throw 触发 audit |
| P2-T06 | RA | 1d | `extensions/api/lifecycle.rs` + `api/commands.rs` 第一切片 | P2-T03, P2-T04 | command register/execute round-trip |
| P2-T07 | TS | 2d | `@cronymax/extension` SDK v0：导出 lifecycle + commands + logging（OutputChannel）类型 | P0-T02 | npm publish 公网（`@cronymax/extension` scoped 0.x） |
| P2-T08 | TS | 1d | 写 hello-world 测试扩展（30 行 TS） | P2-T05, P2-T07 | 命令面板能调 hello.greet |
| P2-T09 | RA | 1d | Phase 2 性能基准：**command 端到端 RPC round-trip P99 < 5ms**（实测 spike P99 ~20μs，250× 冗余预期）| P2-T08 | 基准报告 |
| P2-T10 | RA | 1d | 安全冒烟测试：`fs.readFileSync('/etc/passwd')` 拦；扩展未声明 network 时 `fetch()` 拦；声明了 network 时 `fetch()` 通（v1 boolean）；inspector / addons / wasi 默认禁 | P2-T08 | 全过 |
| **P2-T11** | RA | 2d | **扩展日志系统平台侧**：`extensions/logging.rs` 4 类 log writer（host.log / output.log / channels/\*.log / audit.log）+ rotate + 异步 tokio task；EH 错误处理 6 层 A-F 平台侧（活/uncaught/hung/crashed/exit；ping/pong 5s 间隔） | P2-T03, P2-T05 | 6 层 A-F 单测全过 |
| **P2-T12** | TS | 1d | `@cronymax/extension` SDK `window.createOutputChannel` 实现（含 LogOutputChannel + NDJSON 格式化 + RPC `log/channel`）| P2-T07 | 测试扩展能 `info/warn/error` 写 channel 文件 |

---

## P3 · 其余 L1 Kernel（2 周，可与 P5/P6 并行）

每条 API 含 capability gate（由 Node Permission 自动管）+ 单测：

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P3-T01 | RA | 2d | `api/events.rs`：扩展侧 pub/sub topic + 跨进程路由 | P2-T06 | 双向事件通 |
| P3-T02 | RA | 1d | `api/config.rs`：get/update + onDidChange | P2-T06 | 配置变更触发回调 |
| P3-T03 | RA | 2d | `api/secrets.rs`：macOS Keychain / Win DPAPI / Linux secret-service 集成 | P2-T06 | 跨扩展不可读 |
| P3-T04 | RA | 1d | `api/storage.rs`：global / workspace state KV | P2-T06 | per-extension 隔离 |
| P3-T05 | RA | 1d | `api/webview.rs` stub（实际渲染 P6 做） | P2-T06 | stub 接口可调 |
| P3-T06 | RA | 2d | `api/auth.rs`：内置 OAuth / PKCE / device-flow | P2-T06 | OAuth demo 跑通 |
| P3-T07 | RA | 1d | `api/extensions.rs`：getExtension + exports（VS Code 同款） | P2-T06 | 跨扩展 exports 可用 |
| P3-T08 | TS | 2d | SDK 更新：补齐 events/config/secrets/storage/auth/extensions 类型 | P3-T01..T07 | npm publish v0.1 |
| P3-T09 | * | 30min | Phase 3 验收：每原语 1 个 focused 测试扩展全跑过 | P3-T01..T08 | 全绿 |

---

## P5 · L1.5 平台事件（1 周，与 P3/P6 并行）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P5-T01 | RB | 2d | `extensions/events.rs`：平台事件总线 + 订阅路由 + capability `events.subscribe` 白名单校验 | P3-T01 | 未授权订阅 throw |
| P5-T02 | RB | 1d | 聊天面板 emit `cronymax.session.*` / `cronymax.message.*` / `cronymax.permission.*` | P5-T01 | 实际聊天触发事件 |
| P5-T03 | RB | 1d | Tool 调度 emit `cronymax.tool.*` | P5-T01 | tool 调用触发事件 |
| P5-T04 | TS | 1d | "logger" 测试扩展订阅 `cronymax.message.assistant.done`，落盘 | P5-T01, P3-T01 | 跑聊天日志正常追加 |

---

## P6 · Webview 基建（2 周，与 P3/P5 并行）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P6-T01 | TS | 3d | CEF 自定义协议处理器 `cronymax-webview://<ext-id>/<path>`：路由到 `~/.cronymax/extensions/<ext-id>/` | P2-T03 | iframe 加载扩展资源 |
| P6-T02 | TS | 1d | iframe sandbox CSP 配置（default-src none, script-src self, etc.） | P6-T01 | 跨 origin 隔离 |
| P6-T03 | TS | 2d | `acquireCronymaxApi()` 注入 + iframe ↔ host postMessage 桥 | P6-T01 | postMessage round-trip |
| P6-T04 | RA | 2d | Rust core：CEF ↔ Node host 跨进程 postMessage 中转 + 鉴权（panel 归属哪个扩展） | P6-T03, P2-T04 | 跨进程消息正确路由 |
| P6-T05 | TS | 1d | "ping" 测试扩展：HTML 按钮 → 扩展 main.ts handler → 回 pong → HTML 更新 | P6-T03, P6-T04 | demo 通 |
| P6-T06 | TS | 1d | 验证扩展 A 的 webview access 不到扩展 B | P6-T05 | 越界 throw |

---

## P4 · L2 EP × 6（2 周，依赖 P3 完成，可与 P5/P6 并行末段）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P4-T01 | RB | 2d | `extensions/contributions/`：通用 L2 EP 注册中心（单一机制，6 个 EP 共用） | P3-T08 | 每 EP 一行注册代码 |
| P4-T02 | RB | 1d | `cronymax.command` 整合（P2 完成的整合到 contributions 框架） | P4-T01 | 命令面板列出贡献命令 |
| P4-T03 | TS | 2d | `cronymax.config.schema` —— 设置面板从清单 JSON Schema 自动生成表单 | P4-T01 | Coco config 字段渲染 |
| P4-T04 | TS | 2d | `cronymax.config.page` —— 设置面板嵌入扩展 webview | P4-T01, P6-T04 | Coco 设置页能开 |
| P4-T05 | RB | 3d | **`cronymax.agents.provider`**：Rust trait + registry + 聊天面板硬编码 LLM 选择器替换 | P4-T01, P3-T08 | 聊天面板列扩展贡献的 provider |
| P4-T06 | RB | 1d | `cronymax.content.renderer`（block 类，先支持 mermaid mime）| P4-T01, P6-T04 | mermaid 块自动渲染 |
| P4-T07 | TS | 1d | `cronymax.ui.sidebar.view`（侧栏 webview 注册） | P4-T01, P6-T04 | 扩展能贡献侧栏面板 |
| P4-T08 | RB | 1d | permissionRequest 事件桥接平台权限弹窗 | P4-T05 | 弹窗显示 + 决策回 agent |

---

## P7 · `bytedance.coco` Dogfood（2 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P7-T01 | TS | 0.5d | coco-extension/ 项目骨架（独立 repo 或子目录 + package.json + tsconfig + esbuild config） | P3-T08 | tsc 编译通过 |
| P7-T02 | TS | 1d | manifest cronymax-extension.json（按 spec §11） | P4-T05 | 安装通过校验 |
| P7-T03 | TS | 3d | `src/acp-client.ts`：端口 `/tmp/acp_mcp_client.py` 到 TS，直接 `import { spawn } from "child_process"` | P7-T01 | 单元测试 ACP 协议 round-trip |
| P7-T04 | TS | 2d | `src/coco-session.ts`：ACP event → cronymax AgentEvent 翻译 | P7-T03, P4-T05 | 流式事件全 kind 覆盖 |
| P7-T05 | TS | 1d | `src/main.ts`：register agents.provider("coco", impl) + commands | P7-T04 | activate 跑通 |
| P7-T06 | TS | 2d | `src/settings/index.html` + `index.ts`：自定义 config page（OAuth + 模型探查 UI） | P7-T05, P4-T04 | 设置页能开能交互 |
| P7-T07 | TS | 1d | esbuild bundle 打包 CJS / target node22+ | P7-T01..T06 | dist/main.js 单文件 |
| P7-T08 | TS | 1d | 打包 .crx + 端到端测试（mock coco binary） | P7-T07 | 全流程通 |
| P7-T09 | * | 30min | **通用性 checkpoint**：扩展代码不引用 cronymax 内部模块 | P7-T07 | grep 验证 |
| P7-T10 | TS | 2d | 第二个 dogfood `acme.mermaid-renderer`（content.renderer + sidebar.view） | P4-T06 | mermaid 块自动渲染 |

---

## P8 · Flow Agent 系统接通（1 周，与 P7 末并行）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P8-T01 | RB | 1d | `capability/agent_loader.rs`：AgentDef 加 `provider` + `provider_config` 字段；老 yaml 默认 `provider: native` | — | 老 yaml 兼容 |
| P8-T02 | RB | 2d | `flow/agent_step.rs`：通过 AgentProvider registry 调度（不硬编码）；allowed_tools 透传 + tool 调用门控 | P4-T05, P8-T01 | flow step type=agent 跑 coco 通 |
| P8-T03 | TS | 2d | `web/src/panels/flows/agents/`：list + 编辑器 + 4 步新建向导 | P4-T05 | UI 建 agent 写盘 |
| P8-T04 | * | 30min | Phase 8 验收：UI 建 code-reviewer 绑 coco；flow 跑通；allowed_tools 门控生效 | P8-T01..T03, P7-T08 | demo 全过 |

---

## P9 · SDK + 扩展管理 UI（2 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P9-T01 | TS | 2d | IDL → TS 类型 codegen 脚本（从 `cep-idl/v1/*.ts` 生成 SDK .d.ts） | P0-T02 | 自动同步 |
| P9-T02 | TS | 1d | `@cronymax/extension` v1.0.0 npm 发布（**公网 npm，scoped `@cronymax/extension`**；启动前过 ByteDance 开源/法务流程）| P9-T01 | 包可装 |
| P9-T03 | TS | 3d | 模板仓 `cronymax-extension-template`：hello-world / theme / content-renderer / agent-provider 四示例 + README | P9-T02 | 模板可 clone 直跑 |
| P9-T04 | TS, DS | 3d | 设置面板 - Extensions 标签：列表 / 安装 / 卸载 / 启用 / 禁用 / 详情 / 撤销 capability | P1-T05 | UI 流程通 |
| **P9-T04b** | TS, DS | 2d | **设置面板"日志" tab 简版**：channel 下拉 + 文本滚动 + 时段过滤；自动出现 stdout / stderr 兜底 channel | P9-T04, P2-T11 | UI 能切 channel 看日志 |
| P9-T05 | RA | 1d | CLI 完善：`cronymax ext package <dir>` / `cronymax ext dev <dir> --watch`（透传 stderr/stdout 到终端 + 平台 audit event echo）| P1-T05 | dev watch 热重载 |
| **P9-T05b** | RA | 1.5d | **`cronymax diagnostic-bundle` CLI**：收集所有 session 日志 + manifest + cronymax/OS/Node 版本 + 脱敏（HOME → ~ / Authorization REDACTED / audit args 列删）+ 生成 zip | P2-T11 | 命令产生可用 zip |
| **P9-T05c** | RA | 0.5d | 命令面板 "Developer: Show Logs..." + "Developer: Open Log Folder" | P9-T04 | 命令可用 |
| P9-T06 | * | 1d | **通用性 checkpoint**：找一个外部同事用模板仓 + README 半天内写出可用扩展 | P9-T03 | 同事产出能跑的扩展 |

---

## P10 · 收尾 + Alpha（1 周）

| ID | 角色 | 估时 | 任务 | 依赖 | 验收 |
|---|---|---|---|---|---|
| P10-T01 | RA | 1d | Crash 恢复测试（SIGKILL host → 自动重启 N 次 → 失败禁用通知；详 `extension-logs.md` §7 E）| — | 各场景测试 pass |
| P10-T02 | RA | 1d | 资源额度：CPU / 内存 / 子进程数 / iframe 数 限制 | — | 超限被强杀 |
| P10-T03 | DS | 2d | Capability 弹窗 UX 终稿：人话清单（含变量翻译"`{WORKSPACE}` → 当前工作区"）+ 撤销路径 + 视觉打磨 + 网络项诚实标注"v1 不区分主机" | — | UX review pass |
| P10-T04 | RA | 1d | 预热常用扩展：cronymax 启动按上次使用列表静默激活 | — | 二次启动激活 < 50ms |
| P10-T05 | TS | 2d | 文档：spec / SDK API ref / 开发者指南 / 安全模型说明 / **扩展日志最佳实践 + 6 层错误手册** | 各 Phase 产出 | 文档完整 |
| P10-T06 | RA | 1d | 性能基准 + 优化（基于 P2/P7 数据） | — | 报告 + 优化项 |
| P10-T07 | * | 0.5d | **v1 Alpha 验收**（跑 §8 验收清单全 11 项） | 全部 Phase | 全 ✅ → Alpha 发布 |

---

## v1 Alpha 验收清单（再次明确）

跑通这 11 项 = Alpha：

- [ ] V1 装 `bytedance.coco` 扩展，授权对话框显示人话清单
- [ ] V2 聊天面板 agent picker 列出 Coco
- [ ] V3 选 Coco / GPT-5.4 / plan，开会话发消息流式渲染、tool call 卡片、permission 弹窗
- [ ] V4 设置面板能改 binaryPath，自定义设置页能开
- [ ] V5 flows panel 用向导建 agent code-reviewer 绑 coco
- [ ] V6 flow step type=agent agent=code-reviewer 运行通过
- [ ] V7 装 `acme.mermaid-renderer`，agent 输出 mermaid 块自动渲染
- [ ] V8 杀掉 Node host → 自动重启
- [ ] V9 卸载 → 残留全清
- [ ] V10 安全：`fs.readFile('/etc/passwd')` / `fetch('evil.com')` / native addon / `eval("require...")` 全部 Node 层拦
- [ ] V11 一个不知道 cronymax 内部的同事用模板仓 + README，半天内写出可用扩展

---

## 任务总览

| Phase | 任务数 | 估总人日 | 团队配置时长 |
|---|---|---|---|
| P0 | 7 | ~10d | 1w（并行）|
| P1 | 6 | ~10d | 2w |
| P2 | 10 | ~17d | 2w |
| P3 | 9 | ~14d | 2w（与 P5/P6 并行可缩）|
| P4 | 8 | ~13d | 2w |
| P5 | 4 | ~5d | 1w（并行）|
| P6 | 6 | ~10d | 2w（并行）|
| P7 | 10 | ~14d | 2w |
| P8 | 4 | ~6d | 1w（与 P7 末并行）|
| P9 | 6 | ~10d | 2w |
| P10 | 7 | ~9d | 1w |
| **合计** | **77** | **~118d** | **10-12 周** |

按 3 人团队（Rust A + Rust B + TS）并行：8-10 周。
按 1.5 人团队（Rust + TS）串并行混合：10-12 周。

---

## 跨阶段持续工作

| 工作 | 频率 |
|---|---|
| 通用性自检 5 条（spec §1）| 每周五 5min |
| Phase 末验收 demo | 每 Phase 结束 |
| 风险登记复盘 | 每周一 15min |
| spec / plan / tasks 文档同步 | 决策变更后 7 天内 |

---

## 启动条件（all-of, Phase 0 末）

进 Phase 1 必须：

- [ ] spec-v0.3 + plan-v0.3 + tasks-v0.3 评审定稿
- [ ] IDL v1 freeze + 至少 2 个 +1 review
- [ ] **Node 26 Permission spike 验收报告全 ✅**
- [ ] MessagePack-RPC ping-pong P99 < 5ms
- [ ] flow legacy brief 完成
- [ ] DRI 7 项决策签字（Node 版本、MessagePack 实现、CLI 位置、SDK 发布渠道、coco binary 部署、内部 alpha 时机、Node 26 ship 时 LTS 状态）
- [ ] 团队人员对齐（最低 1.5 人，推荐 3 人 + 0.5 design）

---

文档版本：v0.3 · 2026-05-19
DRI：待定

---

## Phase 0 执行进度（最终 · 2026-05-20）

| ID | 状态 | 备注 |
|---|---|---|
| P0-T01 | ✅ 完成 | Phase 0 评议 2026-05-20；纪要 `docs/extensions/phase-0-review.md` |
| P0-T02 | ✅ 完成 | IDL 14 个 .ts 文件（含评议加的 logging.ts）+ tsconfig + README；`npm run check` strict 全过 |
| P0-T03 | ✅ 完成 | `docs/extensions/legacy-agent-step.md`：当前不存在 AgentProvider 抽象，chat/flow 共享 ReactLoop |
| P0-T04 | ✅ 完成 | `docs/extensions/node26-permission-spike.md`：5/9 全绿；4 偏差落档；评议接纳所有缓解 |
| P0-T05 | ✅ 完成 | `docs/extensions/msgpack-rpc-spike.md`：P99 18-30μs，目标 5ms（250× 冗余）；FU-4 重跑 fd 3 待执行 |
| P0-T06 | ✅ 完成 | `crates/cronymax/src/extensions/` 骨架；`cargo build -p cronymax` 0 warning |
| P0-T07 | ✅ 完成 | Phase 0 评议 PASS；Phase 1 立刻启动（FU-1..7 与 P1 并行）|

### Phase 0 评议决议（已落档；详 `phase-0-review.md`）

- §A `--allow-net` v1 boolean，manifest network.allow 仅人话授权用 ✅
- §B `build_node_flags` 永远 emit `--no-warnings` + 平台 audit log 结构化记录 high-risk flag ✅
- §C **重决**：RPC 改 inherited fd 3，`--allow-net` 还给用户作为 capability ✅
- §D **重设**：manifest fs 改 `[{path, mode}]` 数组 + 平台变量集（`{WORKSPACE}` / `{HOME}` / `{EXT_STORAGE}` 等）+ 平台展开 canonicalize 双填 ✅
- §E 性能指标改字（P2-T09 / spike checklist #8）；**不**加 workspace.glob（avoid speculative API） ✅
- 扩展日志系统纳入 v1（`extension-logs.md` v0.2）+ `createOutputChannel` SDK ✅
- `cep-idl/v1/logging.ts` + `manifest.ts` fs schema 变更作 v1 freeze 单次例外（v1 未 ship 前可补）✅
- §3.4 SDK 公网 npm（`@cronymax/extension`）✅
- §3.5 coco binary 走 PATH 找 ✅
- §3.6 不发 Phase 7 alpha，等 Phase 10 一次发 ✅
- §0.1 DRI = 单人 ✅；IDL review 政策 = AI 协助 self-review + 决策理由文档化
- §4 Phase 1 准入 **PASS**

### 进 Phase 1 的硬指标（最终）

| 项 | 状态 |
|---|---|
| spec-v0.3 + plan-v0.3 + tasks-v0.3 评审定稿 | ✅ Phase 0 评议 2026-05-20 |
| IDL v1 freeze + review | ✅ AI self-review 政策（单人项目妥协）；FU-7 输出 review 报告 |
| Node 26 spike 报告全 ✅（含偏差缓解决策）| ✅ |
| MessagePack-RPC P99 < 5ms | ✅（实测 18-30μs，250× 冗余）|
| flow legacy brief 完成 | ✅ |
| DRI 7 项决策签字 | ✅ 全部决议 |
| 团队人员对齐 | ✅ 单人项目；估时拉长到 14-16 周 |

---

## Phase 1 执行进度（2026-05-20）

| ID | 状态 | 备注 |
|---|---|---|
| P1-T01 | ✅ 完成 | `manifest.rs` serde 反序列化；`FsCapability::mode` 改 enum；10 单测 |
| P1-T02 | ✅ 完成 | `Manifest::validate` 5 个子函数 + `PLATFORM_VARS` 表；新增 3 个 `ExtensionError` 变体；29 单测 |
| P1-T03 | ✅ 完成 | `ExtensionRegistry` install/list/refresh/enable/disable/uninstall；`registry.json` tmp+rename 原子写；15 单测 |
| P1-T04 | ✅ 完成 | `ActivationEvent::parse` + `Trigger::matches` + `parse_all`；validate 联动；13 单测 |
| P1-T05 | ✅ 完成 | 新增 `cronymax` 二进制（`src/bin/cronymax.rs`），hand-rolled argv，免 clap 依赖 |
| P1-T06 | ✅ 完成 | `tests/p1_acceptance.rs`：纯声明扩展走完整 install→list→disable→enable→uninstall 生命周期；4 集成测试通过 |

### Phase 1 验证

- `cargo test -p cronymax --lib extensions::` → **67 passed**（manifest 39 + activation 13 + registry 15）
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt -p cronymax --check` → clean
- `cargo run -p cronymax --bin cronymax -- --help` → 输出正常

### Phase 1 顺手修复

- `runtime/handler.rs:3003,3119`：移除 `Arc::new(RuntimeServices::new_minimal(...))` 双包（baseline 上阻塞 `cargo test --lib` 的旧 bug，新 manifest::tests 触发后定位）

---

## Phase 2 执行进度（部分 · 2026-05-20）

无外部依赖、单 session 内可完整覆盖单测的 P2 任务：

| ID | 状态 | 备注 |
|---|---|---|
| P2-T01 | ⏸ 待办 | Node 26 多平台二进制打包 — CI/构建基础设施工作 |
| **P2-T02** | ✅ 完成 | `capability.rs::build_node_flags` + `ExpansionCtx` + canonicalize 双填；18 单测含 macOS symlink 验证 |
| P2-T03 | ⏸ 待办 | Node host spawn — 阻塞于 P2-T01（无 Node 二进制无从测试 fd 3 stdio） |
| **P2-T04** | ✅ 完成 | `rpc::codec` + `rpc::server` —  MessagePack-RPC Request/Response/Notify + `$/cancel` cancellation；18 单测含 partial frame、concurrent dispatch、token propagation |
| P2-T05 | ⏸ 待办 | `extension-host-bootstrap.js` — TS 工具链工作 |
| **P2-T06** | ✅ 完成 | `api/lifecycle.rs` (`LifecycleState`) + `api/commands.rs` (`CommandRegistry`)；15 单测含 `cronymax.*` 命名空间拒、跨扩展冲突拒、unregister_all_for |
| P2-T07 | ⏸ 待办 | `@cronymax/extension` SDK npm publish — 需外部 npm token |
| P2-T08 | ⏸ 待办 | hello-world 测试扩展 — 依赖 P2-T05/T07 |
| P2-T09 | ⏸ 待办 | 性能基准 — 依赖完整链路（P2-T01..T08） |
| P2-T10 | ⏸ 待办 | 安全冒烟测试 — 同上 |
| **P2-T11** | ✅ 完成 | `extensions/logging.rs`：`LogManager` + `LogWriter` + `AuditWriter` + 6 层 A-F 事件常量；12 单测含 size-based 滚动、`max_history` 截断、并发写序列化、channel 名 sanitize |
| P2-T12 | ⏸ 待办 | SDK `window.createOutputChannel` — TS 工作 |

### Phase 2 新增 workspace 依赖

```toml
rmp-serde = "1.3"
rmpv = { version = "1", features = ["with-serde"] }
```

两者都源自 Phase 0 RPC spike 已验证的选型（spike §3.2、§6 决策表）。

### Phase 2 验证（部分）

- `cargo test -p cronymax --lib extensions::` → **130 passed**（manifest 39 + activation 13 + registry 15 + capability 18 + logging 12 + rpc::codec 12 + rpc::server 6 + api::lifecycle 5 + api::commands 10）
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt --check` → clean
- 已完成 4/12 P2 任务（非阻塞于外部基础设施和 TS 工具链的全部 Rust 任务）

### Phase 2 接下来的解锁顺序

1. **P2-T01**（Node 二进制打包）→ 解锁 P2-T03 测试
2. **P2-T03**（Node host spawn）→ 解锁 P2-T08 / T09 / T10
3. **P2-T05 + P2-T07 + P2-T12**（TS 工具链一起做）→ 解锁 P2-T08
4. **P2-T08..T10**（验收 + 基准 + 冒烟）→ Phase 2 收尾

---

## Phase 3 执行进度（部分 · 2026-05-20）

Phase 3 任务依赖 P2-T06（已完成）—— 状态层都可在无 Node host 的情况下提前落地。

| ID | 状态 | 备注 |
|---|---|---|
| P3-T01 | ⏸ 待办 | events 跨进程路由 — 需 RpcClient（P2 后续） |
| **P3-T02** | ✅ 完成 | `api/config.rs`：`ConfigStore` + on_change 订阅 + drop-unsubscribe Guard + 同值不重复 fire；12 单测 |
| P3-T03 | ⏸ 待办 | secrets — macOS Keychain / Win DPAPI / Linux secret-service 集成 |
| **P3-T04** | ✅ 完成 | `api/storage.rs`：`ExtensionStorage` Workspace/Global 双 scope + tmp+rename 原子写 + 跨实例持久化 + 跨扩展隔离；10 单测 |
| P3-T05 | ⏸ 待办 | webview stub — 留待 P6 webview 基建一起做 |
| P3-T06 | ⏸ 待办 | auth.rs — 内置 OAuth/PKCE/device-flow |
| **P3-T07** | ✅ 完成 | `api/extensions.rs`：`ExportsRegistry` + `view_of` 三源 join（registry + lifecycle + exports）；9 单测 |
| P3-T08 | ⏸ 待办 | SDK 类型补齐 — TS 工作 |
| P3-T09 | ⏸ 待办 | 验收测试扩展 — 依赖 P3-T08 + Node host |

### Phase 3 验证（部分）

- `cargo test -p cronymax --lib extensions::` → **194 passed**（更新后）
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt --check` → clean

---

## 后续累计完成（2026-05-20 第三批）

### RPC 双向 Connection 重构（解锁事件路由 + ping/pong）

`rpc/server.rs` 改成纯 handler 表（去掉 `run` 方法）；新增 `rpc/connection.rs`：

- `Connection::open(reader, writer, server) → (Arc<Connection>, JoinHandle)` 单流双向
- 入站 Request 走 handler 表 + CancellationToken，Response 走 pending map（`oneshot::Sender`），Notify 仅处理 `$/cancel`
- 出站 `request/notify/cancel` API
- 11 单测覆盖：双向 round-trip / unknown method / handler error / 64 KB payload / 32 并发请求 / cancellation token / 连接断开时 pending 失败

### P3-T03 secrets ✅

`api/secrets.rs`：
- `SecretBackend` trait + `SecretStore`（`os_default` / `in_memory`）
- **macOS**：`security-framework::passwords` 集成，`KEYCHAIN_SERVICE = "ai.cronymax.extensions"`
- **Linux/Win**：in-memory fallback + warn（后续集成 secret-service / DPAPI）
- `ExtensionSecrets::for_extension(ext_id, namespace)` —— key 强制 `<namespace>.` 前缀；空 key / NUL 字符拒
- 10 单测含 namespace 隔离、sub-namespace 独立、store clones 共享 backend

### P3-T06 auth ✅

`api/auth.rs` 纯数据层（不引入 HTTP 客户端）：
- **PKCE**：`PkcePair::new_random` + `from_verifier`，S256-only，**RFC 7636 §B.1 测试向量验证通过**
- **CSRF state**：`AuthState` URL-safe base64，constant-time `verify`
- **Device flow**：`DeviceFlowSession::apply_poll` 状态机，`DevicePoll::SlowDown` 自动 +5s 退避；`expires_at` 检测
- **Session store**：in-memory `HashMap<(ext_id, provider_id, session_id), AuthSession>`，RwLock 共享
- 18 单测含 RFC 7636 测试向量、constant-time eq、device flow 6 种 poll outcome、session 过期、b64url 边界

### P2-T05 extension-host-bootstrap.js ✅

`crates/cronymax/bundled/extension-host-bootstrap.js`（371 行）：
- fd 3 → `new net.Socket({ fd: 3 })`，无需 `--allow-net`
- `@msgpack/msgpack` 流式 decoder（累积缓冲 + `Decoder.decodeMulti`）
- 双向 RPC：`rpcRequest/notify` 出站，`registerRpcHandler` 入站
- **EH Layer B**：`console.log/info/warn/error/debug` 拦截 → 同写 stdout/stderr + `log/console` notify
- **EH Layer C/E/F**：`uncaughtException` / `unhandledRejection` 全局 handler，带 phase 信息（activate vs running）
- **SDK shim**：`globalThis.cronymax = { window, commands, workspace, extensions, ExtensionMode }`，window 含 `createOutputChannel` + `show*Message`，commands 含 `register/execute`，workspace 含 `getConfiguration`
- **生命周期**：`extension/activate` 加载 `manifest.main`、调用 `activate(ctx)`、audit `activate.ok/failed`；`extension/deactivate` 反向跑 `subscriptions[].dispose()`
- **handshake**：启动末尾发 `$/ready` notify（平台凭此切到 Active）
- `node --check` 语法验证通过

### Phase 2 / Phase 3 累计完成度

| ID | 状态 | 备注 |
|---|---|---|
| P2-T05 | ✅ | bootstrap.js 371 行，`node --check` 通过 |
| P3-T03 | ✅ | macOS Keychain + in-memory fallback，10 单测 |
| P3-T06 | ✅ | PKCE + Device Flow + Session Store，18 单测，RFC 7636 向量 |
| RPC refactor | ✅ | Connection 双向，11 新单测 |

### 仍需外部基础设施的剩余项

- **P2-T01** Node 26 二进制多平台打包（CI 工作）
- **P2-T03** Node host spawn（阻塞 P2-T01）
- **P2-T07 / P2-T12 / P3-T08** TypeScript SDK + npm publish
- **P2-T08 / T09 / T10** dogfood 扩展 + 基准 + 安全冒烟（阻塞 P2-T01 + P2-T03 + P2-T07）
- **P3-T01** 事件 pub/sub 跨进程路由（现已有 Connection 双向接口，下次可推）
- **P3-T05** webview stub（留 P6 一起做）
- **P3-T09** 验收测试扩展（需 SDK）

---

## 后续累计完成（2026-05-20 第四批：阻塞项推进）

| ID | 状态 | 备注 |
|---|---|---|
| **P2-T01** (部分) | ✅ | `scripts/fetch-node26.sh` 单平台 Node 26 下载脚本（macOS arm64 起步，自动检测 OS/arch）；多平台 CI 工作未做 |
| **P2-T03** | ✅ | `extensions/host/node.rs`：`NodeHost::spawn` + socketpair + `pre_exec` dup2 fd 3 + UnixStream → Connection 集成 + ping/pong health monitor + SIGTERM/SIGKILL graceful shutdown；6 单测（mock 用 `/bin/true` / `/bin/sleep` + env-dump shell 脚本） |
| **P2-T05** | ✅ | bootstrap.js 已存在；本批仅修正 `commands.register/execute` 命名对齐 IDL |
| **P2-T07** | ✅ | `sdk/extension/` TypeScript 包：14 个 IDL `.ts` + `runtime.ts` globalThis shim；tsc 编译产出 `dist/`，0 type errors |
| **P2-T08** | ✅ | `examples/hello-world/`：30 行 TS extension，注册命令 + 弹消息；用 `@cronymax/extension` 包成功编译 |
| **P2-T12** | ✅ | `window.createOutputChannel` + `LogOutputChannel` 已在 bootstrap.js + SDK 内 |
| **P3-T01** | ✅ | `extensions/events.rs`：`EventBus` 含 capability 白名单 gate + `EmitPattern::{Exact, Prefix}` 通配符 + `SubscriptionGuard` drop-自动 unsubscribe + 平台/扩展双向 emit；13 单测含 `cronymax.*` 拒、wildcard cap、cross-extension routing |
| **P3-T05** | ✅ | `api/webview.rs`：`WebviewRegistry` + `PanelSlot` enum + 所有权 / dispose / set_visible / list_for；9 单测 |

### 跨任务

- **`forbid(unsafe_code)` → `deny(unsafe_code)`**：fd 3 inheritance 在 host 模块需要 `pre_exec` + `dup2` + `from_raw_fd`，三个本质 unsafe。改成 `deny` 后整个 crate 只有 `extensions::host::node` 这一个模块 opt-in `#![allow(unsafe_code)]`。
- 新增 workspace 依赖 `libc = "0.2"`，开启 `nix` 的 `socket` feature。

### 最终验证

- `cargo test -p cronymax --lib extensions::` → **222 passed**（manifest 39 + activation 13 + registry 15 + capability 18 + logging 12 + rpc 23 + api 88 + events 13 + host 6 + webview 9）
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt --check` → clean
- `node --check bundled/extension-host-bootstrap.js` → OK
- `tsc -p sdk/extension/tsconfig.json` → 0 errors，产出 `dist/`
- `tsc -p examples/hello-world/tsconfig.json` → 0 errors，产出 `dist/main.js`
- `bash -n scripts/fetch-node26.sh` → OK

### 真正还做不到的（需外部资源）

- **npm publish 真实发布** —— 需要 npm 账号 + token
- **真实 OAuth IdP 集成测试** —— 需要 GitHub/Google 等 client credentials
- **跨平台 Node 26 二进制**（macOS x64 / Linux x64 / Win x64）—— 需要对应 CI 平台或交叉编译
- **OS Keychain 集成测试** —— macOS Keychain 已实现，Linux secret-service / Win DPAPI 需要平台 + 真实环境
- **Phase 7 γ 阶段** sandbox-exec / seccomp / AppContainer —— 平台特有，alpha 之后再做

---

## 后续累计完成（2026-05-20 第五批：跑通真实 Node 26 + 双向 RPC + 多 root workspace）

| 项 | 状态 | 备注 |
|---|---|---|
| **Node 26 二进制下载实装** | ✅ | `scripts/fetch-node26.sh` 修正 msgpack 安装位置；macOS arm64 v26.1.0 + `@msgpack/msgpack` 已落盘到 `crates/cronymax/bundled/{node,node_modules}/` |
| **RPC notify 入站 dispatch** | ✅ | `RpcServerBuilder::on_notify`；`Connection::dispatch` 把入站 Notify 派给注册的 handler（先前只处理 `$/cancel`） |
| **AgentProvider 协议落档** | ✅ | `rpc/codec.rs::agents_method` 8 个 method 常量；`api/agents.rs::AgentProviderRegistry`（namespace gate + ownership + multi-extension support），8 单测 |
| **真实 Node 26 端到端测试** | ✅ | `tests/p2_node_host_e2e.rs` 4 个集成测试：spawn → `$/ready` → activate → audit；`/etc/passwd` 真被拒并通过 `log/console` 上报；workspace folders 数组到达扩展（多 root + 零 root） |
| **Workspaces default-rw 重设计** | ✅ | manifest **不再申报** workspace；平台对每个 root 自动 emit canonical rw；`ExpansionCtx.workspaces: Vec<PathBuf>`；`SpawnConfig.workspace_dirs: Vec<PathBuf>`；env `CRONYMAX_WORKSPACE_FOLDERS=<JSON 数组>`；bootstrap.js + SDK 暴露 `workspaceFolders: WorkspaceFolder[]` + `rootUri` 便捷字段；零 workspace 时 `workspaceFolders === []` 而非 null |
| **Canonical-only emission** | ✅ | spec §6.1 原"expanded → canonical 双填"撤回；只 emit canonical；扩展契约性地用 `ctx.*Path` / env（已 canonical），硬写 `/tmp/...` 这种 symlink 路径是 dev 的 bug |

### 关键设计决策

1. **Workspace 是隐式 capability**（spec §6.1 + §6.2 修订）。manifest 不申报，安装弹窗不列。任何已装扩展自动获得当前 / 未来加入的所有 workspace root 的 rw。
2. **Workspace 变更走方案 A：重启扩展 host**（Node 26 Permission flags spawn 后不可变；唯一干净的处置）。spawn 接口已支持每次新 `workspace_dirs`；触发重启的胶水代码归 P4 / P6 chat panel 接通时做。
3. **Canonical-only path emission** 替代 "双填"。开发者通过 `ctx.workspaceFolders[0].uri` / `ctx.storagePath` 等拿到的都是 canonical，根本撞不上 `/tmp` vs `/private/tmp` 的 realpath 解析坑。

### Node 26 Permission Model 实测要点

- `realpathSync` 在 `require()` 内部对每个**祖先符号链接**单独施加 fs-read 权限检查。macOS `/tmp` `/var` `/etc` 都是符号链接 → 硬写这些路径需要单独授权 `/tmp` 本身，granular path prefix 不够
- 解决路径：扩展只用 platform-supplied canonical 路径
- 测试里使用 `--allow-fs-read=*` 是粗暴绕过；production 不该这么做

### 最终验证（截至第五批）

- `cargo test -p cronymax --lib extensions::` → **231 passed**
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo test -p cronymax --test p2_node_host_e2e` → **4 passed**（真实 Node 26 + bootstrap.js 全链路）
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt --check` → clean
- `node --check bundled/extension-host-bootstrap.js` → OK
- `tsc -p sdk/extension/tsconfig.json` → 0 errors
- `tsc -p examples/hello-world/tsconfig.json` → 0 errors
- `crates/cronymax/bundled/node/bin/node --version` → `v26.1.0`

### Phase 完成度概览（截至 2026-05-20）

| Phase | 完成 / 总数 | 状态 |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成 |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf bench / T10 安全冒烟 仍需 dogfood 全链路 |
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4 L2 EP × 6（chat panel 路由）| 0 / ? | 未启动（AgentProviderRegistry 已就位） |
| Phase 5 L1.5 平台事件 emit | 0 / 4 | 未启动（EventBus 已就位） |
| Phase 6 Webview 基建 | 0 / ? | 未启动（stub 在 P3-T05） |
| Phase 7 coco dogfood | 0 / ? | 未启动 |
| Phase 8 Flow agent 接通 | 0 / ? | 未启动 |
| Phase 9 SDK + 扩展管理 UI | 0 / ? | 未启动 |
| Phase 10 收尾 + Alpha | 0 / ? | 未启动 |

### 到 coco extension 能开发的距离

**层级 A（写源码 + 装到本地 cronymax + 跑命令）**：✅ 已经齐了。`scripts/fetch-node26.sh` 已经下载好 Node 26；hello-world 已经跑通；扩展可以用 fs / process / network / secrets / window outputChannel。

**层级 B（coco 作为 chat panel agent provider 被调起）**：仍需以下 3 块（都需要进 web/ 子项目）：
1. chat panel 读 AgentProviderRegistry.list() + 渲染 provider 选择（P4，React + GIPS）
2. Phase 8 chat / flow runtime 共享 AgentProvider 抽象的 refactor
3. `window/showInformationMessage` 平台 handler → React toast（P3-T05 / P4 一起）

**workspace 切换重启编排**：spawn 接口已支持；缺主进程订阅 cronymax 桌面 workspace 状态变更并触发 `host.shutdown().await + NodeHost::spawn(new_cfg)`。归 P4 一起做。

---

## 后续累计完成（2026-05-20 第六批：撤回 Node 26 Permission Model）

经过 dogfood UX + workspace-切换难题 + Node 26 spawn-time-only flag 的实测三方面评估，**v1 alpha 整套撤回 Node 26 Permission Model**。详见 [`permission-removal.md`](permission-removal.md) 决策记录。

### 主要改动

| 文件 | 改动 |
|---|---|
| `capability.rs` | `build_node_flags` 缩成只 emit `--no-warnings`；删 `PLATFORM_VARS` / `expand_path` / `canonicalize_best_effort` 等 fs 翻译逻辑（**461 → 110 行**，19 测试 → 2 测试） |
| `manifest.rs` | `Capabilities` 退化为接受任意 JSON 的 inert 字段；删 `FsCapability` / `FsMode` / `NetworkCapability` / `SecretsCapability` 等枚举；validator 从 5 子函数缩到 4 个（required / id format / `cronymax` publisher 保留 / activation event 解析）；删 29 个 fs path validator 测试 |
| `error.rs` | 删 `FsPathInvalid` / `ContributionNotDeclared` 两个变体 |
| `logging.rs` | 删 `AuditWriter` 类型 + `audit` 模块（10 事件常量）+ `LogKind::ExtensionAudit` 变体 + `LogManager::audit_writer` 方法 + 2 个 audit 测试 |
| `bundled/extension-host-bootstrap.js` | 删所有 `rpcNotify("audit", ...)` / `rpcNotify("log/console", ...)` / `rpcNotify("log/error", ...)`；activate 不再 try/catch + audit；console 拦截只写 stdout/stderr |
| `tests/p2_node_host_e2e.rs` | 删 `fs_permission_denial_propagates_to_bootstrap_log` 测试；`CapturedNotifies` 缩到只追 `ready`；node_flags 缩到 `["--no-warnings"]` |
| `spec-v0.3.md` | 顶部加 v1-alpha 修订 banner；§6 整章重写（从"capability gate"改成"install-time author trust"）；§7 γ 阶段改为可选未来工作；§8 schema 删 `capabilities` 字段；§10 / §11 同步 |
| `docs/extensions/permission-removal.md` | **新增**决策记录 |

### 净影响

- **删码 ~1500 行**
- 196 测试通过（189 lib + 4 acceptance + 3 e2e）
- 0 clippy warning / fmt clean / bootstrap.js syntax OK

### 撤回后的信任模型

| 项 | 现状 |
|---|---|
| 扩展进程 Node API | 完整（fs / process / network / workers / native addons 全开） |
| `--permission` 标志 | 不 emit |
| `--allow-*` 标志 | 不 emit |
| 安装弹窗 | 只问"由 \<publisher\> 提供，是否安装" —— 无 per-cap 列表、无风险提示 |
| Audit log | 撤回。`host.log` / `output.log` / `extension-host.log` 仍记录操作排错用途（不是 security audit） |
| 信任边界 | 用户在 install-time 信任扩展作者 |
| `cronymax.*` 命名空间 | 仍保留（platform-RPC 路由层；不是 OS 强制） |
| Per-extension host 崩溃隔离 | 保留（cronymax 相对 VS Code 的差异化卖点） |

### Workspace 切换问题消失

Node 26 flag 不可变约束没了 → 加 / 删 / 切 workspace folder 都只是 RPC notify，扩展自己响应；**不再需要 host 重启编排**。之前讨论的 idle / busy 排队、host restart-on-workspace-change 全部撤销。

### Phase 完成度（更新）

| Phase | 完成 / 总数 | 状态（撤回 permission 后） |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成（validator 大幅简化） |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf / T10 安全冒烟（安全冒烟语义改变了 — 现在测的是"扩展能正常用，不是被 deny"） |
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4-10 | 未启动 | 同 |

---

## Phase 4 执行进度（2026-05-21）

P4 重点是 wiring：Phase 0–3 已把每个 L2 EP 的 typed registry 建好，但全没接 RPC。本批把它们全接通，并落下 top-level orchestrator。

| ID | 状态 | 备注 |
|---|---|---|
| **P4-T01** | ✅ 完成 | `contributions/mod.rs::ingest()` 遍历六个 `contributes.*` 字段，序列化进 `entries: HashMap<ep_id, HashMap<ext_id, ContributionEntry>>`；`remove_extension(ext_id)` 反向清理；10 单测 |
| **P4-T02** | ✅ 完成 | `api/renderers.rs` (`ContentRendererRegistry`) + `api/sidebar.rs` (`SidebarViewRegistry`)：跟 `AgentProviderRegistry` 同 shape——`Arc<RwLock<HashMap>>` + 命名空间 gate + ownership + `unregister_all_for`；8 + 6 单测 |
| **P4-T03 / P4-T04** | ✅ 完成（合并一次完成） | `extensions/runtime.rs`：`ExtensionRuntime` orchestrator 持有六个 typed registry + 每扩展 NodeHost。`activate()` 路径：(1) 读 manifest (2) ingest contributions (3) build per-extension `RpcServer` 注册 8 个 notify handler (4) spawn host (5) 填 `LateConn` slot (6) RPC 调 `extension/activate` 等结果 (7) mark lifecycle。`deactivate()`：调 `extension/deactivate` → 清六个 registry → kill host。 |
| **P4-T05** | ✅ 完成 | `error.rs` 加 `NotActivated` + `AlreadyActivated`；`LifecycleState::mark_activated/_deactivated` 切到新变体 |
| **P4-T06** | ✅ 完成 | `tests/p4_extension_runtime_e2e.rs`：合成 manifest 申报 4 个 L2 EP，扩展 `activate()` 里调 `cronymax.commands.register / cronymax.agents.registerProvider / cronymax.renderers.registerRenderer / cronymax.sidebar.register`，验证 4 个 typed registry 都观察到注册；`deactivate()` 全清 |
| **P4-T07** | ✅ 完成（本节）|  |
| P4-T08 | ⏸ 待办 | permissionRequest 事件桥接平台权限弹窗——需要 chat panel UI 接通 |

### Phase 4 关键设计选择

1. **`ExtensionRuntime` 是 single owner**：六个 typed registry + ExtensionRegistry + LifecycleState 全归它持有，外部（chat panel / flow runtime / settings UI）只通过它访问。这是 plan §2 "每个 L2 EP 走 `contributions/`" 的工程化落点。
2. **`LateConn` 解 wiring 死循环**：`RpcServer` 必须在 `NodeHost::spawn` 之前 build（spawn 接收 RpcServer），但 register-notify handler 又需要 spawn 后才存在的 `Arc<Connection>` 去填进 `ProviderEntry.conn`。方案：在 build 阶段把一个空 `LateConn` slot 借给闭包，activate() 拿到 conn 后立刻 `slot.set(conn)`。Handler 第一次 fire 时（必然在 `extension/activate` 之后）一定能读到。
3. **register-notify 找 manifest declaration**：扩展 `cronymax.agents.registerProvider("alice.x.gpt", impl)` 时只发 `{ providerId }`，不发 label/icon/supports_*。Runtime 从 `manifest.contributes.agent_providers` 里 lookup 取这些字段。这保证两件事：(a) 扩展运行时改不了 manifest 申报的 metadata（防混淆）（b) 未在 manifest 申报就 register 的会被拒（`BadContribution` 错误，silent drop notify）。
4. **`activate()` 同步等 `extension/activate` RPC**：早期版本 host spawn 完就 return，让调用方决定何时 kick activate。改为 runtime 内部直接调，因为：(a) 调用方真正想要的语义是"扩展能用了" (b) 不调 activate 的话扩展永远不会发 register notify (c) 失败时回滚 contributions 干净

### Phase 4 验证

- `cargo test -p cronymax --lib extensions::` → **223 passed**（189 baseline + 10 contributions + 8 renderers + 6 sidebar + 10 runtime）
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo test -p cronymax --test p2_node_host_e2e` → **3 passed**（撤回 permission 后剩余）
- `cargo test -p cronymax --test p4_extension_runtime_e2e` → **1 passed**（真实 Node 26 + 合成 alice.p4 扩展全链路 activate→register×4→deactivate→clear）
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0 warnings
- `cargo fmt --check` → clean
- `node --check bundled/extension-host-bootstrap.js` → OK（新增 `agents.registerProvider` / `renderers.registerRenderer` / `sidebar.register` SDK shim）

### Phase 4 接下来还能推什么

- **P4-T08** permissionRequest 事件桥接：需要 chat panel React + tool 调度 emit `permission/needRequest`，runtime 路由到 platform 弹窗。归 P5 chat panel 接通时一起做
- **chat panel UI**：从 `runtime.providers().list()` 渲染 provider picker；这是 V2 验收清单的内容
- **flow runtime agent step**：`flow/agent_step.rs` 改成查 `runtime.providers().get(id)`——V5/V6 验收清单

### Phase 完成度（Phase 4 接入后）

| Phase | 完成 / 总数 | 状态 |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成 |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf / T10 安全冒烟（撤回 permission 后语义改变）|
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4 L2 EP × 6 wiring | 7 / 8 | ✅ T01-T07 主体完成；T08 permissionRequest 等 chat panel 接通 |
| Phase 5-10 | 未启动 | 同 |

---

## Phase 4 follow-ups（2026-05-21 下午）

讨论关键设计时发现两个不满意点；都改了。

### F1 + F2：删 `LateConn`,Entry 不再持 conn

**问题**：`ProviderEntry.conn: Arc<Connection>` 引入了 chicken-and-egg —— handler 闭包要在 spawn 之前 build,但 conn 要在 spawn 之后才有。`LateConn` 是个 set-once slot 的解法,但是时序耦合脆。

**解法**：entry 不再持 conn。chat panel / flow 通过 `runtime.send_to_extension(ext_id, method, params)` / `runtime.notify_extension(...)` 发请求,runtime 内部从 `state.handles[ext_id]` lookup 当前的 conn。activate() 流程改成 spawn 后**立即**把 handle 塞进 state.handles(而不是等 activate RPC 返回之后),这样 handle.conn 在 register-notify handler fire 时已经可用。失败时 rollback 把 handle 拿出来 shutdown。

**改动**:
- `ProviderEntry` / `RendererEntry` / `SidebarViewEntry` 都删 `conn: Arc<Connection>` 字段
- runtime.rs 删 `LateConn` 结构和 `late_conns` map
- 加 `send_to_extension(ext_id, method, params) → Result<Value>` 和 `notify_extension(...)` 公共 API
- `state.handles.insert` 时机从 "activate RPC 成功后" 提前到 "spawn 完成后";rollback 路径加 `rollback_failed_activate`
- 加 `NodeHost::dummy_for_test()` 让单测能注入 conn-only handle

### F3：反向 `extension/registerError` notify

**问题**：register-notify 处理失败时只是 silent drop,扩展开发者完全看不到。

**解法**:
- 加新 RPC method 常量 `method::EXTENSION_REGISTER_ERROR = "extension/registerError"`
- 每个 register handler 失败时通过 `report_register_outcome(...)` 反向发 `{ ep, id, reason }` notify 给扩展进程
- bootstrap.js 注册 `extension/registerError` handler,`console.error` 出来,进 stderr,落 extension-host.log
- bootstrap.js 同时拓展 type=2 inbound notify dispatch,过去只处理 `$/cancel`,现在所有 inbound notify 都 lookup handlers 表

### 顺手发现并修复:bootstrap.js msgpack decoder.pos bug

**症状**：`decoder.decodeMulti(buf).next()` 之后 `decoder.pos | 0` 返回的不是真实消耗字节数 —— 短方法名(如 `extension/activate` 18 字符)碰巧吻合,长方法名(如 `commands/execute:alice.p4.hi` 28 字符)就报 23 字节消耗实际 33 字节,导致 10 字节 stale data 残留 buffer,下一帧解码错位变成 `notify undefined`。

**修复**：换成 `for (const frame of decoder.decodeMulti(buf))` 迭代,每次成功 yield 后用 `decoder.pos` 更新一个本地 `consumed` 累加器。`Decoder` 也改成每次 on(data) 创建新实例避免跨调用状态污染。Iterator 抛 RangeError 表示需要更多 bytes,break 出循环并 trim 已 consumed 部分。

### 验证（最终）

- `cargo test -p cronymax --lib extensions::` → **226 passed**(+3:新增 send_to_extension / notify_extension not-activated 测试 + registerError emit 测试 - 原 silent-drop 测试改写)
- `cargo test -p cronymax --test p1_acceptance` → **4 passed**
- `cargo test -p cronymax --test p2_node_host_e2e` → **3 passed**
- `cargo test -p cronymax --test p4_extension_runtime_e2e` → **2 passed**(新增 registerError e2e + 原 e2e 加 send_to_extension round-trip)
- `cargo clippy -p cronymax --bins --lib --tests -- -D warnings` → 0
- `cargo fmt --check` → clean

合计 **235 tests pass**。

---

## Phase 4.5 执行进度（2026-05-22 · chat-provider 接通）

P4 把六个 L2 EP 的 typed registry 接通到 RPC,但 `AgentProviderRegistry` 仍是"列得出、用不了"——聊天面板没列、`StartRun` 不路由。本批把「聊天面板 → 扩展 AgentProvider」整条链路打通并测过,对应 P4-T05 验收项「聊天面板列扩展贡献的 provider」。分 4 个 commit 落在 `feat/plugins`(`ba92601` / `2ca97aa` / `7050120` / `121fedb`)。

| 子项 | 状态 | 备注 |
|---|---|---|
| 聊天面板列 provider | ✅ 完成 | `AgentRegistryList` 控制响应追加 `kind:"extension_provider"` 条目;`RuntimeServices` 启动时从 `~/.cronymax/extensions/` 自建 `ExtensionRuntime`(与 CLI 共用 `default_registry_root()`);web `AgentSummary` schema + `agentPickerDescription()` 渲染 |
| dispatch 安全护栏 | ✅ 完成 | `StartRun` 命中扩展 provider 时不再 silent fallback 到 `load_agent_with_builtin` 占位 AgentDef |
| bootstrap.js session 生命周期 | ✅ 完成 | `session.create` 暂存 session 到 map;新增 `session.prompt`(迭代 + 发 `agents/event` + done 兜底 + 错误 trap)/ `session.dispose` / `session.cancel` / `session.resolvePermission` 四个 handler;`$/cancel` 标志桥接成 IDL `CancellationToken` |
| 入站事件路由层 | ✅ 完成 | `AgentSessionEvent` 类型化枚举(IDL §AgentEvent 1:1)+ `AgentSessionRouter`(session_id → mpsc sink)+ 每扩展 RPC server 注册 `agents/event` / `agents/turn.done` 入站 notify handler;`rmpv_to_json` / `json_to_rmpv` 互转 helper |
| chat dispatcher | ✅ 完成 | `runtime/ext_dispatch.rs::drive_extension_session`:session.create → 注册 sink → 后台 session.prompt → 事件循环译成 `RuntimeEventPayload`(text→Token / thinking→ThinkingToken / toolCall→Trace / done→run 状态)→ session.dispose → complete/fail run;`StartRun` 命中扩展 provider 时 spawn 它,绕过 legacy ReactLoop |
| 端到端联调测试 | ✅ 完成 | duplex 假扩展 peer 跑通 happy path(text→Token、turn.done→Succeeded、sink 清理)+ session.create 出错→run Failed |
| ResumeRun 护栏 + agent_id 持久化 | ✅ 完成 | code review 发现 `handle_resume_run` 无对称护栏:扩展 chat run 重启后被 `rehydrate` 转 `Paused`,resume 会走 native ReactLoop 静默串台。修复:`handle_start_run` 把 `resolved_agent_id` 镜像进 `Run.spec`(根因——agent 身份此前根本没持久化),`handle_resume_run` spec 优先解析 + 命中扩展 provider 时返回 `InvalidState`(在 `mark_run_running` 前,留 `Paused` 不变孤儿)|

### Phase 4.5 关键设计选择

1. **激活前置条件,不做懒激活**:`StartRun` 命中未激活扩展的 provider 时返回 `ControlError::InvalidState`(消息点名 provider + owning_ext),而非自动 spawn。懒激活需要 bundled Node 路径解析 + `build_node_flags` 生产接线,是独立一刀,且夹着"打包后 Node 放哪"的打包决策。
2. **session.prompt 走后台 task**:bootstrap.js 的 `session.prompt` handler 迭代抽干后才返回,若与事件循环同 task await 会死锁——入站 notify 需要并发 pump。dispatcher 先 `register` sink 再发 prompt,关掉"事件先于 await 恢复到达"的竞态。
3. **`AgentSessionRouter` 挂 runtime 作用域**:wire 格式按 sessionId 路由(不是 owning_ext),所以 router 是跨扩展共享的单例;sink 缺失(取消竞态 / 陈旧 dispatcher)按 debug 日志丢弃,不报错。
4. **flow 路径暂不接**:带 `flow_id` 的 run 跳过扩展 dispatch——flow runtime 有自己的 per-step provider 查找路径(P8)。
5. **agent 身份必须持久化进 `Run.spec`**:`handle_start_run` 给 `start_run_with_session` 的 typed `agent_id` 传 `None`(那个槽位是给持久化 Agent 实体的),StartRun 控制字段又不进 payload——结果 `Run` 上没存 agent 身份,resume 永远回退 Crony。修复是把 `resolved_agent_id` 镜像进 `payload["agent_id"]`;这同时也修好了 native 非 Crony agent 的 resume 身份丢失。`handle_start_run` / `handle_resume_run` 两个入口都要有 extension-provider 护栏——dispatch 抽象只要有第二扇没上锁的门,silent 串台就会从那里漏进来。

### Phase 4.5 后续:merge origin/main + ResumeRun 护栏(2026-05-22)

`feat/plugins` 落后 `origin/main` 13 commit,已合并(merge commit `9fff00e`)。唯一真冲突是 `runtime/handler.rs` 的 modify/delete——main 把 3712 行的 `handler.rs` 拆成了 `runtime/handler/` 子模块目录,Phase 4.5 的三处改动(StartRun dispatch / AgentRegistryList 列表 / 3 个 handler 测试)重新移植到 `run_start.rs` / `registry_ops.rs` / `handler/mod.rs`;其余 4 个 both-modified 文件自动合并干净。

main 这波对插件架构**净正面**:`emit_for_run` 升级成多 topic fan-out(`run:{id}` + `session:{sid}`),扩展 chat run 带 `session_id` 创建,流式事件经 `emit_for_run` 自动进 session topic,无需扩展侧改动;`dispatch.rs` 改有界 drain,修了大量 outbound 时 keepalive 被饿死的 bug(扩展流式 Token 正是这个负载)。

review「main 对插件架构的影响」时挖出 Phase 4.5 自身的一个洞,已修(commit `e68d968`,见上表「ResumeRun 护栏」行):重启 → `rehydrate` 把 `Running` 扩展 run 转 `Paused` → resume 走 native 路径静默串台。根因比"加护栏"更深——agent 身份此前根本没落盘,见关键设计选择 #5。

### Phase 4.5 遗留(各自独立一刀)

- **懒激活**:`ExtensionRuntime::activate()` 需完整 `SpawnConfig`(bundled Node 路径 / bootstrap.js 路径 / `build_node_flags` / storage 目录);生产侧零 spawn 接线。建议「打包后 Node 放哪」定了再开。
- **`cancel.run` → `$/cancel`**:需给每个 run 存 cancellation handle。
- **`permissionRequest` → 审批子系统**:目前先用 `Trace` 事件透出,未接 review。对应 P4-T08。

### Phase 4.5 验证

- `cargo test -p cronymax --lib` → **451 passed**(merge 后 main 已修原先 pre-existing 的 `crony_def_prompt_is_sealed`;含 ResumeRun 护栏 2 个新测试 `start_run_persists_agent_id_into_run_spec` / `resume_run_for_extension_provider_returns_invalid_state`)
- `cargo test -p cronymax --test agent_runner_test` → **2 passed**
- `cargo clippy -p cronymax --lib --tests` → 0 warnings
- `cargo fmt --check` → clean
- `node --check bundled/extension-host-bootstrap.js` → OK
- web:`npm --prefix web typecheck` + `npm --prefix web test -- --run chat_store` → **28 passed**

### Phase 完成度(Phase 4.5 接入后)

| Phase | 完成 / 总数 | 状态 |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成 |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf / T10 安全冒烟 |
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4 L2 EP × 6 wiring | 7 / 8 | T08 permissionRequest 待做 |
| Phase 4.5 chat-provider 接通 | 主体 ✅ | dispatch 链路打通且有测试;遗留懒激活 / cancel / permission 桥接 |
| Phase 5-10 | 未启动 | 同 |

---

## Phase 8 执行进度(2026-05-23 · agent_provider 接通 chat + flow)

Phase 4.5 把扩展 AgentProvider 接到聊天面板,但只在用户**直接选 provider**(Case A)时生效。两条还没接通的路径:

1. **命名 workspace agent 被扩展背书**(`.cronymax/agents/<name>.agent.yaml` 写 `agent_provider:`)。chat 选这种 agent → 应路由扩展(Case B)。
2. **flow agent 接扩展**。flow 节点 owner 是命名 agent,`AgentRunner::spawn_agent` 之前硬走 `agent_loader → ReactLoop`,不认扩展。

P8 把"哪个引擎跑这个 agent"做成 agent 定义自己的属性,**chat 和 flow 两个入口同样处理** —— 完成 spec invariant("chat 和 flow 共享一套 AgentProvider 表面")。

### 决定性约束(reframes P8)

`cep-idl/v1/agents.ts` **FROZEN**,无 platform→extension 的 tool-result 通道(`AgentSession` 只有 `prompt`/`resolvePermission`/`cancel`,`AgentEvent` 闭合)。意味着扩展 agent **无法调用 cronymax flow 工具**(`submit_document`)。所以 P8 v1 选 **设计 A:turn-level 适配器** —— 扩展 worker 的回合输出**就是**文档,cronymax 经抽出的 `persist_flow_document` 落盘并 push 到 `doc_tx`,下游 supervision 链路无感知。

### YAML schema

```yaml
# 内置 agent —— 没有 agent_provider 键
name: rd
llm: { provider: copilot, model: claude-sonnet-4-6 }
system_prompt: "..."

# 扩展 agent —— scalar 或 map 形态
agent_provider: bytedance.coco.agent                                  # 简单
# 或:
agent_provider: { id: bytedance.coco.agent, model: claude-opus-4-7 }  # 带 per-agent model 默认
```

判别规则:
- 缺省 / `agent_provider: builtin` → native `ReactLoop`
- 有键且解析得到 provider id → 扩展引擎
- 解析后 provider 没装 / 未激活 → **硬错误**(chat: `InvalidState`,flow: `fail_run`),**绝不**静默回退 native(Phase 4.5 ResumeRun 护栏同源教训)

`llm:` 与 `agent_provider:` **严格互斥** —— `llm:` 块对扩展 agent 整块忽略。

### 关键决定

- **`mode` 不入 yaml**:`mode` 是 runtime/聊天面板的概念(IDL `SessionOptions.mode` 是冻结字段,仍存在,P8 只是不从 yaml 取)。v1 不需要 per-agent 默认 mode;扩展用 provider 默认。"plan" 这种是聊天面板运行时选,不烧死在 yaml 里。
- **model 优先级**(与 native 对称):
  - **chat** → 运行时 payload model 赢,yaml `agent_provider.model` 忽略
  - **flow** → yaml `agent_provider.model` 赢(flow 无运行时选择器)

  跟 native 行为完全一致:`grep llm_model` 在 `run_start.rs` 零命中(native chat 不读 `chat_agent_def.llm_model`);native flow `spawn_agent`(`agent_runner.rs:140`)和 `spawn_chat`(`:364`)用 `if llm_model.is_empty() { payload } else { llm_model }` 的 yaml-first。
- **Worker-only**:`kind: reviewer` + `agent_provider` → `fail_run("not supported in v1")`。reviewer-kind 扩展 agent 推后(需要从 turn 输出里解析 verdict,脆)。
- **`effort` 不传给扩展**:`reasoning_effort` / `anthropic_effort` 是 cronymax 自己调 LLM 的参数,扩展 agent 自跑 loop、自调 LLM,IDL `SessionOptions` 无 effort 槽位 —— 设计上不传,不是缺口。

### 改动(6 个文件,+1024/-170)

| 文件 | 改动 |
|---|---|
| `capability/agent_loader.rs` | `AgentProviderRef { id, model }` + `parse_agent_provider`(scalar-or-map);`AgentDef.agent_provider` 字段 |
| `crony/mod.rs` | `CronyBuiltin::def()` 加 `agent_provider: None`(Crony 永远 native) |
| `runtime/ext_dispatch.rs` | 抽出 `run_extension_turn` 共享 core(chat+flow 复用);新增 `drive_extension_flow_agent`(flow 终态:`persist_flow_document` → `doc_tx`);新增 `resolve_agent_provider` helper(chat / flow 共用的解析,无 runtime 时 / 未注册 / 未激活均报错)|
| `capability/submit_document.rs` | 抽出 `persist_flow_document` 核心 —— tool handler 委托,flow 扩展 dispatcher 也调,**下游不可见差别** |
| `runtime/agent_runner.rs` | `spawn_agent` 加 engine fork;`render_system_message_with(SubmitMode)` 变体(扩展 worker 用 `TurnOutput` 告知其回复就是文档,无 `submit_document` 工具)|
| `runtime/handler/run_start.rs` | Case B 接通:`preloaded_chat_agent_def` 早 load,`extension_dispatch` = Case A OR Case B;model 始终用 payload(与 native chat 一致)|

下游 supervision / `on_document_submitted` / 节点激活 **一字未改** —— 扩展 agent 在 `doc_tx` 之后完全隐形。

### 验证

- `cargo test -p cronymax --lib` → **462 passed**(451 baseline + 11 新:5 agent_loader 解析 + 1 `render_system_message_with` TurnOutput + 1 `drive_extension_flow_agent_submits_turn_output_as_document` + 4 `resolve_agent_provider` 分支)
- 集成:`p1_acceptance` 4 + `p2_node_host_e2e` 3(真实 Node 26)+ `p4_extension_runtime_e2e` 2(真实 Node 26)+ `agent_runner_test` 2 → **11 passed**
- `cargo clippy -p cronymax --lib --tests -- -D warnings` → 0
- `cargo fmt --check` → clean

### 显式不做(范围外或推后)

- **reviewer-kind 扩展 agent**:turn-output verdict 解析脆,等真实需要再做
- **P8-T03**:`web/src/panels/flows/agents/` 4 步新建向导(web 工作)
- **P8-T04**:真 `bytedance.coco` 端到端验收(需要 ACP + coco binary)
- **`agent_provider:` + `llm:` 都写时的 validator 警告**:语义已正确(`llm:` 整块忽略),缺写错时的提示
- **model 到达扩展 `session.create` payload 的测试断言**:链路接通,`drive_extension_flow_agent` 测试用 `model: None`,没真断言收到的 payload 含 `model`

### Phase 完成度(Phase 8 接入后)

| Phase | 完成 / 总数 | 状态 |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成 |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf / T10 安全冒烟 |
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4 L2 EP × 6 wiring | 7 / 8 | T08 permissionRequest 待做 |
| Phase 4.5 chat-provider 接通 | 主体 ✅ | 遗留懒激活 / cancel / permission |
| **Phase 8 agent_provider 接通 chat+flow** | **主体 ✅** | **worker-only;reviewer / P8-T03 / P8-T04 推后** |
| Phase 5–7, 9–10 | 未启动 | |

---

## Phase 5 执行进度(2026-05-24 · 平台事件总线接通 ext_dispatch)

P5 把 P3-T01 留下的事件总线骨架接到 chat/flow 流上。`EventBus` 实现 + capability 校验早在 P3 时就写好了(`extensions/events.rs`,7 个单测),P5 做的是 **把它装回 `ExtensionRuntime`、暴露 RPC、给 ext_dispatch 加 emit 站点**,顺手把 SDK runtime 的 `cronymax.events` 名空间补齐。

### 关键决定

- **manifest schema**:`Capabilities` 之前是 `#[serde(flatten)] _ignored`,P5 拆出结构化 `events: EventsCapability { subscribe, emit }`。flatten 不冲突,旧 manifest 写 `events.subscribe`(带点的扁平键)依旧落到 `_ignored`(不再生效)。新 manifest 用 `capabilities.events.{subscribe,emit}: string[]`。
- **`events/emit` 是 request,不是 notify**:IDL `emit(topic, payload): Promise<void>`,只有 request 能拒绝回 Promise。silent drop 比噪声糟糕(capability 配错就藏起来了),所以走 `handle()` 而非 `on_notify()`。`events/subscribe` / `events/unsubscribe` 仍是 notify(对应 `on()` 的同步表面)。
- **per-extension `SubscriptionGuard` 保管在 `ExtensionHandle`**:每个 `events/subscribe` 调用产一个 guard,挂进 `event_subscriptions: HashMap<topic, Vec<Guard>>`。同 topic 多次订阅独立栈,各自 dispose;deactivate 时整张表掉,听器一次性断。注:deactivate 解构 handle 以保留 `host.shutdown().await`,event_subscriptions 在 host 关停前显式 drop。
- **emit 仅在 ext_dispatch**:暂未在 native chat / native ReactLoop 加 emit。`run_extension_turn` 是 chat + flow **唯一**走扩展 provider 的路径,P5 主要落点这里。native 路径接入推后(scope 限定 + 风险:改 ReactLoop 链路面要测)。
- **优化:`emit_from_platform_if_subscribed`**:hot 路径(per-token delta)走这个 helper,无订阅则不构造 JSON payload。topic-lookup short-circuit 不过是一次 HashMap::get,廉价。

### 改动(6 个文件)

| 文件 | 改动 |
|---|---|
| `extensions/manifest.rs` | 加 `EventsCapability { subscribe, emit }` + `Capabilities.events` 字段,保留 `_ignored` flatten |
| `extensions/events.rs` | 加 `emit_from_platform_if_subscribed<F: FnOnce() -> Value>` 优化 helper |
| `extensions/rpc/codec.rs` | 加 `EVENTS_UNSUBSCRIBE` / `EVENTS_EMIT` 方法常量(`EVENTS_PUBLISH` / `EVENTS_SUBSCRIBE` 已在 P3 占位)|
| `extensions/runtime.rs` | `RuntimeState.events: EventBus` + `events()` 公开;`ExtensionHandle.event_subscriptions: HashMap<String, Vec<SubscriptionGuard>>`;activate/deactivate 注册/注销 bus caps;`build_rpc_server` 加 3 handler(events/subscribe / events/unsubscribe / events/emit);加 `build_publish_frame` + `lookup_field` helper |
| `runtime/ext_dispatch.rs` | `run_extension_turn` 加 7 emit 站点:`SessionStarted`(创会后) / `MessageUserSent`(发 prompt 前) / `MessageAssistantDelta`(per `Text` 事件) / `ToolInvoked`(`ToolCall`) / `ToolCompleted`(`ToolCallUpdate`) / `PermissionRequested`(bridge 前 fire,不阻塞 review) / `MessageAssistantDone`(turn 结束) / `SessionEnded`(dispose 后) |
| `bundled/extension-host-bootstrap.js` | 加 `cronymax.events.on/emit` 实现:本地 `eventHandlers` table + 顶层 `events/publish` notify handler 派发;首次 `on(topic)` 发 `events/subscribe`,最后一个 dispose 发 `events/unsubscribe`;`emit` 走 `rpcRequest` 返回 `Promise<void>` |

### 验证

- `cargo test -p cronymax extensions::` → **254 passed**(原 248 + 6 新:`platform_emit_reaches_subscribed_extension_via_publish_notify` / `subscribe_without_capability_does_not_install_listener` / `unsubscribe_severs_the_forwarding_listener` / `extension_emit_request_succeeds_for_declared_topic` / `extension_emit_request_rejects_undeclared_topic` / `extension_emit_request_rejects_cronymax_topic` / `cross_extension_emit_reaches_other_subscribers`)
- `cargo build -p cronymax` → clean
- `node --check bundled/extension-host-bootstrap.js` → OK
- 注:`runtime_e2e::full_run_lifecycle_round_trips_through_persistence` 在 `feat/plugins` head 上**已经**红,与 P5 无关(`git stash; cargo test`确认)。

### 显式不做(留作后续)

- **native ReactLoop 路径 emit**:chat 走 native agent 时不会触发 `cronymax.*` 事件。等到 native 路径也需要给 dogfood 扩展暴露遥测时再做(可能 P7 验收 `logger` 扩展时浮上来)。
- **`cronymax.tool.invoked`/`.completed` 在 native tool 调度处**:同上,目前只在扩展 agent 的 ToolCall/ToolCallUpdate 翻译时 emit。
- **`logger` 测试扩展**:P5-T04 任务卡里的「订阅 `cronymax.message.assistant.done` 落盘」扩展,框架已具备(单测验过 publish 链路),实际扩展工程留给 P7 dogfood 一起做。
- **SDK 类型补 `Capabilities.events`**:`@cronymax/extension` 当前 IDL ts 没有 `EventsCapability` interface 字段。codegen(P9-T01)落地时一并补。

### Phase 完成度(Phase 5 接入后)

| Phase | 完成 / 总数 | 状态 |
|---|---|---|
| Phase 0 基础 + spike | 7 / 7 | ✅ 完成 |
| Phase 1 manifest + registry + activation | 6 / 6 | ✅ 完成 |
| Phase 2 Node host + L1 第一切片 | 11 / 12 | T09 perf / T10 安全冒烟 |
| Phase 3 其余 L1 Kernel | 8 / 9 | T09 验收扩展 待做 |
| Phase 4 L2 EP × 6 wiring | 7 / 8 | T08 permissionRequest 桥 待做 |
| Phase 4.5 chat-provider 接通 | 主体 ✅ | 遗留懒激活 / cancel / permission |
| **Phase 5 L1.5 平台事件** | **主体 ✅** | **8 topic 从 ext_dispatch 接通;native 路径 emit + logger 测试扩展推后** |
| Phase 6 Webview 基建 | 未启动 | |
| Phase 7 coco dogfood | 未启动 | |
| Phase 8 agent_provider 接通 chat+flow | 主体 ✅ | worker-only;reviewer / P8-T03 / P8-T04 推后 |
| Phase 9 SDK + 扩展管理 UI | 部分(统一 ContributionDescriptor) | Settings UI / CLI ext package / 模板仓 待做 |
| Phase 10 收尾 + Alpha | 未启动 | |
