# Legacy Flow Agent Step 现状摸底

> Phase 8 改造前的快照。配合 spec-v0.3 §4 + §11 + §12 看。
> 摸底范围：`crates/cronymax/src/agent_loop/`、`crates/cronymax/src/capability/agent_loader.rs`、`crates/cronymax/src/flow/`、`crates/cronymax/src/llm/`、`crates/cronymax/src/runtime/{handler,agent_runner}.rs`、`web/src/panels/chat/`。

---

## 1. 现状

### 1.1 AgentDef 数据结构

定义在 `crates/cronymax/src/capability/agent_loader.rs:41-94`。字段（运行时使用的子集）：

- `name`、`kind: AgentKind`（`Worker` | `Reviewer`，`agent_loader.rs:97-102`）
- `llm_provider: String`、`llm_model: String`（空串表示 "用 workspace 默认"）
- `system_prompt`、`prompt_source: PromptSource`（`Builtin` / `UserYaml(path)`，`agent_loader.rs:30-35`）
- `memory_namespace`、`tools: Vec<String>`、`reasoning_effort`、`inject_workspace`、`vars`、`reflection`

**关键观察**：当前 `AgentDef` 没有 `provider` / `provider_config` 字段 —— `llm_provider` 是 LLM 后端选择字符串（`"copilot"` / `"openai"`），不是 spec-v0.3 §4 的 `agents.provider`（扩展贡献的 AgentProvider id）。

### 1.2 agent yaml 解析

入口：`load_agent(workspace_root, agent_id)` (`agent_loader.rs:264-267`)，固定从 `<workspace>/.cronymax/agents/<id>.agent.yaml` 加载；失败回退默认 `AgentDef`（`agent_loader.rs:327-338`）。

特殊路径：`load_agent_with_builtin` (`agent_loader.rs:278-315`) 拦截 `""` / `"crony"`，返回 `CronyBuiltin::def()` + 外围 override（只允许 `memory_namespace` / `vars` / `reflection_enabled`，prompt 永远 `Builtin`）。

YAML 形状（`RawAgentDef`，`agent_loader.rs:116-136`）：`name` / `kind` / `llm`（既支持 `llm: gpt-4o` 字符串又支持 mapping `{provider, model, reasoning_effort}`，`agent_loader.rs:192-213`） / `system_prompt` / `memory_namespace` / `tools` / `reasoning_effort` / `inject_workspace`。**未声明字段静默忽略**（doc comment, `agent_loader.rs:5-6`）。

### 1.3 flow runtime 调度路径

flow 不存在 `agent_step.rs` 文件。`type: agent` 的概念分散在以下处：

- **`FlowDefinition`**（`flow/definition.rs:387-414`）字段：`name` / `description` / `agents: HashMap<String, String>`（agent ID → 相对路径）/ `nodes: Vec<FlowNode>` / `graph`。`FlowNode` (`definition.rs:99-104`) 只有 `id` / `owner` / `outputs`，**没有 `type` 字段**。"是否走 agent" 由 `owner != "human"` 决定（`flow/runtime.rs:433-454`）。
- **`InvocationContext`**（`flow/runtime.rs:152-166`）：`node_id` / `owner` / `trigger` / `available_docs` / `pending_ports` / `review_comments`。是 flow → agent loop 的传递信封。
- **调度入口**：`FlowRuntime::schedule_node_with_context` (`flow/runtime.rs:962-987`) 只构造 `InvocationContext` + 写 trace，**不直接起 agent**；agent 启动由 `AgentRunner::spawn_agent` 完成（见下节）。
- **触发点**：`fire_and_join_for` (`flow/runtime.rs:404-457`)、`on_document_approved` (`flow/runtime.rs:600-718`)、`on_rejected_requeue` (`flow/runtime.rs:722-752`)、`on_reviewer_verdict` (`flow/runtime.rs:764-856`)。每个返回 `Vec<InvocationContext>` 给 caller，caller（`RuntimeHandler` 的 supervision task）再分别 `spawn_chat` 或 `spawn_agent`（`runtime/handler.rs:686-732`）。

### 1.4 当前 agent 怎么找 LLM provider

不存在 "AgentProvider" 抽象。`AgentRunner::spawn_agent` (`runtime/agent_runner.rs:45-220`) 内联做这些事：

1. `agent_loader::load_agent(&run_ctx.workspace_root, &agent_id)` 拿 `AgentDef`（`agent_runner.rs:62`）
2. 用 `agent_def.system_prompt` + `render_system_message(&inv_ctx)` 拼 system prompt（`agent_runner.rs:64-80`，渲染函数在 `agent_runner.rs:368-476`）
3. 直接调 `services.llm_factory.build(&effective_llm_config)` 拿 `Arc<dyn LlmProvider>`（`agent_runner.rs:187`）
4. 直接调 `services.capability_factory.build(...)` 拿工具 dispatcher（`agent_runner.rs:98-152`）
5. 组 `LoopConfig` → `ReactLoop::new(authority, run_id, cfg).run().await`（`agent_runner.rs:196-214`）

**LLM provider 是 closed enum `LlmConfig`**（`llm/config.rs:9-34`）：`OpenAi` / `Anthropic` / `Copilot` 三种，由 `DefaultLlmProviderFactory::build`（`llm/factory.rs:67-134`）match 分发，对应三个具体 provider（`OpenAiProvider`、`AnthropicProvider`、复用 OpenAI client + `copilot_mode: true`）。`agent_def.llm_provider` 字段读出来但**没有任何 dispatch 逻辑用它**（grep 整个 crate 只在 `agent_loader.rs` 内部赋值，没有读消费者）；当前只用 `agent_def.llm_model` 来覆盖 `run_ctx.llm_config` 里的 model（`agent_runner.rs:82-93`、`apply_model_override` `agent_runner.rs:481-507`）。

`LlmProvider` trait 本身（`llm/provider.rs:60-66`）极简：单 `stream(request) -> Stream<LlmEvent>` 方法。`LlmEvent` 枚举（`llm/provider.rs:18-52`）：`Delta` / `ThinkingDelta` / `ToolCallDelta` / `Usage` / `Done` / `Error`。

`LlmProviderRegistry`（`llm/registry.rs:76-191`）只持久化用户配的 provider 配置（base_url / api_key / model override），**和扩展贡献的 AgentProvider 不是同一个概念**。

### 1.5 chat 面板 vs flow runtime —— 是否共用一份 provider 抽象

**完全共用同一个底层路径**，区别只是 wrapper：

- chat 面板入站（`web/src/panels/chat/App.tsx:1668-1676`）：UI 把 `provider_kind` / `base_url` / `api_key` / `model` 当 `agentRun` 入参，runtime 侧 `RuntimeHandler` (`runtime/handler.rs:415` 附近) 用 `LlmConfig::from_payload_fields` 解析。
- chat 路径 `RuntimeHandler::start_run` 直接构造 `LoopConfig` + `ReactLoop`（`runtime/handler.rs:1326-1364`、`runtime/handler.rs:1665-1687`），加载 `chat_agent_def` 用的也是 `load_agent_with_builtin`。
- flow 路径：supervision task 收到 `DocumentSubmitted` 后调 `FlowRuntime::on_document_submitted` 得到 `Vec<InvocationContext>`，对每个非 human owner 调 `AgentRunner::spawn_agent`（`runtime/handler.rs:686-732`、`runtime/agent_runner.rs:45`）；spawn_agent 内同样调 `services.llm_factory.build(...)` + 同样组 `LoopConfig` + 同样跑 `ReactLoop::new(...).run()`。
- `AgentRunner::spawn_chat` (`runtime/agent_runner.rs:227-358`) 用于 flow 通知人类 reviewer 时反向给原 chat session 推一段消息，**也是同一个 ReactLoop 流程**，只是 `initial_thread` 从 ChatStore / authority session 取，且 `session_id` 注入 `LoopConfig` 让 ReactLoop 跑完后 flush 回原 session。

**结论**：chat 和 flow 共用 `ReactLoop` + `LlmProvider` + `CapabilityFactory` + `agent_loader`，但**没有 spec-v0.3 §4 意义上的 `AgentProvider` 抽象**。当前 "provider" 一词在代码里 = LLM 后端枚举 + 配置；spec 里要新增的 `AgentProvider` = "整个会话能力（含 LLM 调用 + tool 系统 + session 生命周期）的可插拔单元"，目前是隐式由 `AgentRunner::spawn_agent` 完成的固定流程。

---

## 2. Phase 8 改造点

### 2.1 AgentDef 扩字段

文件：`crates/cronymax/src/capability/agent_loader.rs`

- `AgentDef` 加 `provider: String`（默认 `"native"`，承载老 yaml 兼容性）+ `provider_config: HashMap<String, serde_json::Value>`（`agent_loader.rs:41-94`）。
- `RawAgentDef` (`agent_loader.rs:116-136`) 加 `provider: Option<String>` + `provider_config: Option<serde_yml::Value>`，`into_agent_def` (`agent_loader.rs:176-238`) 里映射进去。
- `Default for AgentDef` (`agent_loader.rs:157-174`) 同步加默认值。
- 现有 `llm_provider` / `llm_model` 字段含义不变，但当 `provider != "native"` 时被忽略（路由权交给扩展 provider 的 `provider_config`，比如 `provider_config.model`）。

### 2.2 新增 AgentProvider registry

文件：`crates/cronymax/src/runtime/` 下新增（具体目录由 Phase 8 决定，可能复用 `runtime/services.rs`）。

新建 trait：`AgentProvider`，行为参照 spec-v0.3 §4 的 `interface AgentProvider`：`listModels` / `modes?` / `createSession(SessionOptions) -> AgentSession`。`AgentSession::prompt` 返回 `AsyncIterable<AgentEvent>`，需要从 `LlmEvent` 派生但更宽（加 `permissionRequest` / `done.stopReason`）。

新建 `AgentProviderRegistry`：`Arc<DashMap<String, Arc<dyn AgentProvider>>>` 之类，在 `RuntimeServices` 里挂一个。`"native"` provider 在启动时由 `RuntimeServices` 注册，内部实现就是封装现在 `AgentRunner::spawn_agent` 的固定流程（拿 `LlmConfig` + `CapabilityFactory` 起 `ReactLoop`）。

### 2.3 flow → AgentProvider 调用迁移

文件：`crates/cronymax/src/runtime/agent_runner.rs`

- `AgentRunner::spawn_agent` (`agent_runner.rs:45-220`)：
  1. `load_agent` 之后多一步 —— 读 `agent_def.provider`，从 registry 拿 `Arc<dyn AgentProvider>`。
  2. 把现在的 `LoopConfig` 组装 + `ReactLoop::new(...).run()` 替换为 `provider.createSession(SessionOptions { cwd, model, mode, mcpServers, systemPrompt, allowedTools })` 然后 `session.prompt(...)` 迭代。
  3. `LlmConfig` / `llm_factory.build` / `capability_factory.build` 的直接调用，挪到 `"native"` provider 的 `createSession` 实现里。
  4. `apply_model_override` (`agent_runner.rs:481-507`)、`build_middleware_chain` (`agent_runner.rs:514-532`) 这些 helper 一并进 `"native"` provider。
- `render_system_message` (`agent_runner.rs:368-476`) 留在 `agent_runner.rs`（与 provider 无关，是 flow → agent 的 invocation context 渲染），把渲染结果当 `SessionOptions.systemPrompt` 传下去。
- AgentEvent → RuntimeEventPayload 翻译（现在 `ReactLoop::drive` 内嵌完成，`agent_loop/react.rs:339-415`）要么挪到 `AgentRunner` 统一翻译，要么由 `"native"` provider 内部继续做（其他扩展 provider 各自负责）。

### 2.4 chat 路径同步迁移

文件：`crates/cronymax/src/runtime/handler.rs`

- 直接 chat（无 flow_id）的 `LoopConfig` + `ReactLoop` 组装（`handler.rs:1326-1364`、`handler.rs:1665-1687`）也要走 `AgentProviderRegistry` 拿 `"native"` provider（用 `Chat.agent.yaml` 的 `provider` 字段，缺省 `"native"`）。
- `resume_run` 路径同样（`handler.rs:1601-1687`）。
- `AgentRunner::spawn_chat` (`agent_runner.rs:227-358`) 走同一个 registry 路径。

### 2.5 agent yaml 字段约定

文件：`crates/cronymax/src/capability/agent_loader.rs`

- `provider: native`（默认） → 走 `"native"` provider，内部读 `agent_def.llm_provider` / `agent_def.llm_model` / `agent_def.reasoning_effort`（保留全部老语义）。
- `provider: coco`（或其它扩展贡献的 id）→ 走扩展 registry，`provider_config` 的内容（`model: GPT-5.4` / `mode: plan` / `mcp_servers` / 任意）当 `serde_json::Value` 透传给 `AgentProvider::createSession`，**runtime 不解读**。

### 2.6 invocation 数据流

文件：无文件改动，但要确认数据通路：

- `InvocationContext` (`flow/runtime.rs:152-166`) 当前由 `render_system_message` (`agent_runner.rs:368-476`) 翻成 markdown system prompt。改造后这段 markdown 当 `SessionOptions.systemPrompt` 传给 provider，**不需要新增字段**。
- `agent_def.tools` (`agent_loader.rs:74`) 改造后映射到 `SessionOptions.allowedTools`。
- `agent_def.system_prompt` (`agent_loader.rs:61`) + 渲染好的 invocation context 拼接（`agent_runner.rs:66-80`）后整体传 `SessionOptions.systemPrompt`。

---

## 3. 兼容性

### 3.1 老 yaml 缺 `provider` 字段 → 默认 `"native"`

- `RawAgentDef.provider: Option<String>` 反序列化时 `serde(default)` 给 `None`，`into_agent_def` 里 `unwrap_or_else(|| "native".to_owned())`。所有现存 `<agent>.agent.yaml`（含 `Crony.agent.yaml` override）继续按 `"native"` 走。
- 现有 `RawAgentDef` 已经有 `#[serde(default)]` 习惯（`agent_loader.rs:117-136`）+ "unknown YAML keys silently ignored"（`agent_loader.rs:6` 注释），新字段加入不破坏现有文件。

### 3.2 "native" provider 内部走老路径

- `"native"` provider 的 `createSession` 实现 = 现在 `AgentRunner::spawn_agent` 函数体（`agent_runner.rs:62-219`）里"拿 AgentDef 之后到 ReactLoop 之前"的全部 LLM / capability 组装逻辑。
- `LlmConfig` (`llm/config.rs:9-34`)、`DefaultLlmProviderFactory` (`llm/factory.rs:67-134`)、`LlmProviderRegistry` (`llm/registry.rs:76-191`) 全部保留，只是从 `"native"` provider 内部消费，不再被 `AgentRunner` 直接 `match`。

### 3.3 数据流对称性

- chat 面板 / `RuntimeHandler::start_run` 直接 chat 路径 → 通过 `Chat.agent.yaml`（默认 `provider: native`）→ `AgentProviderRegistry::get("native")` → 同一个 LoopConfig。
- flow `type: agent` 节点 / `AgentRunner::spawn_agent` → 通过 `<agent>.agent.yaml`（默认 `provider: native`）→ `AgentProviderRegistry::get(...)` → 同一个 LoopConfig 或扩展会话。
- 两条路径在 `AgentProviderRegistry` 收敛，符合 spec-v0.3 §4 末段 "聊天面板 + flow runtime（共享同一份 registry）" 的描述。

---

## 4. 风险/不确定

### 4.1 耦合较深处

- **`ReactLoop` 与 `RuntimeAuthority` 强耦合**：`react.rs:148-244` 直接调 `authority.mark_run_running` / `emit_for_run` / `open_review_with_completion` / `complete_run` / `fail_run` / `flush_thread`。这些是 spec-v0.3 §4 AgentSession 抽象底下的实现细节。如果 `"native"` provider 还想复用 `ReactLoop`，要把 `RuntimeAuthority` 从 `ReactLoop` 里挖出来（或让 `"native"` provider 持有 authority），不然扩展 provider 无法对称发出同样的事件。
- **`AgentRunner::spawn_agent` 内嵌 `register_submit_review` / `register_flow_tools` 等 flow 专用工具注册**（`agent_runner.rs:121-146`、`agent_runner.rs:280-294`）：这些和 capability dispatcher 绑定，扩展 provider 没法直接复用（它们用自己的 tool 体系）。改造时要把这块逻辑明确归属到 `"native"` provider；扩展 provider 用 spec-v0.3 §4 的 `allowedTools` 自己映射。
- **`apply_model_override` 硬编码 `LlmConfig` 三个 variant**（`agent_runner.rs:481-507`）：扩展 provider 不用 `LlmConfig`，这套逻辑只属于 `"native"` provider。
- **`render_system_message` 把 invocation context 渲成 markdown**（`agent_runner.rs:368-476`）：和 `submit_document` / `flow_submit_review` 工具名硬编码绑定。扩展 provider 收到这段 prompt 时可能没有同名工具，需要扩展自己映射或文档约定。

### 4.2 chat 路径绕过点

- `RuntimeHandler::start_run` (`handler.rs:1326-1364`)、`RuntimeHandler::resume_run` (`handler.rs:1665-1687`) **直接构造 `LoopConfig` + `ReactLoop`**，没经过 `AgentRunner`。Phase 8 必须把这两处也改成走 `AgentProviderRegistry`，不然 chat 路径仍然绕过抽象。
- 直接 chat 还有"基于 Chat.agent.yaml + workspace 注入块"的 system_prompt 拼装逻辑（`handler.rs:1116-1139`），和 flow 路径的 `render_system_message` 不一样。两条路径需要在 `SessionOptions.systemPrompt` 里继续保留各自的拼装，但拼装时机要在调 `createSession` 之前。

### 4.3 事件 enum 映射

spec-v0.3 §4 的 `AgentEvent`：

```
text | thinking | toolCall | toolCallUpdate | permissionRequest | done
```

当前 `LlmEvent` (`llm/provider.rs:18-52`)：

```
Delta | ThinkingDelta | ToolCallDelta | Usage | Done | Error
```

`RuntimeEventPayload`（`crates/cronymax/src/protocol/events.rs`，未细读）会额外发 `PermissionRequest` / `Trace` / `Token` / `ThinkingToken` 等。映射差异：

- spec `toolCallUpdate` 是 tool 执行**结果**（status: completed / failed + output），当前由 `ReactLoop::run_one_tool` (`react.rs:531-640`) 拿到 `ToolOutcome` 后写进 `history` 并经 middleware 发 trace，**没有作为 LlmEvent 一档**。如果要让扩展 provider 对称发事件，需要 AgentEvent 层新增一档。
- spec `permissionRequest` 当前由 `RuntimeAuthority::open_review_with_completion` (`react.rs:562-571`) 通过 `RuntimeEventPayload::PermissionRequest` 发；扩展 provider 要么也调 authority，要么 spec 层定义新 trait method 让 runtime 自己开 review。
- spec `done.stopReason` 当前由 `LlmEvent::Done { finish_reason }` (`llm/provider.rs:48`) 承载，但 `ReactLoop` 内部还会因为 `Terminal` 工具 / `MaxTurns` / `Cancelled` 多种 LoopError 提前终止（`react.rs:25-37`、`react.rs:451-526`），这些状态需要全部能映射成 spec 的 `done.stopReason`。
- thinking buffer 当前累积但不入 history（`react.rs:334`、`react.rs:341-351`），spec 也未要求落 history，行为一致。

### 4.4 其他

- `agent_def.llm_provider` 字段在代码里被解析但**没有任何消费方**（搜索全 crate 只有 `agent_loader.rs` 内部赋值）。改造时要决定：保留并定义"`provider=native` 时由 llm_provider 字段决定 LLM backend"，还是直接砍掉用 `provider_config.llm_provider` 替代。
- `LlmConfig` (`llm/config.rs:10`)、`LlmProviderKind` (`llm/registry.rs:39-44`)、`DefaultLlmProviderFactory::build` (`llm/factory.rs:69`) 都是**封闭 enum**，扩展 provider 不可能往里加 variant，符合 Phase 8 把 LLM 后端选择和 AgentProvider 抽象分层的预期（LlmConfig 是 `"native"` provider 的内部细节，扩展 provider 完全不碰）。
- `FlowDefinition.agents`（`flow/definition.rs:395`）是 `HashMap<String, String>`（agent ID → YAML 路径）。Phase 8 改造不需要动 flow yaml schema，agent 选择仍然是 `node.owner = "<agent_id>"`，"是否走扩展 provider" 完全由该 agent 的 `<id>.agent.yaml` 里的 `provider:` 字段决定。
