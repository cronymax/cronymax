# Cronymax 扩展平台 · AI 自执行任务清单 v0.3

> **本文档由 AI 写给未来的自己**。
> 目的：让未来某次 session 拿到这个文件就能照着干，不依赖之前对话上下文。
> 配套：[`spec-v0.3.md`](spec-v0.3.md) 是设计真相之源，本文是执行步骤。
> 任何模糊不清的地方，**先停下来读 spec，不要猜**。

---

## 0. 上下文锁定（每次开工前必读这一节）

### 我是谁、要做什么

我正在 cronymax 仓库（`/Users/bytedance/Workspace/cronymax`）实现**扩展平台 v1 alpha**。

最终验收 = `bytedance.coco` 扩展能装、能选、能聊、能在 flow 里跑；`acme.mermaid-renderer` 扩展能渲染 agent 输出的 mermaid 块。

### 平台架构（30 秒掌握）

- **Rust core**（已有）：cronymax 主进程
- **CEF Renderer**（已有）：cronymax UI + 扩展 webview iframe
- **Node 26 host**（新增）：每扩展独立的 Node subprocess，跑扩展 main.js
- **三者通过 Unix socket + MessagePack-RPC 通信**

### 唯一不能违反的 5 条规则

每写一段代码 / 接收每个 PR 前自问：

1. `crates/cronymax/src/extensions/` 里**不能出现** "coco" / "mermaid" / "slack" 字眼（具体扩展名只在 dogfood 扩展代码里）
2. **所有 L2 EP 走同一套注册机制**（通用 `contributions/` 中心，不要为每个 EP 写专用 handler 模块）
3. **任何 L1 API 必经 Node Permission**（不写内置扩展豁免快速路径）
4. **AgentProvider 接口对 chat 面板和 flow runtime 完全对称**
5. **加内置扩展前先尝试用纯 SDK 写**，发现要改核心就停下加 EP 或事件主题

### Spec 的关键决策一句话总结

- **Runtime**：Node 26 subprocess，每扩展独立 host，lazy activate，不主动 deactivate
- **安全 α**：纯 Node Permission Model（`--permission --allow-fs-read/-write/-net` etc.），**不写**任何 require 劫持、命令白名单、网络包装、审计 hook
- **manifest capabilities** 极简：`fs: { scope, mode }` / `network: { allow: [...] }` / `process: true` / `workers: false` etc.
- **IPC**：Unix socket（Win Named Pipe）+ MessagePack-RPC，`@msgpack/msgpack` + `rmp-serde`
- **跨扩展通信**：VS Code 同款 `extension.exports`（无 schema/semver）
- **AgentProvider** 而非 ChatProvider（不绑 chat 表面）
- **CJS 主推**（v1），ESM 推 M1

### 文件位置约定

```
crates/cronymax/src/extensions/        ← 扩展平台 Rust 实现（新增）
├── mod.rs                              ← 模块入口
├── manifest.rs                         ← manifest schema + 校验
├── registry.rs                         ← 扩展元数据 + 启用状态
├── activation.rs                       ← activationEvents 匹配
├── capability.rs                       ← manifest → Node flags 转换
├── host/                               ← Node host 进程管理
│   ├── mod.rs
│   └── node.rs                         ← spawn / 监控 / 重启
├── rpc/                                ← MessagePack-RPC
│   ├── mod.rs
│   ├── codec.rs                        ← 编解码
│   └── server.rs                       ← 服务端 dispatch
├── contributions/                      ← L2 EP 注册中心（通用）
│   └── mod.rs
├── events.rs                           ← L1.5 平台事件总线
├── api/                                ← cronymax/v1 API Rust 实现
│   ├── mod.rs
│   ├── lifecycle.rs
│   ├── commands.rs
│   ├── events.rs
│   ├── config.rs
│   ├── secrets.rs
│   ├── storage.rs
│   ├── webview.rs
│   ├── auth.rs
│   ├── extensions.rs
│   ├── agents.rs                       ← AgentProvider 注册
│   ├── window.rs
│   └── workspace.rs
└── cep-idl/v1/                         ← IDL TS interface（真相之源）
    └── *.ts

bundled/                                ← 打包资源
├── node/                               ← Node 26 二进制（各平台）
│   ├── darwin-arm64/node
│   ├── darwin-x64/node
│   ├── linux-x64/node
│   └── win32-x64/node.exe
└── extension-host-bootstrap.js         ← Node 启动后第一个跑的脚本

web/src/
├── extensions/sdk/                     ← @cronymax/extension npm 包源码
│   └── ...
├── panels/
│   ├── chat/                           ← 修改：用 registry 列 provider
│   ├── flows/agents/                   ← 新增：agent 列表 + 向导
│   └── settings/extensions/            ← 新增：扩展管理 UI
└── shells/
    └── extension.ts                    ← 新增：渲染侧 SDK 桥

~/.cronymax/extensions/                 ← 运行时扩展安装位置
├── registry.json                       ← 启用状态
└── <publisher>.<name>/
    ├── cronymax-extension.json
    └── dist/main.js + ...
```

### 命名约定

- Rust：snake_case 文件、PascalCase 类型、snake_case 函数
- TS：camelCase 函数/变量、PascalCase 类型、kebab-case 文件名（webview 资源）
- 错误类型：`ExtensionError` enum in `crates/cronymax/src/extensions/error.rs`
- 日志：`tracing` crate，target = `"cronymax::extensions"`

---

## P0 · Foundation（1 周）

### P0-T01 · 评审会决策落档

**Goal**：把对话里的所有决策固化到文档，避免后续来回。

**Action**：
1. 读 `docs/extensions/spec-v0.3.md` 全部
2. 读 `docs/extensions/implementation-plan-v0.3.md` 全部
3. 读 `docs/extensions/tasks-v0.3.md` 全部
4. 确认与本文档一致，不一致则以 spec-v0.3 为准

**Done when**：三份文档语义一致；任何冲突在 spec 修正后同步 plan + tasks + exec-plan。

---

### P0-T02 · IDL v1 freeze

**Goal**：把扩展 ↔ 平台契约用 TypeScript interface 写死，从此**只增不删**（破坏性变更要发 v2）。

**Files to create**：
```
crates/cronymax/src/extensions/cep-idl/v1/
├── index.ts                ← 统一导出
├── primitives.ts           ← URI / Disposable / CancellationToken / Event
├── lifecycle.ts            ← ExtensionContext / activate / deactivate
├── commands.ts             ← register / execute
├── events.ts               ← on / emit / topics 列表
├── workspace.ts            ← rootUri / fs / config
├── window.ts               ← messages / inputBox / quickPick / webviewPanel
├── secrets.ts
├── auth.ts                 ← getSession / AuthSession
├── extensions.ts           ← getExtension / Extension
├── agents.ts               ← AgentProvider / AgentSession / AgentEvent / SessionOptions
├── renderers.ts            ← content renderer registration
└── manifest.ts             ← Manifest / Capabilities schema 类型
```

**Key interfaces 不能改的部分**（违反等于破坏 v1 兼容）：

```ts
// agents.ts
export interface AgentProvider {
  listModels(): Promise<ModelInfo[]>;
  modes?: ModeInfo[];
  createSession(opts: SessionOptions): Promise<AgentSession>;
}

export interface AgentSession {
  readonly id: string;
  prompt(message: PromptMessage, token: CancellationToken): AsyncIterable<AgentEvent>;
  cancel(): Promise<void>;
  dispose(): Promise<void>;
}

export type AgentEvent =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "toolCall"; id: string; name: string; input: unknown; source: string; status: "in_progress" }
  | { kind: "toolCallUpdate"; id: string; status: "completed" | "failed"; output: unknown }
  | { kind: "permissionRequest"; requestId: string; tool: string; options: unknown }
  | { kind: "done"; stopReason: "end_turn" | "max_tokens" | "tool_calls" | "cancelled" | "error" };
```

**Verify**：`tsc --noEmit crates/cronymax/src/extensions/cep-idl/v1/index.ts` 通过。

**Done when**：所有 .ts 文件 commit；spec-v0.3 §2/§4 列出的所有 API 都有 TS 类型对应。

---

### P0-T03 · 摸清现有 flow runtime

**Goal**：搞清楚 Phase 8 改造范围；写 brief。

**Action**：
1. 读 `crates/cronymax/src/agent_loop/` 全部
2. 读 `crates/cronymax/src/capability/agent_loader.rs` 全部
3. 读 `crates/cronymax/src/flow/` 全部
4. 找出现在 agent step 怎么调度的（哪个文件、哪些函数、哪些数据流）
5. 写到 `docs/extensions/legacy-agent-step.md`，结构：
   - 现状：当前怎么工作（30 行内）
   - 改造点：Phase 8 要改哪几个函数 / 文件
   - 兼容性：老 yaml 怎么处理（默认 `provider: native`）

**Verify**：brief 落地，且能在 30 秒讲清楚 Phase 8 要动什么。

---

### P0-T04 · Node 26 真机 spike

**Goal**：验证 spec 假设的 Node 26 行为全部成立。

**Action**：
1. 准备 Node 26 二进制（如本地有 node 26 用之，否则用 24/22 + 备注差异）
2. 写 spike 脚本到 `/tmp/node-perm-spike-final/`
3. 跑以下测试，结果写到 `docs/extensions/node26-permission-spike.md`：

| 测试 | 期望 |
|---|---|
| `--permission` 启动 hello world | 正常打印 |
| `--allow-fs-read=$PWD` 读 `./test.txt` | OK |
| 同上读 `/etc/passwd` | `ERR_ACCESS_DENIED` |
| 路径含 symlink（macOS `/tmp` → `/private/tmp`） | 需 canonicalize 后才允许 |
| `--allow-net=api.example.com` | 该 host 可 fetch；其他拒 |
| `--allow-net=*.example.com` | 通配测试（可能不支持，需验证 syntax）|
| 不带 `--allow-child-process` 时 `spawn` | 拒 |
| 带时 spawn any command | 允许（boolean） |
| 默认禁 `--allow-addons`：尝试 `require('addon')` | 拒（addon 文件用任意 .node） |
| 默认禁 `--allow-ffi`：尝试 `node:ffi` | 拒 |
| 默认禁 `--allow-inspector`：尝试启 inspector | 拒 |
| 绕路 `eval("require('fs').readFileSync(...)")` | 拒 |
| 绕路 `process.binding('fs').open(...)` | 拒 |
| 绕路 `Function('return require')()` | require 不可达 |
| 绕路动态 `import('fs')` | 拒 |
| npm 包：`yaml` / `zod` / `date-fns` / `lodash` 在 permission 下正常工作 | ✅ |
| 性能：`readFileSync` 1000 次 overhead | < 50% |

**Done when**：报告 ≥ 90% 项目 ✅；如果 `--allow-net` 在 Node 26 还没正式 release，回 v0.3 改决策 4 为"等 Node ship 26 LTS 或用 24 + 接受无 --allow-net"。

---

### P0-T05 · MessagePack-RPC spike

**Goal**：验证 Rust ↔ Node IPC 性能可接受。

**Files**：
- `/tmp/msgpack-spike/rust-server/Cargo.toml` 用 `rmp-serde` + `tokio` + Unix socket
- `/tmp/msgpack-spike/node-client/package.json` 用 `@msgpack/msgpack` + node:net

**Test**：1000 次 round-trip ping-pong（每次 ~100 字节 payload）。

**Expect**：P99 < 5ms / P50 < 1ms。

**Done when**：`docs/extensions/msgpack-rpc-spike.md` 落地报告。

---

### P0-T06 · 模块骨架

**Goal**：把空模块加进 Rust crate 编译，不影响现有代码。

**Action**：
1. 编辑 `crates/cronymax/src/lib.rs`：加 `pub mod extensions;`
2. 创建 `crates/cronymax/src/extensions/mod.rs`：
   ```rust
   pub mod manifest;
   pub mod registry;
   pub mod activation;
   pub mod capability;
   pub mod events;
   pub mod host {
       pub mod node;
   }
   pub mod rpc {
       pub mod codec;
       pub mod server;
   }
   pub mod contributions;
   pub mod api;
   pub mod error;
   ```
3. 每个子模块写 `// TODO: P{n}-T{m}` 占位 + 空 pub use
4. 创建 `crates/cronymax/src/extensions/error.rs`：
   ```rust
   use thiserror::Error;
   
   #[derive(Debug, Error)]
   pub enum ExtensionError {
       #[error("manifest invalid: {0}")]
       ManifestInvalid(String),
       #[error("namespace reserved: {0}")]
       NamespaceReserved(String),
       #[error("capability denied: {0}")]
       CapabilityDenied(String),
       #[error("extension not found: {0}")]
       NotFound(String),
       #[error("host crashed too many times")]
       HostCrashLoop,
       #[error("rpc error: {0}")]
       Rpc(String),
       #[error("io error: {0}")]
       Io(#[from] std::io::Error),
   }
   ```

**Verify**：`cargo build -p cronymax` 通过。

---

### P0-T07 · Phase 0 验收 + 闸门

**Goal**：进 Phase 1 前所有先决条件都满足。

**Checklist**：
- [ ] P0-T02 IDL freeze + ≥2 +1
- [ ] P0-T03 brief 落地
- [ ] P0-T04 Node 26 spike 全 ✅
- [ ] P0-T05 RPC spike P99 < 5ms
- [ ] P0-T06 cargo build 通
- [ ] 7 项 DRI 决策签字（tasks-v0.3.md §10）

**Done when**：上面全 ✅。

---

## P1 · Manifest + Registry + Activation（2 周）

### P1-T01 · `manifest.rs` schema 定义

**Files**：`crates/cronymax/src/extensions/manifest.rs`

**Implement**：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub engines: Engines,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, rename = "activationEvents")]
    pub activation_events: Vec<String>,
    #[serde(default)]
    pub contributes: serde_json::Map<String, serde_json::Value>,
    pub capabilities: Capabilities,
    #[serde(default, rename = "extensionDependencies")]
    pub extension_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Engines {
    pub cronymax: String,  // semver range
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    #[serde(default)]
    pub fs: Option<FsCap>,
    #[serde(default)]
    pub network: Option<NetworkCap>,
    #[serde(default)]
    pub process: Option<bool>,
    #[serde(default)]
    pub workers: Option<bool>,
    #[serde(default)]
    pub native_addons: Option<bool>,
    #[serde(default)]
    pub secrets: Option<SecretsCap>,
    #[serde(default, rename = "events.subscribe")]
    pub events_subscribe: Vec<String>,
    #[serde(default, rename = "events.emit")]
    pub events_emit: Vec<String>,
    #[serde(default, rename = "ui-slots")]
    pub ui_slots: Vec<String>,
    #[serde(default, rename = "extension-points")]
    pub extension_points: Vec<String>,
    #[serde(default, rename = "auth.providers")]
    pub auth_providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsCap {
    pub scope: String,  // "workspace" | "none"
    pub mode: String,   // "ro" | "rw"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkCap {
    pub allow: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsCap {
    pub namespace: String,
}

impl Manifest {
    pub fn from_path(path: &std::path::Path) -> Result<Self, crate::extensions::error::ExtensionError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Manifest = serde_json::from_str(&content)
            .map_err(|e| crate::extensions::error::ExtensionError::ManifestInvalid(e.to_string()))?;
        Ok(manifest)
    }
}
```

**Done when**：`cargo test -p cronymax extensions::manifest` 通过；能解 Coco manifest 示例。

---

### P1-T02 · manifest 校验

**Files**：`crates/cronymax/src/extensions/manifest.rs`（追加）

**Implement**：

```rust
impl Manifest {
    pub fn validate(&self) -> Result<(), ExtensionError> {
        // 1. id 格式 publisher.name
        if !self.id.starts_with(&format!("{}.", self.publisher)) {
            return Err(ExtensionError::ManifestInvalid(
                format!("id '{}' must start with publisher '{}.'", self.id, self.publisher)));
        }
        if self.publisher == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(
                "publisher 'cronymax' is reserved".into()));
        }
        
        // 2. contributes 的 key 必须以 cronymax. 开头且在 extension-points 申报
        for key in self.contributes.keys() {
            if !key.starts_with("cronymax.") {
                return Err(ExtensionError::ManifestInvalid(
                    format!("contribution '{}' must start with 'cronymax.'", key)));
            }
            if !self.capabilities.extension_points.contains(key) {
                return Err(ExtensionError::ManifestInvalid(
                    format!("contributing to '{}' requires capabilities.extension-points entry", key)));
            }
        }
        
        // 3. events.emit / events.subscribe 不能含 cronymax.* 写入
        for topic in &self.capabilities.events_emit {
            if topic.starts_with("cronymax.") {
                return Err(ExtensionError::NamespaceReserved(
                    format!("cannot emit to cronymax.* topic: {}", topic)));
            }
        }
        
        // 4. secrets.namespace 必须是 publisher.* 或子集
        if let Some(sec) = &self.capabilities.secrets {
            let expected_prefix = format!("{}.", self.publisher);
            if !sec.namespace.starts_with(&expected_prefix) {
                return Err(ExtensionError::NamespaceReserved(
                    format!("secrets.namespace '{}' must start with publisher prefix '{}'",
                            sec.namespace, expected_prefix)));
            }
        }
        
        // 5. activationEvents = ["*"] 警告（log，不拒）
        if self.activation_events.contains(&"*".to_string()) {
            tracing::warn!(target: "cronymax::extensions", "extension {} uses '*' activation, will eagerly start", self.id);
        }
        
        Ok(())
    }
}
```

**Tests**：
- ✅ 合法 manifest 通过
- ✅ id 不匹配 publisher → reject
- ✅ publisher = "cronymax" → reject
- ✅ contributes 含 "acme.foo" key → reject
- ✅ events.emit 含 "cronymax.message.x" → reject
- ✅ secrets.namespace 不是 publisher 前缀 → reject

**Done when**：所有 tests 通。

---

### P1-T03 · `registry.rs` 扩展元数据 + 启用状态

**Files**：`crates/cronymax/src/extensions/registry.rs`

**Implement**：

```rust
pub struct ExtensionRegistry {
    base_dir: PathBuf,                          // ~/.cronymax/extensions/
    entries: HashMap<String, RegistryEntry>,    // id → entry
}

pub struct RegistryEntry {
    pub manifest: Manifest,
    pub enabled: bool,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub granted_capabilities: Vec<String>,
    pub install_dir: PathBuf,
}

impl ExtensionRegistry {
    pub fn open(base_dir: PathBuf) -> Result<Self, ExtensionError> { ... }
    pub fn scan(&mut self) -> Result<(), ExtensionError> { ... }
    pub fn install(&mut self, crx_path: &Path, granted: Vec<String>) -> Result<&RegistryEntry, ExtensionError> { ... }
    pub fn uninstall(&mut self, id: &str) -> Result<(), ExtensionError> { ... }
    pub fn enable(&mut self, id: &str) -> Result<(), ExtensionError> { ... }
    pub fn disable(&mut self, id: &str) -> Result<(), ExtensionError> { ... }
    pub fn get(&self, id: &str) -> Option<&RegistryEntry> { ... }
    pub fn list(&self) -> Vec<&RegistryEntry> { ... }
    fn persist(&self) -> Result<(), ExtensionError> { ... }
}
```

**Persistence file**：`~/.cronymax/extensions/registry.json`

```json
{
  "version": 1,
  "extensions": {
    "bytedance.coco": {
      "enabled": true,
      "installedAt": "2026-05-19T...",
      "version": "0.1.0",
      "grantedCapabilities": ["fs", "network", "process", "secrets"]
    }
  }
}
```

**Done when**：
- 扫盘能识别 valid manifest 的扩展
- install / uninstall 写盘 round-trip 正确
- 状态变更立即 persist

---

### P1-T04 · `activation.rs` 激活事件匹配

**Files**：`crates/cronymax/src/extensions/activation.rs`

**Implement**：

```rust
pub struct ActivationIndex {
    // event_name → extension_ids
    by_event: HashMap<String, HashSet<String>>,
}

impl ActivationIndex {
    pub fn build(registry: &ExtensionRegistry) -> Self { ... }
    pub fn match_event(&self, event: &str) -> Vec<String> { ... }
}

pub fn parse_activation_event(s: &str) -> ActivationEvent {
    // "onCommand:foo.bar" → ActivationEvent::OnCommand("foo.bar")
    // "onAgentProvider:coco" → ActivationEvent::OnAgentProvider("coco")
    // "onStartup" → ActivationEvent::OnStartup
    // "*" → ActivationEvent::Wildcard
}

pub enum ActivationEvent {
    OnCommand(String),
    OnAgentProvider(String),
    OnView(String),
    OnStartup,
    OnLanguage(String),
    Wildcard,
    WorkspaceContains(String),
}
```

**Done when**：unit tests 覆盖 5 种激活事件类型 + 通配符匹配。

---

### P1-T05 · CLI `cronymax ext` 命令组

**Files**：`crates/cronymax/src/cli/ext.rs`（新增）+ `crates/cronymax/src/cli/mod.rs`（修改注册）

**Subcommands**：
- `cronymax ext install <crx-or-dir>` — 解压（如 .crx）、校验 manifest、弹授权（CLI 模式 print），写 registry
- `cronymax ext list` — 列已装扩展（id, version, enabled, capabilities summary）
- `cronymax ext enable <id>` / `cronymax ext disable <id>`
- `cronymax ext uninstall <id>` — 移除文件 + 清状态
- `cronymax ext info <id>` — 详细信息

**Done when**：装一个 `tests/fixtures/extensions/dummy-noop` 极简扩展，list/enable/disable/uninstall 全过。

---

### P1-T06 · Phase 1 验收

**Action**：
1. 写 `tests/fixtures/extensions/dummy-noop/`（manifest only，无 main.js）
2. `cronymax ext install tests/fixtures/extensions/dummy-noop` → 成功
3. `cronymax ext list` → 含 dummy-noop
4. `cronymax ext disable dummy.noop` → enabled=false
5. `cronymax ext uninstall dummy.noop` → 残留全清
6. 故意改 manifest（publisher=cronymax）→ install 拒
7. 故意改 manifest（contributes 无对应 capability 申报）→ install 拒

**Done when**：上面全 ✅。

---

## P2 · Node Host + L1 第一切片（2 周）

### P2-T01 · Node 26 多平台打包

**Files**：
- `bundled/node/darwin-arm64/node`
- `bundled/node/darwin-x64/node`
- `bundled/node/linux-x64/node`
- `bundled/node/win32-x64/node.exe`
- `build/CMakeLists.txt`（或类似）—— 在 build 时拷贝对应平台 binary 到 app bundle

**Action**：
1. 下载 Node 26 各平台 release tarball
2. 提取 `bin/node` 到上述位置
3. CMakeLists 加 install 规则
4. 写一个 `crates/cronymax/src/extensions/host/node_bin.rs`：根据 `cfg!(target_os)` + `cfg!(target_arch)` 返回正确路径

```rust
pub fn node_binary_path() -> PathBuf {
    let app_resources = crate::workspace::app_resources_dir();
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64")  => "darwin-x64",
        ("linux", "x86_64")  => "linux-x64",
        ("windows", "x86_64")=> "win32-x64",
        _ => panic!("unsupported platform"),
    };
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    app_resources.join("bundled/node").join(platform).join(name)
}
```

**Done when**：在 4 平台 cronymax build 都包含可执行 node 26。

---

### P2-T02 · `capability.rs` build_node_flags

**Files**：`crates/cronymax/src/extensions/capability.rs`

**Implement**：

```rust
use std::path::{Path, PathBuf};
use crate::extensions::manifest::Manifest;

pub fn build_node_flags(
    manifest: &Manifest,
    workspace: &Path,
    ext_data_dir: &Path,
) -> Result<Vec<String>, ExtensionError> {
    let mut flags = vec!["--permission".to_string()];
    
    // fs
    if let Some(fs_cap) = &manifest.capabilities.fs {
        if fs_cap.scope == "workspace" {
            let ws = canonicalize_path(workspace)?;
            flags.push(format!("--allow-fs-read={}", ws.display()));
            if fs_cap.mode == "rw" {
                flags.push(format!("--allow-fs-write={}", ws.display()));
            }
        }
        let ed = canonicalize_path(ext_data_dir)?;
        flags.push(format!("--allow-fs-read={}", ed.display()));
        flags.push(format!("--allow-fs-write={}", ed.display()));
    }
    
    // network (Node 26)
    if let Some(net_cap) = &manifest.capabilities.network {
        for host in &net_cap.allow {
            flags.push(format!("--allow-net={}", host));
        }
    }
    
    // child_process boolean
    if matches!(manifest.capabilities.process, Some(true)) {
        flags.push("--allow-child-process".to_string());
    }
    
    // worker / addons
    if matches!(manifest.capabilities.workers, Some(true)) {
        flags.push("--allow-worker".to_string());
    }
    if matches!(manifest.capabilities.native_addons, Some(true)) {
        flags.push("--allow-addons".to_string());
    }
    
    // ffi / inspector / wasi v1 一律不开
    
    Ok(flags)
}

fn canonicalize_path(p: &Path) -> Result<PathBuf, ExtensionError> {
    std::fs::canonicalize(p).map_err(|e| ExtensionError::Io(e))
}
```

**Tests**：覆盖每个 capability 的有/无组合，输出 flags 与预期一致。

**Done when**：所有 tests 通。

---

### P2-T03 · `host/node.rs` Node 进程管理

**Files**：`crates/cronymax/src/extensions/host/node.rs`

**Implement**：

```rust
pub struct NodeHost {
    pub extension_id: String,
    pub child: tokio::process::Child,
    pub socket_path: PathBuf,
    pub rpc: Arc<RpcConnection>,
    restart_count: u32,
    started_at: Instant,
}

pub struct NodeHostPool {
    hosts: HashMap<String, Arc<Mutex<NodeHost>>>,
}

impl NodeHostPool {
    pub async fn activate(&mut self, ext: &RegistryEntry, workspace: &Path) -> Result<Arc<Mutex<NodeHost>>> {
        if let Some(existing) = self.hosts.get(&ext.manifest.id) {
            return Ok(existing.clone());
        }
        let host = NodeHost::spawn(ext, workspace).await?;
        let h = Arc::new(Mutex::new(host));
        self.hosts.insert(ext.manifest.id.clone(), h.clone());
        Ok(h)
    }
    
    pub async fn deactivate(&mut self, id: &str) -> Result<()> { ... }
    pub async fn deactivate_all(&mut self) -> Result<()> { ... }
}

impl NodeHost {
    pub async fn spawn(ext: &RegistryEntry, workspace: &Path) -> Result<Self> {
        let socket_path = generate_socket_path(&ext.manifest.id);
        let ext_data_dir = ext_data_dir_for(&ext.manifest.id);
        
        // 起 RPC listener 先
        let listener = UnixListener::bind(&socket_path)?;
        
        // 准备 Node flags
        let mut flags = build_node_flags(&ext.manifest, workspace, &ext_data_dir)?;
        flags.push(bootstrap_js_path().to_string_lossy().into_owned());
        flags.push(format!("--ext-id={}", ext.manifest.id));
        flags.push(format!("--socket={}", socket_path.display()));
        
        let mut cmd = tokio::process::Command::new(node_binary_path());
        cmd.args(&flags)
           .stdin(Stdio::piped())
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());
        
        let child = cmd.spawn()?;
        
        // 等 Node 连进来（5s 超时）
        let (stream, _) = tokio::time::timeout(
            Duration::from_secs(5),
            listener.accept()
        ).await??;
        
        let rpc = Arc::new(RpcConnection::new(stream).await?);
        
        // 发 initialize RPC
        rpc.call("initialize", InitParams { extension_id: ext.manifest.id.clone() }).await?;
        
        Ok(Self {
            extension_id: ext.manifest.id.clone(),
            child,
            socket_path,
            rpc,
            restart_count: 0,
            started_at: Instant::now(),
        })
    }
    
    pub async fn activate(&self) -> Result<()> {
        self.rpc.call("extension/activate", ()).await
    }
    
    pub async fn deactivate(&self) -> Result<()> {
        self.rpc.call("extension/deactivate", ()).await
    }
    
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await?;
        Ok(())
    }
}

// 监控 host，崩了重启
async fn supervise(host: Arc<Mutex<NodeHost>>) { ... }
```

**Done when**：
- 能 spawn 一个 dummy Node 进程
- 进程异常退出 → 自动重启 N=3 次 → 失败禁用扩展
- 杀进程 PID → 自动重启

---

### P2-T04 · `rpc/` MessagePack-RPC

**Files**：`crates/cronymax/src/extensions/rpc/{codec,server}.rs`

**Implement**：基于 MessagePack-RPC 规范的简化版本（method id 用 string）。

```rust
// codec.rs
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Frame {
    // [0, msgid, method, params]
    Request(u8, u32, String, serde_json::Value),
    // [1, msgid, error, result]
    Response(u8, u32, Option<String>, serde_json::Value),
    // [2, method, params]
    Notification(u8, String, serde_json::Value),
}

// server.rs
pub struct RpcConnection {
    next_id: AtomicU32,
    pending: Arc<Mutex<HashMap<u32, oneshot::Sender<Result<Value>>>>>,
    write_tx: mpsc::Sender<Vec<u8>>,
}

impl RpcConnection {
    pub async fn call<P: Serialize, R: DeserializeOwned>(&self, method: &str, params: P) -> Result<R> { ... }
    pub async fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<()> { ... }
    pub fn register_handler<F>(&self, method: &str, handler: F)
        where F: Fn(Value) -> BoxFuture<Result<Value>> + Send + Sync + 'static { ... }
}
```

**Done when**：
- Rust 发 RPC，Node 收到并回 response
- Node 发 notification，Rust 收到
- 1000 round-trip P99 < 5ms

---

### P2-T05 · `bootstrap.js`

**Files**：`bundled/extension-host-bootstrap.js`

**Implement**：

```js
'use strict';
const net = require('node:net');
const vm = require('node:vm');
const path = require('node:path');
const fs = require('node:fs');
const { encode, decode } = require('@msgpack/msgpack');

const args = parseArgs(process.argv);
const SOCKET = args.socket;
const EXT_ID = args['ext-id'];
const EXT_DIR = path.dirname(args.manifest || `~/.cronymax/extensions/${EXT_ID}/cronymax-extension.json`);

// 连 Unix socket
const sock = net.createConnection(SOCKET);
const pending = new Map();
let nextId = 1;

function rpcCall(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    const frame = [0, id, method, params];
    sock.write(encode(frame));
  });
}

function rpcNotify(method, params) {
  const frame = [2, method, params];
  sock.write(encode(frame));
}

// 接受帧
let buf = Buffer.alloc(0);
sock.on('data', (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  // 简化版：每次解一帧（实际用 msgpack streaming）
  while (true) {
    try {
      const frame = decode(buf);
      handleFrame(frame);
      buf = Buffer.alloc(0);
    } catch { break; }
  }
});

const handlers = {};
function handleFrame([type, ...rest]) {
  if (type === 0) {
    const [id, method, params] = rest;
    const handler = handlers[method];
    if (!handler) return rpcRespondError(id, `unknown method ${method}`);
    Promise.resolve(handler(params))
      .then(r => sock.write(encode([1, id, null, r])))
      .catch(e => sock.write(encode([1, id, String(e), null])));
  } else if (type === 1) {
    const [id, err, result] = rest;
    const p = pending.get(id);
    if (!p) return;
    pending.delete(id);
    if (err) p.reject(new Error(err));
    else p.resolve(result);
  } else if (type === 2) {
    const [method, params] = rest;
    if (handlers[method]) handlers[method](params);
  }
}

// 加载扩展
let extensionModule = null;
let extensionCtx = null;

handlers['initialize'] = async ({ extension_id }) => {
  if (extension_id !== EXT_ID) throw new Error('id mismatch');
  return { ok: true, node_version: process.version };
};

handlers['extension/activate'] = async () => {
  const manifestPath = path.join(EXT_DIR, 'cronymax-extension.json');
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const mainPath = path.resolve(EXT_DIR, manifest.main);
  
  // 创建 vm.Context
  const ctx = vm.createContext({
    require: createRequireFor(EXT_ID, EXT_DIR),
    console: makeConsole(EXT_ID),
    setTimeout, setInterval, clearTimeout, clearInterval,
    fetch, WebSocket, AbortController, AbortSignal,
    URL, URLSearchParams, TextEncoder, TextDecoder,
    Buffer, process: makeProcessProxy(EXT_ID),
    Promise, Symbol, Reflect, Proxy,
    crypto: globalThis.crypto,
    module: { exports: {} },
    exports: {},
    __filename: mainPath,
    __dirname: path.dirname(mainPath),
  });
  
  const code = fs.readFileSync(mainPath, 'utf8');
  vm.runInContext(code, ctx, { filename: mainPath });
  
  extensionModule = ctx.module.exports;
  extensionCtx = ctx;
  
  if (typeof extensionModule.activate === 'function') {
    const extContext = makeExtensionContext(EXT_ID);
    await extensionModule.activate(extContext);
  }
  return { activated: true };
};

handlers['extension/deactivate'] = async () => {
  if (extensionModule && typeof extensionModule.deactivate === 'function') {
    await extensionModule.deactivate();
  }
  // dispose subscriptions
  return { deactivated: true };
};

function createRequireFor(extId, extDir) {
  const Module = require('node:module');
  const realRequire = Module.createRequire(path.join(extDir, '_'));
  return (name) => {
    if (name === '@cronymax/extension') return makeCronymaxApi(extId);
    // 其他都直通：Node 26 Permission 自己管
    return realRequire(name);
  };
}

function makeCronymaxApi(extId) {
  // 把 cronymax/v1 namespace 暴露给扩展
  // 每个调用底下 rpcCall(...)
  return {
    commands: {
      register: (id, handler) => {
        const localHandlers = (handlers._cmd ||= {});
        localHandlers[id] = handler;
        rpcCall('commands.register', { extId, id });
        return { dispose: () => { delete localHandlers[id]; rpcCall('commands.unregister', { extId, id }); } };
      },
      execute: (id, ...args) => rpcCall('commands.execute', { id, args }),
    },
    events: { /* ... */ },
    workspace: { /* ... */ },
    agents: { /* ... */ },
    // ...
  };
}

handlers['cmd/invoke'] = async ({ id, args }) => {
  const handler = handlers._cmd?.[id];
  if (!handler) throw new Error(`command not found: ${id}`);
  return await handler(...args);
};
```

**Done when**：spawn host → activate → register command → execute command round-trip 通。

---

### P2-T06 · `api/lifecycle.rs` + `api/commands.rs`

**Files**：`crates/cronymax/src/extensions/api/{lifecycle,commands}.rs`

**Implement**：Rust 侧 RPC handler 注册。

```rust
// api/commands.rs
pub fn register_handlers(rpc: &RpcConnection, registry: &Arc<Mutex<CommandRegistry>>) {
    let reg1 = registry.clone();
    rpc.register_handler("commands.register", move |params: Value| async move {
        let p: RegisterCommandParams = serde_json::from_value(params)?;
        reg1.lock().await.register(p.ext_id, p.id);
        Ok(Value::Null)
    });
    
    let reg2 = registry.clone();
    rpc.register_handler("commands.execute", move |params: Value| async move {
        let p: ExecuteCommandParams = serde_json::from_value(params)?;
        let result = reg2.lock().await.execute(&p.id, p.args).await?;
        Ok(result)
    });
}
```

**Done when**：hello-world 扩展能注册命令，CLI `cronymax cmd run hello.greet -- world` 能调到。

---

### P2-T07 · `@cronymax/extension` SDK v0

**Files**：`web/src/extensions/sdk/`

**Structure**：
```
sdk/
├── package.json
├── tsconfig.json
├── src/
│   ├── index.ts           ← export *
│   ├── lifecycle.ts
│   ├── commands.ts
│   └── rpc-client.ts      ← MessagePack-RPC（在扩展进程里跑）
└── dist/                  ← tsc 输出
```

`package.json`：
```json
{
  "name": "@cronymax/extension",
  "version": "0.1.0",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "dependencies": { "@msgpack/msgpack": "^3.0.0" }
}
```

**Done when**：`npm pack` 出来的 tgz 能被另一个项目 install。

---

### P2-T08 · hello-world 扩展（dogfood 验证）

**Files**：`tests/fixtures/extensions/hello-world/`

```
hello-world/
├── package.json
├── cronymax-extension.json
└── src/main.ts
```

**main.ts**：
```ts
import * as cronymax from "@cronymax/extension";

export async function activate(ctx) {
  ctx.subscriptions.push(
    cronymax.commands.register("hello.greet", (name) => `Hello, ${name}!`)
  );
}
```

**manifest**：
```json
{
  "id": "test.hello-world",
  "publisher": "test",
  "name": "Hello World",
  "version": "0.1.0",
  "main": "./dist/main.js",
  "activationEvents": ["onCommand:hello.greet"],
  "contributes": {
    "cronymax.command": [{ "id": "hello.greet", "title": "Hello: Greet" }]
  },
  "capabilities": {
    "extension-points": ["cronymax.command"]
  }
}
```

**Done when**：
1. 装它
2. 触发 onCommand:hello.greet
3. Node host 起来
4. activate 跑
5. command 调起来回 "Hello, world!"

---

### P2-T09 · 性能基准

**Files**：`crates/cronymax/benches/extensions.rs`（用 criterion）

**Benchmark**：1000 次 `commands.execute` round-trip，输出 P50/P95/P99/max。

**Pass criteria**：P99 < 5ms（本地 dev 机），P50 < 1ms。

---

### P2-T10 · 安全冒烟测试

**Files**：`tests/fixtures/extensions/security-test/src/main.ts`

```ts
import * as cronymax from "@cronymax/extension";
import * as fs from "fs";

export async function activate(ctx) {
  ctx.subscriptions.push(
    cronymax.commands.register("sec.try-read-etc-passwd", () => {
      try {
        fs.readFileSync('/etc/passwd');
        return "🚨 BYPASS";
      } catch (e) {
        return "✅ blocked: " + e.code;
      }
    }),
    cronymax.commands.register("sec.try-eval-fs", () => {
      try {
        eval("require('fs').readFileSync('/etc/passwd')");
        return "🚨 BYPASS";
      } catch (e) {
        return "✅ blocked: " + e.code;
      }
    }),
    cronymax.commands.register("sec.try-fetch-evil", async () => {
      try {
        await fetch("https://evil.example.com");
        return "🚨 BYPASS";
      } catch (e) {
        return "✅ blocked: " + e.code;
      }
    })
  );
}
```

manifest：`fs: { scope: "workspace", mode: "ro" }`, `network: { allow: ["api.openai.com"] }`。

**Done when**：3 个 command 都返回 `"✅ blocked"`。

---

## P3 · 其余 L1 Kernel（2 周）

每个原语流程一致：

1. Rust 端写 RPC handler 接 SDK 调用
2. SDK 端写 namespace 函数转 RPC
3. 写 focused 测试扩展

简略列出：

### P3-T01 · events
- `api/events.rs`：subscribe / emit + topic 路由 + capability 校验
- SDK：`cronymax.events.on/emit`
- Test：双向事件 round-trip

### P3-T02 · config
- `api/config.rs`：get/update + onDidChange
- 持久化 `~/.cronymax/extensions/<id>/config.json`
- SDK：`cronymax.workspace.getConfiguration`
- Test：值变更触发回调

### P3-T03 · secrets
- `api/secrets.rs`：macOS 用 `security` crate / Win DPAPI / Linux secret-service
- 跨扩展不可读（按 publisher namespace）
- SDK：`cronymax.secrets.{get,set,delete}`
- Test：跨扩展读 → 拒

### P3-T04 · storage
- `api/storage.rs`：sled / sqlite KV
- per-extension dir
- SDK：`ctx.globalState` / `ctx.workspaceState`

### P3-T05 · webview stub
- 接口签名定义好，实现 P6 做
- SDK：`cronymax.window.createWebviewPanel`（stub return）

### P3-T06 · auth
- `api/auth.rs`：内置 oauth-generic / pkce / device-flow
- SDK：`cronymax.authentication.getSession`
- Test：device-flow demo

### P3-T07 · extensions
- `api/extensions.rs`：getExtension + exports
- vm.Context 间共享 exports（VS Code 同款）
- Test：扩展 A export 函数，扩展 B 调

### P3-T08 · SDK 同步更新
- `@cronymax/extension` 加上面所有 namespace

### P3-T09 · Phase 3 验收
- 7 个测试扩展全过

---

## P4 · L2 EP × 6（2 周）

### P4-T01 · `contributions/` 通用注册中心

**Files**：`crates/cronymax/src/extensions/contributions/mod.rs`

```rust
pub struct ContributionRegistry {
    // ep_name → (ext_id → contributions)
    by_ep: HashMap<String, HashMap<String, Vec<Value>>>,
}

impl ContributionRegistry {
    pub fn register(&mut self, ep: &str, ext_id: &str, contributions: Vec<Value>) { ... }
    pub fn unregister(&mut self, ext_id: &str) { ... }
    pub fn list(&self, ep: &str) -> Vec<(String, &Value)> { ... }
    pub fn subscribe(&self, ep: &str) -> impl Stream<Item = ContributionChangeEvent> { ... }
}
```

通用机制：每个 EP 用同一份注册中心，平台 UI 模块只是 subscribe + 渲染。

### P4-T02 · `cronymax.command` 整合
- 已在 P2 完成，整合进 contribution 框架

### P4-T03 · `cronymax.config.schema`
- 设置面板组件：读 contribution，按 JSON Schema 自动生成表单
- SDK 已有 `cronymax.workspace.getConfiguration`

### P4-T04 · `cronymax.config.page`
- 依赖 P6 webview
- 设置面板里加 "Advanced" 标签，嵌入扩展提供的 HTML

### P4-T05 · `cronymax.agents.provider` ★核心★

**Files**：
- `crates/cronymax/src/extensions/api/agents.rs`
- `web/src/panels/chat/agent-picker.tsx`（修改）
- `crates/cronymax/src/flow/agent_step.rs`（修改，P8 完成）

**Rust trait**：
```rust
pub struct AgentProviderHandle {
    pub ext_id: String,
    pub provider_id: String,
    rpc: Arc<RpcConnection>,
}

impl AgentProviderHandle {
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.rpc.call("agents.listModels", &self.provider_id).await
    }
    pub async fn create_session(&self, opts: SessionOptions) -> Result<AgentSessionHandle> { ... }
}

pub struct AgentSessionHandle { ... }
impl AgentSessionHandle {
    pub async fn prompt(&self, msg: PromptMessage, token: CancellationToken) -> impl Stream<Item = AgentEvent> { ... }
    pub async fn cancel(&self) -> Result<()> { ... }
    pub async fn dispose(&self) -> Result<()> { ... }
}
```

**SDK**：
```ts
cronymax.agents.registerProvider("coco", {
  listModels: async () => [...],
  modes: [...],
  createSession: async (opts) => new MySession(opts),
});
```

**聊天面板改造**：
```ts
// web/src/panels/chat/agent-picker.tsx
const providers = bridge.send('extensions/contributions/list', {
  ep: 'cronymax.agents.provider'
});
// 把 providers 渲染成 dropdown
```

**Done when**：装个 mock-provider 测试扩展，聊天面板能列、能选、能开 session。

### P4-T06 · `cronymax.content.renderer`
- block 类，先支持 mermaid mime
- 平台读 contribution，匹配 mime 后渲染 iframe

### P4-T07 · `cronymax.ui.sidebar.view`
- 依赖 P6
- 扩展能贡献侧栏面板

### P4-T08 · permissionRequest 桥
- AgentEvent permissionRequest → 平台权限弹窗
- 用户决策回 agent provider

---

## P5 · L1.5 Events（1 周，与 P3/P6 并行）

### P5-T01 · `extensions/events.rs` 平台事件总线
- subscribe / emit + capability 校验
- 已有 `api/events.rs` 是扩展 ↔ 扩展事件；这里加平台 → 扩展事件

### P5-T02 · 聊天面板 emit
位置：`web/src/panels/chat/` 各处 emit
- `cronymax.session.started`
- `cronymax.session.ended`
- `cronymax.message.user.sent`
- `cronymax.message.assistant.delta`
- `cronymax.message.assistant.done`
- `cronymax.permission.requested`

### P5-T03 · Tool 调度 emit
- `cronymax.tool.invoked`
- `cronymax.tool.completed`

### P5-T04 · logger 测试扩展
- 订阅 `cronymax.message.assistant.done`
- 写到 `~/.cronymax/logs/agent-messages.jsonl`

---

## P6 · Webview 基建（2 周，与 P3/P5 并行）

### P6-T01 · CEF 协议处理器
- 注册 scheme `cronymax-webview://`
- 路由到 `~/.cronymax/extensions/<ext-id>/`

### P6-T02 · iframe sandbox CSP
- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src cronymax-webview: data:;`

### P6-T03 · `acquireCronymaxApi()` 注入
- 在 iframe 内注入 global `acquireCronymaxApi` 函数
- 返回 { postMessage, onDidReceiveMessage, setState, getState }

### P6-T04 · 跨进程 postMessage 中转
- iframe → CEF browser → Rust core → Node host 路由
- 反向同理
- 鉴权：每个 panel 标记归属哪个扩展

### P6-T05 · ping 测试扩展
- HTML 按钮点 → postMessage `{type: "ping"}` → 扩展 main.ts handler → 回 `{type: "pong"}` → HTML 更新

### P6-T06 · 隔离测试
- 装两个扩展 A、B，各自 webview
- A 的 webview 尝试 postMessage 给 B → 应失败

---

## P7 · `bytedance.coco` Dogfood（2 周）

### P7-T01 · 项目骨架
```
coco-extension/
├── package.json (含 esbuild + tsc)
├── cronymax-extension.json
├── tsconfig.json
├── src/{main.ts, acp-client.ts, coco-session.ts, settings/...}
```

### P7-T02 · manifest（按 spec §11 写）

### P7-T03 · `src/acp-client.ts`
- 端口 `/tmp/acp_mcp_client.py` 到 TS
- 直接 `import { spawn } from "child_process"`
- JSON-RPC 协议帧

### P7-T04 · `src/coco-session.ts`
- ACP event → AgentEvent 翻译表（spec §11.3）
- 覆盖所有 kind

### P7-T05 · `src/main.ts`
- register agents.provider("coco", ...)
- register commands

### P7-T06 · 设置页
- OAuth 流（如有）+ 模型探查
- postMessage 桥到 main.ts

### P7-T07 · 打包
- esbuild bundle CJS / target node22+ → dist/main.js 单文件
- HTML/CSS 资源拷到 dist/

### P7-T08 · 打包 .crx + E2E
- `cronymax ext package coco-extension/` → coco.crx
- 装 → 选 → 聊天 → 验

### P7-T09 · 通用性 checkpoint
- `grep -r "cronymax/src" coco-extension/` 应为空（只 import @cronymax/extension）

### P7-T10 · 第二 dogfood mermaid
- 类似但 content.renderer 而非 agents.provider

---

## P8 · Flow Agent 接通（1 周，与 P7 末并行）

### P8-T01 · agent_loader.rs 改造
- AgentDef 加 `provider: String` + `provider_config: Value` 字段
- 老 yaml 默认 `provider: "native"`

### P8-T02 · flow agent step 改造
- 通过 `cronymax.extensions.api.agents` registry 拿 provider
- allowed_tools 透传

### P8-T03 · agents UI
- `web/src/panels/flows/agents/AgentList.tsx`
- `web/src/panels/flows/agents/NewAgentWizard.tsx`（4 步）
- `web/src/panels/flows/agents/AgentEditor.tsx`

### P8-T04 · 验收
- UI 建 code-reviewer 绑 coco
- flow step type=agent agent=code-reviewer 跑通

---

## P9 · SDK + 扩展管理 UI（2 周）

### P9-T01 · IDL → TS codegen
- 读 `cep-idl/v1/*.ts` AST，生成 SDK 的 .d.ts

### P9-T02 · SDK v1.0.0 发布到内网 npm

### P9-T03 · 模板仓 `cronymax-extension-template`
- 4 个示例：hello-world / theme / content-renderer / agent-provider

### P9-T04 · 设置面板 - Extensions 标签
- 列表 + 详情 + 安装/卸载/启用/禁用 + 撤销 capability

### P9-T05 · CLI 补全
- `cronymax ext package <dir>` → .crx
- `cronymax ext dev <dir> --watch` → 热重载

### P9-T06 · 外部同事 dogfood
- 给一个不知道 cronymax 内部的同事 + 模板仓 + README
- 看他半天内能不能写出可用扩展

---

## P10 · 收尾 + Alpha（1 周）

### P10-T01 · Crash 恢复测试
- 各种崩溃场景（panic / OOM / SIGKILL）→ 自动重启 → 失败禁用

### P10-T02 · 资源额度
- ulimit / cgroups（Linux）/ Job Object（Win）/ rlimit（macOS）
- CPU / 内存 / 子进程数 / iframe 数

### P10-T03 · Capability 弹窗 UX 终稿

### P10-T04 · 预热常用扩展
- cronymax 启动按上次使用列表静默 activate

### P10-T05 · 文档
- spec / SDK API ref / 开发者指南 / 安全模型说明

### P10-T06 · 性能基准 + 优化

### P10-T07 · v1 Alpha 验收

跑 spec-v0.3 §15 验收清单全 11 项：
1. 装 bytedance.coco，授权对话框人话清单
2. 聊天面板列 Coco
3. 选 Coco 发消息流式渲染
4. 设置面板能改 binaryPath + 设置页能开
5. flows 用向导建 agent 绑 coco
6. flow step type=agent 跑通
7. mermaid 渲染器扩展工作
8. 杀 Node host 自动重启
9. 卸载残留清干净
10. 安全：4 种绕路全拦
11. 外部同事半天内写出扩展

全过 = Alpha 发布。

---

## 通用工作约定

### 每个任务的 Definition of Done

- [ ] 代码 commit（清晰 commit message）
- [ ] Unit test 覆盖核心逻辑
- [ ] 没有 `unwrap()` 在生产路径（只在 test）
- [ ] tracing 日志补全（target = `"cronymax::extensions"`）
- [ ] 改动 cep-idl 的话同步 SDK
- [ ] PR description 链回任务 ID

### 跨阶段常用命令

```bash
# 编译 + 测试
cargo build -p cronymax
cargo test -p cronymax extensions::

# 装扩展
cargo run -p cronymax -- ext install tests/fixtures/extensions/hello-world

# 跑命令
cargo run -p cronymax -- cmd run hello.greet -- world

# 查看日志
RUST_LOG=cronymax::extensions=debug cargo run -p cronymax

# 性能基准
cargo bench -p cronymax extensions
```

### 出问题怎么办

1. **读 spec-v0.3** —— 优先看 spec 写了什么，本文档只是执行步骤
2. **不确定就停**，发问而不是猜
3. **冲突时**：spec > plan > tasks > exec-plan
4. **改决策**：先改 spec，再回头同步 plan + tasks + exec-plan
5. **绕过通用性规则**：禁。需要为某个扩展加后门 = 设计错了，停下加 EP 或事件主题

---

## 启动闸门（每次新 session 开工前问自己）

- [ ] Phase 几？现在在做哪个任务 ID？
- [ ] 上一次 commit 是什么？git log 有没有进度
- [ ] 那个任务的 Done when 是什么
- [ ] 任何 spec 决策最近改过吗

---

文档版本：v0.3 · 2026-05-19
所有任务都映射到 [`tasks-v0.3.md`](tasks-v0.3.md) 的对应 ID。
