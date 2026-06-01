# Phase 0 评议纪要

- 日期：2026-05-19
- 主持：Claude（AI 协办）· 决议人：DRI（待定，下方 §0.1）
- 配套文档：[`spec-v0.3.md`](spec-v0.3.md) · [`implementation-plan-v0.3.md`](implementation-plan-v0.3.md) · [`tasks-v0.3.md`](tasks-v0.3.md) · [`exec-plan-v0.3.md`](exec-plan-v0.3.md)
- 入会前阅读：[`node26-permission-spike.md`](node26-permission-spike.md) · [`msgpack-rpc-spike.md`](msgpack-rpc-spike.md) · [`legacy-agent-step.md`](legacy-agent-step.md)

---

## 0. 议程

1. 五个 Phase 0 交付物逐项验收
2. spike 发现要 spec-v0.3 改的 5 项确认
3. 七项 DRI 决策签字（plan-v0.3 §10）
4. Phase 1 准入裁决
5. 后续动作

### 0.1 DRI 确认

| 角色 | 人 | 备注 |
|---|---|---|
| 项目 DRI | **待签** | v1 alpha 整体交付负责，含 §3 全部决策的最终权 |
| Rust 主力 | 待定 | Phase 1/2/3/8 主要承担者 |
| TS 主力 | 待定 | Phase 6/7/9 主要承担者 |

---

## 1. 交付物验收

| ID | 交付物 | 状态 | 验证 |
|---|---|---|---|
| P0-T02 | IDL v1 freeze | ✅ | `crates/cronymax/src/extensions/cep-idl/v1/` 13 个 .ts 文件 + tsconfig + README；`npm run check` strict + isolatedModules 全过 |
| P0-T03 | legacy flow brief | ✅ | `docs/extensions/legacy-agent-step.md` 182 行；摸清当前不存在 AgentProvider 抽象、chat/flow 共用 ReactLoop、Phase 8 medium-large 工程量 |
| P0-T04 | Node 26 permission spike | ✅ | `docs/extensions/node26-permission-spike.md`；8 个 .js 测试可重跑（`/tmp/node-perm-spike/`）；5/9 验收点全过，4 项需 spec 修订（见 §2） |
| P0-T05 | MessagePack-RPC spike | ✅ | `docs/extensions/msgpack-rpc-spike.md`；Rust+Node 双端可重跑（`/tmp/msgpack-rpc-spike/`）；P99 18-30μs vs 目标 5ms（250× 冗余） |
| P0-T06 | Rust 模块骨架 | ✅ | `crates/cronymax/src/extensions/` 完整子树；`cargo build -p cronymax` 0 warning；接到 lib.rs |

**结论**：5/5 全部验收。

---

## 2. spec-v0.3 修订项（5 条，全部来自 spike 实测）

每项**默认建议为接受**；DRI 可单独否决。

### 修订 §A · `--allow-net` 在 Node 26.1.0 只是 boolean

**spec 现状**：§6.1 `build_node_flags` 把 `capabilities.network.allow` 翻译成 `--allow-net=<host>` 列表，§6.4 防绕过表说 "fetch 不在白名单 → 拦"。

**实测**：Node 26.1.0 的 `--allow-net` 没有 `=host` 子语法。`--help` 文档形是 `--allow-net`（无 `=...`）；CLI 解析器对 `--allow-net=foo` 不报错只是普遍 `=value` 宽容（accept-and-ignore-value）。Node 27/28 才计划落地 host-scoped。

**建议改法**：
- §6.1 `build_node_flags` 改为：用户 `capabilities.network` 非空 → emit boolean `--allow-net`（无 `=host`）；manifest 字段保留作安装期人话授权用
- §6.4 防绕过表"网络"行改 "v1: boolean，启用即全网开；M1 跟进 Node host-scoped 落地"
- §13 决策 4d 加脚注同义

**影响**：spec 决策 4d（"纯 Node Permission Model"）不变；只是 enforce 力度在 v1 比预期弱，靠 manifest 人话授权 + L3 OS sandbox（γ 阶段）补。Coco 扩展行为零变化。

**待签**：☐ 接受 / ☐ 否决

---

### 修订 §B · `build_node_flags` 永远 emit `--no-warnings`

**spec 现状**：未提及 warning 处理。

**实测**：
- `--allow-net` 启动打 **ExperimentalWarning**
- 6 个 boolean flag（`--allow-child-process / -worker / -addons / -ffi / -inspector / -wasi`）启动**各自**打 **SecurityWarning**："must be used with extreme caution. It could invalidate the permission model."

Coco 这种 `process: true` 扩展每次启动会刷 SecurityWarning 到 stderr，最终用户能看到。

**建议改法**：
- §6.1 `build_node_flags` 永远 emit `--no-warnings`（或 spawn 时设 env `NODE_NO_WARNINGS=1`）
- §6 加一条："扩展开了哪些 high-risk Node flag 走平台审计日志（机器可读），不走 stderr warning"

**影响**：用户看不到吓人 stderr；平台日志多一类记录（low cost）。

**待签**：☐ 接受 / ☐ 否决

---

### 修订 §C · `--allow-net` 是平台基础设施，不是用户 capability

**spec 现状**：§6.1 把 `--allow-net` 当作用户 `capabilities.network` 的产物。

**实测**：Node 26 把 **Unix socket 连接** 也算 network 限制（连接 `/tmp/x.sock` 抛 `connect ERR_ACCESS_DENIED`，code `Use --allow-net to manage permissions.`）。cronymax 的 RPC 通道是 Unix socket，**总要** `--allow-net`，跟扩展是不是声明 `capabilities.network` 无关。

**建议改法**：
- §6.1 `build_node_flags` 永远 emit `--allow-net`（平台必带）
- §6.1 注释明确：用户 `capabilities.network.allow` 是**给安装期人话授权用**的（"此扩展会访问网络：api.openai.com"），不是 Node flag 的输入

**影响**：spec 表达从"按需开放网络"变成"网络永远开放（包括到外网），但安装期 UI 告诉用户它要去哪"。**这是 spike 之前 spec 没暴露的事实**。需要决策者认知到位。

**待签**：☐ 接受 / ☐ 否决

---

### 修订 §D · symlink canonicalize 是 Rust 必做（每路径双填）

**spec 现状**：§6.1 已说 `canonicalize(workspace)` 解 symlink。

**实测**：Node 26 **不**自动解 symlink——`/tmp/x` 和 `/private/tmp/x` 是 ACL 里两条独立路径。给 `--allow-fs-read=/tmp/node-perm-spike` 后读 `/private/tmp/node-perm-spike/x.js` 抛 ERR_ACCESS_DENIED。

**建议改法**：
- §6.1 `build_node_flags` 注释加强：每条 fs 路径 emit **两条** `--allow-fs-read`（原始 symlink form + canonical form），覆盖扩展代码无论传哪种路径都通
- Phase 1 manifest 校验里加 canonicalize 单测

**影响**：实现细节落实化；无 spec 决策变化。

**待签**：☐ 接受 / ☐ 否决

---

### 修订 §E · Phase 2 性能目标 + fs overhead 指标改写

**spec 现状**：tasks-v0.3.md P2-T09 写 "1000 commands.execute round-trip P99 < 5ms"；node26 spike 验收清单写 "permission check overhead < 5%"。

**实测**：
- MessagePack-RPC 实测 P99 18-30μs（250× 冗余于目标）→ 目标可改"command round-trip P99 < 5ms"且大概率随便过
- fs syscall 绝对加 +12μs/call（readFileSync 相对 +98%，statSync 相对 +1300%）；relative 指标看着可怕但绝对值小；"< 5%" 是错指标

**建议改法**：
- tasks-v0.3.md P2-T09 改"端到端 RPC round-trip P99 < 5ms"（保留为 sanity check）
- spike 验收清单 #8 改"绝对 overhead < 50μs per fs call"
- SDK 文档加一段警告 stat-heavy 工作流，推荐 `cronymax.workspace.glob` 平台侧扫描

**影响**：测量指标更准；无性能预期变化。

**待签**：☐ 接受 / ☐ 否决

---

## 3. 七项 DRI 决策

来自 plan-v0.3 §10。✅ 表示 spec 已写明 / 已实测确认；⏳ 表示评议会现场决策。

| # | 决策 | 默认 | 状态 |
|---|---|---|---|
| 1 | Node 版本 | Node 26 | ✅ 已签（spike 验证通过） |
| 2 | MessagePack 实现 | `@msgpack/msgpack` (JS) + `rmp-serde` (Rust) | ✅ 已签（spike 实测互通零调整） |
| 3 | CLI 位置 | 扩在现有 cronymax CLI 上（不另起进程） | ✅ 已签 |
| 4 | SDK npm 发布渠道 | — | **⏳ 待签** |
| 5 | 测试 coco binary 部署 | — | **⏳ 待签** |
| 6 | Phase 7 末发内部 alpha | — | **⏳ 待签** |
| 7 | Node 26 ship 时 LTS 状态 | — | **⏳ 信息性确认** |

下方 §3.4 / §3.5 / §3.6 / §3.7 分别展开。

### 3.4 SDK npm 发布渠道

候选：
- **A. 公司内网 npm**（ByteDance 内部 registry；权限管控好；离线无法装）
- **B. GitHub Packages**（公司 GitHub Enterprise 上托管；用 npm scope 路由）
- **C. 起步阶段从仓库 git 装**（M0 不发，团队从 `git+https://...` 装；M1 再迁内网 npm）

推荐：v1 alpha 阶段 **C**（最低运维成本，团队内部都能 git clone）；M1 用户面扩展开发者前迁 **A**。

### 3.5 测试 coco binary 部署

候选：
- **A. 用户单独装、扩展通过 `PATH` 找**（manifest 申报 `coco.binaryPath` 默认 `"coco"`，用户必须先在系统装好）
- **B. cronymax 内置打包 coco binary**（每平台 + ~50MB 总体积）
- **C. 扩展首次 activate 时按需下载**（manifest 声明下载源，扩展 storage 落地）

推荐：**A**——保持扩展通用性硬规则（spec §1 "扩展不绑特定运行时"），且 coco binary 在内部团队普及度高，"必须先装 coco" 不是高门槛。**B** 违反通用性；**C** 复杂度过高且与 v1 不发 marketplace 的策略矛盾。

### 3.6 Phase 7 末是否发内部 alpha

候选：
- **A. 发**：早期反馈，但 API 未稳；不通过文档承诺"v1 兼容"
- **B. 不发**：等 Phase 10 完工发 alpha，避免 v1 兼容期被反悔

推荐：**A**——价值远超风险。明确写"internal preview，API 可能变"；只发给同事；不对外宣传。

### 3.7 Node 26 ship 时 LTS 状态

事实陈述（无需选择）：
- Node 26 是 2025-10 进 current 通道
- Node 26 进 LTS Active：**2027-10**
- cronymax v1 alpha ship 目标：**2026-08** 左右（10-12 周后）
- ship 时 Node 26 仍是 current

**含义**：v1 alpha 用 current；M1 重新评估（届时 Node 26 是 Active LTS 或仍是 current 取决于实际节奏）。本项不需要决策，仅告知。

---

## 4. Phase 1 准入裁决

plan-v0.3 §0 / tasks-v0.3 末尾的硬指标对照：

| 准入指标 | 状态 |
|---|---|
| spec-v0.3 + plan-v0.3 + tasks-v0.3 评审定稿 | ⏳ 本会决议 |
| IDL v1 freeze + ≥2 +1 review | ☐ 文件已 freeze；**还差 ≥2 人 +1 code review**（建议本会 follow-up） |
| Node 26 spike 报告全 ✅ | ✅ |
| MessagePack-RPC P99 < 5ms | ✅（250× 冗余） |
| flow legacy brief 完成 | ✅ |
| DRI 7 项决策签字 | ⏳ 本会 §3 决议 |
| 团队人员对齐 | ⏳ 本会 §0.1 决议 |

**裁决候选**：
- **PASS（进 Phase 1）**：spec 修订 5 项全采纳 + DRI 7 项全签 + 团队对齐
- **PASS-conditional**：spec 修订 + DRI 决策定，但等 IDL code review +2 才正式开 Phase 1（推荐）
- **HOLD**：有 spec 修订项否决；需补做 spike 或重新设计

---

## 5. 后续动作（评议会落地清单）

| ID | 动作 | DRI | 截止 |
|---|---|---|---|
| FU-1 | 把本纪要 §2 接受的 spec 修订项 patch 进 spec-v0.3.md | Rust 主力 | 评议后 3 天内 |
| FU-2 | IDL v1 找 2 人 +1 review | 全员 | 评议后 1 周内 |
| FU-3 | 在 tasks-v0.3.md P1 任务卡里把"卡 `--no-warnings` 必带"等修订项落到具体任务描述 | 同 FU-1 | 同 FU-1 |
| FU-4 | 若 §3.6 选 A：起草"internal preview" 用户口径 | TS 主力 | Phase 7 启动前 |
| FU-5 | 启动 Phase 1（按 tasks-v0.3 P1-T01..T06） | Rust 主力 | FU-1 + FU-2 完成后立即 |

---

## 6. 决议落档（进行中）

| 项 | 决议 | 备注 |
|---|---|---|
| §0.1 DRI | ✅ 单人 | 评议 2026-05-20；DRI = Rust 主力 = TS 主力 = 用户；估时按 1 人拉长到 14-16 周；IDL review 政策 = AI 协助 self-review + 决策理由文档化 |
| §2-A `--allow-net` 只是 boolean，spec 改 | ✅ 接受 | 评议 2026-05-19 |
| §2-B `--no-warnings` 永远开 + audit log | ✅ 接受 | 评议 2026-05-19；与 [`extension-logs.md`](extension-logs.md) §7+§8 一致 |
| §2-C **重决**：RPC 改 inherited fd 3，`--allow-net` 还给用户 | ✅ 接受 | 评议 2026-05-19；spike 实测 fd 3 wrap 不需 --allow-net。spec §6.1 IPC 改写法；+1-1.5d 工程量；弹窗"网络"项变诚实 |
| §2-D **重设**：manifest fs 改平台变量方案（`{WORKSPACE}` / `{HOME}` / `{EXT_STORAGE}` 等）| ✅ 接受 | 评议 2026-05-19；manifest fs 变数组 `[{path, mode}]`；变量名白名单封闭；平台展开 + canonicalize 一次；扩展不接触绝对路径。+1.5d。`cep-idl/v1/manifest.ts` 作 freeze 单次例外（同 logging.ts） |
| §2-E Phase 2 性能目标 + fs overhead 指标改字 | ✅ 接受 | 评议 2026-05-19；P2-T09 改"command 端到端 RPC P99 < 5ms"；spike checklist #8 改"绝对 overhead < 50μs/call"；**不**加 workspace.glob（avoid speculative API） |
| §3.4 SDK 渠道 | ✅ 公网 npm | 评议 2026-05-20；包名建议 `@cronymax/extension`；只发 bundled dist + .d.ts，源码可不公开；启动前过 ByteDance 开源/法务流程；v1 alpha 走 `0.x` 版本 |
| §3.5 coco 部署 | ✅ A：PATH 找 | 评议 2026-05-20；manifest `coco.binaryPath` 默认 `"coco"`；设置面板可改路径；保留扩展通用性硬规则 |
| §3.6 Phase 7 alpha | ✅ B：不发 | 评议 2026-05-20；Phase 10 完工后一次发；节奏更稳，避免 v1 兼容压力 |
| §3.7 LTS 状态 | ✅ 信息性确认 | Node 26 进 Active LTS 2027-10；v1 alpha ship 时是 current；M1 重评估 |
| **NEW** 扩展日志系统纳入 v1 | ✅ 接受 | 评议 2026-05-19；详 [`extension-logs.md`](extension-logs.md) |
| **NEW** `createOutputChannel` SDK 进 v1 | ✅ 接受 | 同上 |
| **NEW** `cep-idl/v1/logging.ts` 作 freeze 例外 | ✅ 接受 | 同上；2 人 review |
| **NEW** bootstrap.js console.* + process.on 不违反 §6.1 | ✅ 接受 | spec §6.1 加澄清注释 |
| §4 Phase 1 准入裁决 | ✅ **PASS** | 评议 2026-05-20；Phase 1 立刻启动；spec patches + IDL 增量并行进行 |

---

## 7. 评议会闭幕清单

### 全部决议汇总

✅ §0.1 DRI = 单人（用户）
✅ §2-A `--allow-net` 仅 boolean → manifest network.allow 退化为安装期人话授权
✅ §2-B 永远 `--no-warnings` + audit log 结构化记录
✅ §2-C 重决：RPC 改 inherited fd 3 → `--allow-net` 还给用户作为 capability
✅ §2-D 重设：manifest fs 改平台变量方案 `[{path, mode}]` + 变量白名单
✅ §2-E 性能指标改字（**不**加 workspace.glob）
✅ §3.4 SDK 走公网 npm（`@cronymax/extension` scoped）
✅ §3.5 coco binary 走 PATH 找（manifest 默认 `"coco"`）
✅ §3.6 Phase 10 完工后一次发 alpha（**不**发 Phase 7 末预览）
✅ §3.7 LTS 状态信息性确认
✅ §4 PASS — Phase 1 立刻启动

✅ 扩展日志系统纳入 v1（`extension-logs.md` v0.2）
✅ `createOutputChannel` SDK 进 v1
✅ `cep-idl/v1/logging.ts` + `manifest.ts` 改 作 v1 freeze 单次例外
✅ bootstrap.js console.* + process.on hook 不违反 spec §6.1

### Phase 1 启动前 follow-up（可与 Phase 1 并行做）

| ID | 动作 | 受影响文件 | 估时 |
|---|---|---|---|
| FU-1 | spec-v0.3.md patch §6.1 / §6.4 / §8 / §13：网络 ACL boolean + always `--no-warnings` + RPC fd 3 + manifest fs 变量方案 | spec-v0.3.md | 1d |
| FU-2 | IDL 增量：`cep-idl/v1/logging.ts` 新增；`window.ts` 加 `createOutputChannel`；`manifest.ts` fs 字段改数组 + 变量；`workspace.ts` 不动 | crates/cronymax/src/extensions/cep-idl/v1/* | 0.5d |
| FU-3 | Rust manifest.rs 同步 IDL 改 | crates/cronymax/src/extensions/manifest.rs | 0.5d |
| FU-4 | msgpack-rpc spike 重跑：RPC 走 fd 3 而非 Unix socket，验证 P99 仍 < 5ms | /tmp/msgpack-rpc-spike/ + spike doc | 0.5d |
| FU-5 | tasks-v0.3.md 更新：P2-T09 指标 / 加扩展日志系统相关任务（~11 天） / 其他评议 patch | docs/extensions/tasks-v0.3.md | 0.5d |
| FU-6 | extension-logs.md v0.2 已就位 | — | done |
| FU-7 | IDL self-review：AI 把 13 个 .ts 跟 spec 对一遍，找未覆盖点，输出 review 报告 | docs/extensions/idl-self-review.md | 0.5d |
| FU-8 | ByteDance 公网 npm 发包流程预问（法务/开源办） | — | 用户行动 |

总 follow-up 工程量：约 3-4 天（AI 主担），可与 Phase 1 启动并行。

签字：DRI (单人) · 日期：2026-05-20
