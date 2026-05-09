//! Workspace-layout utilities and file I/O.
//!
//! This module is the Rust counterpart of `app/workspace/` and
//! `app/document/` (workspace-side parts). C++ equivalents are deleted
//! as part of the Phase 2 migration.

pub mod agent_registry;
pub mod doc_type_registry;
pub mod file_broker;
pub mod flow_yaml;
pub mod layout;

pub use agent_registry::{AgentDef, AgentRegistry};
pub use doc_type_registry::{DocTypeRegistry, DocTypeSchema};
pub use file_broker::FileBroker;
pub use flow_yaml::{
    load_flow_agents, load_flow_yaml, parse_flow_yaml,
    FlowYamlDoc, FlowYamlEdge, FlowYamlNode, FlowYamlNodeOutput,
};
pub use layout::WorkspaceLayout;
