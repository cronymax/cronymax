//! [`HostCapabilityDispatcher`]: a [`ToolDispatcher`] that routes each
//! tool call to a registered [`CapabilityProvider`].
//!
//! Tools are registered at construction time. Each registered tool
//! must declare:
//!
//! * A JSON-schema [`ToolDef`] that the LLM sees on every turn.
//! * An async handler that takes the raw `arguments` JSON and returns
//!   a [`ToolOutcome`].
//!
//! This lets hosts (and tests) compose exactly the tool set they want
//! without hard-coding capability routing inside the agent loop.
//!
//! ## Construction
//!
//! ```ignore
//! use std::sync::Arc;
//! use cronymax::capability::dispatcher::HostCapabilityDispatcher;
//!
//! let mut builder = HostCapabilityDispatcher::builder();
//! builder.register_shell(Arc::new(my_shell_provider));
//! builder.register_browser(Arc::new(my_browser_provider), space_id);
//! let dispatcher = builder.build();
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tracing::warn;

use crate::agent_loop::tools::{ToolDispatcher, ToolOutcome};
use crate::llm::{ToolCall, ToolDef};

use super::browser::{BrowserCapability, PageInspectRequest};
use super::filesystem::{FilesystemCapability, WorkspaceScope};
use super::notify::{NotifyCapability, NotifyRequest, ApprovalRequest, ApprovalResponse};
use super::shell::{ShellCapability, ShellRequest};

// ── Handler type alias ───────────────────────────────────────────────────────

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// A dynamic tool handler: maps raw JSON arguments to a [`ToolOutcome`].
type HandlerFn = Arc<dyn Fn(String) -> BoxFuture<ToolOutcome> + Send + Sync>;

// ── Registered tool ──────────────────────────────────────────────────────────

struct RegisteredTool {
    def: ToolDef,
    handler: HandlerFn,
    /// If true, dispatch requires approval (surfaces `NeedsApproval`
    /// on first call; `dispatch_approved` calls the handler directly).
    needs_approval: bool,
}

// ── HostCapabilityDispatcher ─────────────────────────────────────────────────

/// A [`ToolDispatcher`] that routes tool calls to registered capability
/// providers. Built via [`DispatcherBuilder`].
#[derive(Debug)]
pub struct HostCapabilityDispatcher {
    tools: HashMap<String, RegisteredTool>,
}

impl HostCapabilityDispatcher {
    pub fn builder() -> DispatcherBuilder {
        DispatcherBuilder::new()
    }

    /// Returns the [`ToolDef`] for `name`, if registered.
    pub fn tool_def(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(name).map(|t| &t.def)
    }
}

#[async_trait]
impl ToolDispatcher for HostCapabilityDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        let mut defs: Vec<ToolDef> = self.tools.values().map(|t| t.def.clone()).collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    async fn dispatch(&self, call: &ToolCall) -> ToolOutcome {
        match self.tools.get(&call.name) {
            None => ToolOutcome::Error(format!("no tool registered: {}", call.name)),
            Some(reg) if reg.needs_approval => ToolOutcome::NeedsApproval {
                request: serde_json::json!({
                    "tool": call.name,
                    "arguments": call.arguments,
                }),
            },
            Some(reg) => (reg.handler)(call.arguments.clone()).await,
        }
    }

    async fn dispatch_approved(&self, call: &ToolCall) -> ToolOutcome {
        // Bypass the approval gate and run directly.
        match self.tools.get(&call.name) {
            None => ToolOutcome::Error(format!("no tool registered: {}", call.name)),
            Some(reg) => (reg.handler)(call.arguments.clone()).await,
        }
    }
}

// ── Debug helper ─────────────────────────────────────────────────────────────

impl std::fmt::Debug for RegisteredTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegisteredTool")
            .field("name", &self.def.name)
            .field("needs_approval", &self.needs_approval)
            .finish()
    }
}

// ── Builder ──────────────────────────────────────────────────────────────────

/// Fluent builder for [`HostCapabilityDispatcher`].
pub struct DispatcherBuilder {
    tools: HashMap<String, RegisteredTool>,
}

impl DispatcherBuilder {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    // ── Low-level registration ────────────────────────────────────────────

    /// Register a custom tool with a handler function. `needs_approval`
    /// gates the first dispatch behind a [`ToolOutcome::NeedsApproval`].
    pub fn register<F, Fut>(
        &mut self,
        def: ToolDef,
        needs_approval: bool,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ToolOutcome> + Send + 'static,
    {
        let handler = Arc::new(move |args: String| -> BoxFuture<ToolOutcome> {
            Box::pin(handler(args))
        });
        let name = def.name.clone();
        self.tools.insert(
            name,
            RegisteredTool { def, handler, needs_approval },
        );
        self
    }

    // ── Capability-specific helpers ───────────────────────────────────────

    /// Register a `run_shell` tool backed by `provider`.
    /// Shell execution requires approval by default (`needs_approval: true`).
    pub fn register_shell(
        &mut self,
        provider: Arc<dyn ShellCapability>,
        needs_approval: bool,
    ) -> &mut Self {
        let def = ToolDef {
            name: "run_shell".into(),
            description:
                "Execute a shell command in the workspace sandbox. \
                 Returns stdout, stderr, and exit code."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to run (passed to /bin/sh -c)"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory relative to workspace root"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Execution timeout in seconds"
                    }
                },
                "required": ["command"]
            }),
        };
        self.register(def, needs_approval, move |args| {
            let provider = provider.clone();
            async move {
                let req: ShellRequest = match serde_json::from_str(&args) {
                    Ok(r) => r,
                    Err(e) => {
                        return ToolOutcome::Error(format!("invalid run_shell args: {e}"))
                    }
                };
                match provider.run(req).await {
                    Ok(result) => ToolOutcome::Output(serde_json::json!({
                        "exit_code": match result.exit_status {
                            crate::capability::shell::ExitStatus::Code(c) => c,
                            _ => -1,
                        },
                        "stdout": result.stdout,
                        "stderr": result.stderr,
                        "elapsed_ms": result.elapsed_ms,
                    })),
                    Err(e) => ToolOutcome::Error(format!("shell execution failed: {e}")),
                }
            }
        })
    }

    /// Register a `inspect_page` tool backed by `provider`.
    pub fn register_browser(
        &mut self,
        provider: Arc<dyn BrowserCapability>,
        space_id: crate::runtime::state::SpaceId,
    ) -> &mut Self {
        let def = ToolDef {
            name: "inspect_page".into(),
            description:
                "Return the title, URL, and visible text of the active browser tab \
                 in the current space."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "include_text": {
                        "type": "boolean",
                        "description": "Include the page's visible text body",
                        "default": true
                    },
                    "include_dom": {
                        "type": "boolean",
                        "description": "Include a compact DOM tree (expensive)",
                        "default": false
                    }
                }
            }),
        };
        self.register(def, false, move |args| {
            let provider = provider.clone();
            async move {
                #[derive(serde::Deserialize, Default)]
                struct Args {
                    #[serde(default = "default_true")]
                    include_text: bool,
                    #[serde(default)]
                    include_dom: bool,
                }
                fn default_true() -> bool { true }
                let a: Args = serde_json::from_str(&args).unwrap_or_default();
                let req = PageInspectRequest {
                    space_id,
                    include_text: a.include_text,
                    include_dom: a.include_dom,
                };
                match provider.inspect_page(req).await {
                    Ok(page) => ToolOutcome::Output(serde_json::to_value(page)
                        .unwrap_or_else(|_| Value::Null)),
                    Err(e) => ToolOutcome::Error(format!("inspect_page failed: {e}")),
                }
            }
        })
    }

    /// Register `read_file` and `write_file` tools backed by `provider`
    /// with path scope enforcement via `scope`.
    pub fn register_filesystem(
        &mut self,
        provider: Arc<dyn FilesystemCapability>,
        scope: WorkspaceScope,
    ) -> &mut Self {
        // read_file
        let read_provider = provider.clone();
        let read_scope = scope.clone();
        let read_def = ToolDef {
            name: "read_file".into(),
            description:
                "Read a file's content from the workspace. \
                 Path must be relative to the workspace root."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "offset": { "type": "integer" },
                    "max_bytes": { "type": "integer" }
                },
                "required": ["path"]
            }),
        };
        self.register(read_def, false, move |args| {
            let p = read_provider.clone();
            let s = read_scope.clone();
            async move {
                let req: crate::capability::filesystem::ReadFileRequest =
                    match serde_json::from_str(&args) {
                        Ok(r) => r,
                        Err(e) => return ToolOutcome::Error(format!("invalid read_file args: {e}")),
                    };
                let resolved = match s.resolve(&req.path) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("scope violation: {e}")),
                };
                match p.read_file(&resolved, req.offset, req.max_bytes).await {
                    Ok(r) => ToolOutcome::Output(serde_json::to_value(r)
                        .unwrap_or_else(|_| Value::Null)),
                    Err(e) => ToolOutcome::Error(format!("read_file failed: {e}")),
                }
            }
        });

        // write_file (requires approval)
        let write_provider = provider.clone();
        let write_scope = scope.clone();
        let write_def = ToolDef {
            name: "write_file".into(),
            description:
                "Write content to a workspace file. \
                 Path must be relative to the workspace root. Requires approval."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "create_dirs": { "type": "boolean", "default": true }
                },
                "required": ["path", "content"]
            }),
        };
        self.register(write_def, true, move |args| {
            let p = write_provider.clone();
            let s = write_scope.clone();
            async move {
                let req: crate::capability::filesystem::WriteFileRequest =
                    match serde_json::from_str(&args) {
                        Ok(r) => r,
                        Err(e) => return ToolOutcome::Error(format!("invalid write_file args: {e}")),
                    };
                let resolved = match s.resolve(&req.path) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("scope violation: {e}")),
                };
                match p.write_file(&resolved, &req.content, req.create_dirs).await {
                    Ok(()) => ToolOutcome::Output(serde_json::json!({ "written": true })),
                    Err(e) => ToolOutcome::Error(format!("write_file failed: {e}")),
                }
            }
        });

        // list_dir
        let ls_provider = provider.clone();
        let ls_scope = scope.clone();
        let ls_def = ToolDef {
            name: "list_dir".into(),
            description: "List directory contents within the workspace.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path relative to workspace root" }
                },
                "required": ["path"]
            }),
        };
        self.register(ls_def, false, move |args| {
            let p = ls_provider.clone();
            let s = ls_scope.clone();
            async move {
                #[derive(serde::Deserialize)]
                struct Args { path: String }
                let a: Args = match serde_json::from_str(&args) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("invalid list_dir args: {e}")),
                };
                let resolved = match s.resolve(&a.path) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("scope violation: {e}")),
                };
                match p.list_dir(&resolved).await {
                    Ok(entries) => ToolOutcome::Output(serde_json::json!({ "entries": entries })),
                    Err(e) => ToolOutcome::Error(format!("list_dir failed: {e}")),
                }
            }
        });

        self
    }

    /// Register a `notify` tool backed by `provider`.
    pub fn register_notify(
        &mut self,
        provider: Arc<dyn NotifyCapability>,
    ) -> &mut Self {
        let notify_provider = provider.clone();
        let notify_def = ToolDef {
            name: "notify".into(),
            description: "Post a macOS notification to the user.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string" },
                    "badge": { "type": "integer" }
                },
                "required": ["title", "body"]
            }),
        };
        self.register(notify_def, false, move |args| {
            let p = notify_provider.clone();
            async move {
                let req: NotifyRequest = match serde_json::from_str(&args) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("invalid notify args: {e}")),
                };
                match p.notify(req).await {
                    Ok(()) => ToolOutcome::Output(serde_json::json!({ "sent": true })),
                    Err(e) => ToolOutcome::Error(format!("notify failed: {e}")),
                }
            }
        });

        // request_approval (non-gated — handled inline)
        let approval_provider = provider.clone();
        let approval_def = ToolDef {
            name: "request_approval".into(),
            description:
                "Show the user a lightweight approval prompt. \
                 Returns 'approved', 'denied', or 'dismissed'."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "message": { "type": "string" }
                },
                "required": ["title", "message"]
            }),
        };
        self.register(approval_def, false, move |args| {
            let p = approval_provider.clone();
            async move {
                let req: ApprovalRequest = match serde_json::from_str(&args) {
                    Ok(r) => r,
                    Err(e) => return ToolOutcome::Error(format!("invalid request_approval args: {e}")),
                };
                match p.request_approval(req).await {
                    Ok(resp) => ToolOutcome::Output(serde_json::json!({
                        "response": match resp {
                            ApprovalResponse::Approved => "approved",
                            ApprovalResponse::Denied => "denied",
                            ApprovalResponse::Dismissed => "dismissed",
                        }
                    })),
                    Err(e) => ToolOutcome::Error(format!("request_approval failed: {e}")),
                }
            }
        });

        self
    }

    pub fn build(self) -> HostCapabilityDispatcher {
        HostCapabilityDispatcher { tools: self.tools }
    }
}

impl Default for DispatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::agent_loop::tools::ToolDispatcher;
    use crate::capability::shell::{ExitStatus, ShellCapability, ShellRequest, ShellResult};
    use crate::llm::ToolCall;

    // ── Mock shell provider ──────────────────────────────────────────────────

    #[derive(Debug)]
    struct OkShell;

    #[async_trait]
    impl ShellCapability for OkShell {
        async fn run(&self, req: ShellRequest) -> anyhow::Result<ShellResult> {
            Ok(ShellResult {
                exit_status: ExitStatus::Code(0),
                stdout: format!("ran: {}", req.command),
                stderr: String::new(),
                elapsed_ms: 5,
            })
        }
    }

    #[tokio::test]
    async fn shell_tool_registered_and_dispatch_works() {
        let mut builder = DispatcherBuilder::new();
        builder.register_shell(Arc::new(OkShell), false);
        let dispatcher = builder.build();

        // Tool should be advertised.
        let defs = dispatcher.definitions();
        assert!(defs.iter().any(|d| d.name == "run_shell"));

        // Dispatch should succeed.
        let call = ToolCall {
            id: "c1".into(),
            name: "run_shell".into(),
            arguments: r#"{"command":"echo hello"}"#.into(),
        };
        let outcome = dispatcher.dispatch(&call).await;
        match outcome {
            ToolOutcome::Output(v) => {
                assert_eq!(v["exit_code"], 0);
                assert!(v["stdout"].as_str().unwrap().contains("echo hello"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shell_with_approval_gate() {
        let mut builder = DispatcherBuilder::new();
        builder.register_shell(Arc::new(OkShell), true);
        let dispatcher = builder.build();

        let call = ToolCall {
            id: "c2".into(),
            name: "run_shell".into(),
            arguments: r#"{"command":"rm -rf /"}"#.into(),
        };
        // First dispatch should return NeedsApproval.
        assert!(matches!(
            dispatcher.dispatch(&call).await,
            ToolOutcome::NeedsApproval { .. }
        ));
        // After approval, dispatch_approved should run.
        assert!(matches!(
            dispatcher.dispatch_approved(&call).await,
            ToolOutcome::Output(_)
        ));
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let dispatcher = DispatcherBuilder::new().build();
        let call = ToolCall {
            id: "c3".into(),
            name: "no_such_tool".into(),
            arguments: "{}".into(),
        };
        assert!(matches!(
            dispatcher.dispatch(&call).await,
            ToolOutcome::Error(_)
        ));
    }
}
