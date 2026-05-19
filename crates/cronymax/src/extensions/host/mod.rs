//! Node 26 subprocess management for each extension.
//!
//! Phase 2 implements [`node`]. v1 ships one Node host process per extension
//! (lazy activation, no proactive deactivate). The Node host is given the
//! `--allow-*` flags computed by [`super::capability::build_node_flags`] and
//! a Unix socket / Named Pipe address; everything else flows over
//! MessagePack-RPC.

pub mod node;
