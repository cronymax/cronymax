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
//! * [`dispatcher`] — [`HostCapabilityDispatcher`]: routes tool calls to
//!   registered capability providers.

pub mod browser;
pub mod dispatcher;
pub mod filesystem;
pub mod notify;
pub mod shell;

pub use browser::{BrowserCapability, PageContent, PageInspectRequest};
pub use dispatcher::HostCapabilityDispatcher;
pub use filesystem::{
    FilesystemCapability, LocalFilesystem, ReadFileRequest, ReadFileResult,
    WorkspaceScope, WriteFileRequest,
};
pub use notify::{ApprovalRequest, ApprovalResponse, NotifyCapability};
pub use shell::{classify_command, ExitStatus, LocalShell, RiskLevel, ShellCapability,
    ShellRequest, ShellResult};
