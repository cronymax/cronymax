//! Protocol version constant used for handshake / mismatch detection.
//!
//! The full GIPS message schemas land in tasks 2.x. Only the version
//! type is defined here so `crony` and `cronymax` can already negotiate
//! at startup.

use serde::{Deserialize, Serialize};

/// Semantic version triple for the runtime <-> host protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }

    /// Same-major versions are considered compatible. Pre-1.0 the major
    /// is `0` and any minor bump is treated as breaking; this matches
    /// the cargo / semver convention.
    pub fn is_compatible_with(self, other: ProtocolVersion) -> bool {
        if self.major != other.major {
            return false;
        }
        if self.major == 0 {
            return self.minor == other.minor;
        }
        true
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Current protocol version exported by this build of the runtime.
///
/// Pre-1.0: any minor bump is breaking. Bump `minor` on every wire
/// change until the surface stabilises.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(0, 1, 0);
