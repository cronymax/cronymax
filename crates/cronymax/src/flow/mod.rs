//! Flow orchestration — self-contained in the Rust runtime.
//!
//! Mirrors the `app/flow/` subsystem. All components are C-FFI-free.
//!
//! ## Modules
//!
//! * [`workspace_layout`] — canonical `.cronymax/` directory paths.
//! * [`mention_parser`] — parse `@agent` mentions from document bodies.
//! * [`trace`] — structured trace events and async append-only writer.
//! * [`definition`] — `FlowDefinition` / `FlowNode` / `FlowGraph` parsed
//!   from `flow.yaml` (node-centric model).
//! * [`registry`] — `FlowRegistry`: scan and reload flow definitions.
//! * [`runtime`] — `FlowRuntime`: run state machine, persistence, event
//!   emission.
//! * [`watcher`] — `FsWatcher`: debounced filesystem change notifications.
//! * [`gitignore`] — advisory `.gitignore` suggestions for run artifacts.

pub mod definition;
pub mod gitignore;
pub mod mention_parser;
pub mod registry;
pub mod runtime;
pub mod trace;
pub mod watcher;
pub mod workspace_layout;

pub use definition::{FlowDefinition, FlowGraph, FlowLoadError, FlowNode, FlowNodeOutput};
pub use gitignore::GitignoreHelper;
pub use mention_parser::{parse_mentions, ParsedMention};
pub use registry::FlowRegistry;
pub use runtime::{
    AvailableDoc, EventEmitter, FlowRunDocumentEntry, FlowRunState, FlowRunStatus,
    FlowRuntime, InvocationContext, InvocationTrigger, NodeRunState, PortStatus,
};
pub use trace::{TraceEvent, TraceKind, TraceWriter};
pub use watcher::FsWatcher;
pub use workspace_layout::WorkspaceLayout;
