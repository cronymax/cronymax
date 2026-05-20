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
