//! Flow orchestration — self-contained in the Rust runtime.
//!
//! Mirrors the `app/flow/` subsystem. All components are C-FFI-free.
//!
//! ## Modules
//!
//! * [`workspace_layout`] — canonical `.cronymax/` directory paths.
//! * [`mention_parser`] — parse `@agent` mentions from document bodies.
//! * [`trace`] — structured trace events and async append-only writer.
//! * [`definition`] — `FlowDefinition` / `FlowEdge` parsed from `flow.yaml`.
//! * [`registry`] — `FlowRegistry`: scan and reload flow definitions.
//! * [`router`] — route document submissions to next agents (typed-port +
//!   @mention strategies).
//! * [`runtime`] — `FlowRuntime`: run state machine, persistence, event
//!   emission.
//! * [`watcher`] — `FsWatcher`: debounced filesystem change notifications.
//! * [`gitignore`] — advisory `.gitignore` suggestions for run artifacts.

pub mod definition;
pub mod gitignore;
pub mod mention_parser;
pub mod registry;
pub mod router;
pub mod runtime;
pub mod trace;
pub mod watcher;
pub mod workspace_layout;

pub use definition::{FlowDefinition, FlowEdge, FlowLoadError};
pub use gitignore::GitignoreHelper;
pub use mention_parser::{parse_mentions, ParsedMention};
pub use registry::FlowRegistry;
pub use router::{RouteDecision, RouteReason, RouteTarget, Router};
pub use runtime::{
    EventEmitter, FlowRunDocumentEntry, FlowRunState, FlowRunStatus, FlowRuntime,
};
pub use trace::{TraceEvent, TraceKind, TraceWriter};
pub use watcher::FsWatcher;
pub use workspace_layout::WorkspaceLayout;
