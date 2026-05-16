//! Host capability adapters (task group 6).
//!
//! The runtime dispatches tool calls to these capability providers.
//! Traits and self-contained implementations both live here so the
//! runtime is fully self-hosted with no C++ delegation required:
//!
//! * [`shell`] — [`ShellCapability`] trait + [`LocalShell`] (tokio::process)
//!   + [`classify_command`] risk classifier.
//! * [`browser`] — page inspection wired to the active Space (task 6.2).
//! * [`filesystem`] — [`FilesystemCapability`] trait + [`LocalFilesystem`]
//!   (tokio::fs) + [`WorkspaceScope`] enforcement (task 6.3).
//! * [`notify`] — notifications, dock/status badges, and approval
//!   prompts (task 6.4).
//! * [`agent_loader`] — reads `<workspace>/.cronymax/agents/<id>.agent.yaml`
//!   so `spawn_agent_loop` can use the agent's declared system_prompt,
//!   LLM model, kind, and tools filter.
//! * [`dispatcher`] — [`HostCapabilityDispatcher`]: routes tool calls to
//!   registered capability providers.

pub mod agent_loader;
pub mod browser;
pub mod code_search;
pub mod dispatcher;
pub mod filesystem;
pub mod flow_tools;
pub mod git;
pub mod notify;
pub mod shell;
pub mod submit_document;
pub mod test_runner;

pub use agent_loader::{load_agent, AgentDef, AgentKind};
pub use browser::{BrowserCapability, PageContent, PageInspectRequest};
pub use dispatcher::HostCapabilityDispatcher;
pub use filesystem::{
    FilesystemCapability, LocalFilesystem, ReadFileRequest, ReadFileResult, WorkspaceScope,
    WriteFileRequest,
};
pub use flow_tools::{register_flow_tools, register_submit_review, SpawnAgentFn};
pub use notify::{ApprovalRequest, ApprovalResponse, NotifyCapability};
pub use shell::{
    classify_command, ExitStatus, LocalShell, RiskLevel, ShellCapability, ShellRequest, ShellResult,
};
pub use test_runner::{
    discover_tool_def, get_last_report_tool_def, run_suite_tool_def, DiscoveredSuite,
    LastReportStore, RunnerKind, TestFailure, TestRunnerResult,
};
