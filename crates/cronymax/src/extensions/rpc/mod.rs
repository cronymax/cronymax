//! MessagePack-RPC server for one extension host.
//!
//! Phase 2 implements [`codec`] and [`server`] (`P2-T04`). Wire format:
//! MessagePack-RPC 0/1/2 (request / response / notify) with explicit
//! cancellation token piggy-backed on requests.

pub mod codec;
pub mod server;
