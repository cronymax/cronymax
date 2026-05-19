//! Shared error type for the extension platform.

use std::path::PathBuf;

use thiserror::Error;

/// All errors raised by `crates/cronymax/src/extensions/`.
///
/// Variants are added (never repurposed) as new phases land.
#[derive(Debug, Error)]
pub enum ExtensionError {
    // ── manifest / registry ────────────────────────────────────────────────
    #[error("manifest parse failed: {0}")]
    ManifestParse(String),

    #[error("manifest validation failed: {0}")]
    ManifestInvalid(String),

    #[error("namespace `{0}` is reserved by the platform")]
    NamespaceReserved(String),

    #[error("publisher prefix mismatch: id `{id}` does not start with `{publisher}.`")]
    PublisherPrefixMismatch { id: String, publisher: String },

    #[error("extension `{0}` is not installed")]
    NotInstalled(String),

    #[error("extension `{0}` is already installed (version `{1}`)")]
    AlreadyInstalled(String, String),

    #[error("extension `{0}` is not enabled")]
    NotEnabled(String),

    // ── activation / host ──────────────────────────────────────────────────
    #[error("activation event `{0}` is not understood")]
    UnknownActivationEvent(String),

    #[error("node host spawn failed: {0}")]
    HostSpawn(String),

    #[error("node host `{ext_id}` crashed (exit `{exit}`); restarted {restarts} times")]
    HostCrashed {
        ext_id: String,
        exit: i32,
        restarts: u32,
    },

    #[error("node host `{0}` did not complete handshake in time")]
    HostHandshakeTimeout(String),

    // ── rpc ────────────────────────────────────────────────────────────────
    #[error("rpc: {0}")]
    Rpc(String),

    #[error("rpc method `{0}` not implemented")]
    RpcMethodMissing(String),

    #[error("rpc cancelled")]
    RpcCancelled,

    // ── capability / permission ────────────────────────────────────────────
    #[error("capability `{0}` is not granted to this extension")]
    CapabilityDenied(String),

    #[error("path canonicalize failed: {path:?}: {source}")]
    PathCanonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ── contributions ──────────────────────────────────────────────────────
    #[error("contribution point `{0}` is unknown")]
    UnknownContributionPoint(String),

    #[error("contribution `{point}` from `{ext_id}` is invalid: {reason}")]
    BadContribution {
        point: String,
        ext_id: String,
        reason: String,
    },

    // ── i/o and serde ──────────────────────────────────────────────────────
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(String),
}

impl From<serde_json::Error> for ExtensionError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err.to_string())
    }
}

pub type ExtensionResult<T> = Result<T, ExtensionError>;
