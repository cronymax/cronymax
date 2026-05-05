//! Host capability adapters (task group 6).
//!
//! The runtime can dispatch tool calls to platform capabilities owned
//! by the host process. This module defines the provider-facing
//! abstractions so the agent loop stays host-agnostic:
//!
//! * [`shell`] — sandboxed shell / PTY execution (task 6.1).
//! * [`browser`] — page inspection, wired to the active Space (task 6.2).
//! * [`filesystem`] — workspace-scoped file mediation and secret access
//!   (task 6.3).
//! * [`notify`] — notifications, dock/status badges, and approval
//!   prompts (task 6.4).
//! * [`dispatcher`] — [`HostCapabilityDispatcher`], which implements the
//!   [`crate::agent_loop::ToolDispatcher`] trait by routing each tool
//!   call to the registered capability provider.
//!
//! Concrete implementations live in `crony/` (or any future host crate)
//! so that `crates/cronymax` stays C-FFI-less.

pub mod browser;
pub mod dispatcher;
pub mod filesystem;
pub mod notify;
pub mod shell;

pub use browser::{BrowserCapability, PageContent, PageInspectRequest};
pub use dispatcher::HostCapabilityDispatcher;
pub use filesystem::{FilesystemCapability, ReadFileRequest, ReadFileResult, WorkspaceScope, WriteFileRequest};
pub use notify::{ApprovalRequest, ApprovalResponse, NotifyCapability};
pub use shell::{ExitStatus, ShellCapability, ShellRequest, ShellResult};
