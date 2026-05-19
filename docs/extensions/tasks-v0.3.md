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
