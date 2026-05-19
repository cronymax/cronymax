//! Rust implementations of the IDL-v1 API surface — the methods extensions
//! reach by calling `cronymax.*` in their `main.ts`.
//!
//! Each submodule pairs with a `.ts` IDL file under
//! `crates/cronymax/src/extensions/cep-idl/v1/`. RPC dispatch in
//! [`super::rpc::server`] routes incoming method names here.
//!
//! Per spec §6: there are **no** Rust-side `fs.rs`, `process.rs`, or
//! `network.rs` modules — those calls go straight to Node's standard library
//! and are enforced by `--allow-*` flags. The platform never wraps them.

pub mod agents;
pub mod auth;
pub mod commands;
pub mod config;
pub mod events;
pub mod extensions;
pub mod lifecycle;
pub mod secrets;
pub mod storage;
pub mod webview;
pub mod window;
pub mod workspace;
