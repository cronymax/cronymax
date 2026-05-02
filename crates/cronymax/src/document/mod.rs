//! Document store — Rust port of `app/document/document_store.cc`.
//!
//! The C++ `DocumentStore` is fully replaced by this module. All
//! `document.*` bridge channels and the `document` capability handler
//! are now handled here via `ControlRequest` variants.

pub mod store;

pub use store::{DocInfo, DocumentStore, WriteResult};
