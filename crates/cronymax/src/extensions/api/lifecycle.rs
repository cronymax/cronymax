//! `extension/activate` and `extension/deactivate` RPC handlers.
//!
//! Tracks **runtime** activation state — whether the Node host has reported
//! a successful `activate()` for an extension. This is per-runtime; the
//! `enabled` flag in [`super::super::registry::ExtensionRegistry`] is the
//! on-disk "should this auto-activate?" bit. Two different concepts:
//!
//! | `enabled` (registry) | `activated` (here) | What it means              |
//! |---|---|---|
//! | true  | false | Eligible but not yet activated (lazy activation event hasn't fired) |
//! | true  | true  | Node host running, `activate()` returned OK                       |
//! | false | false | User disabled it; nothing happens                                  |
//! | false | true  | Impossible in practice (we deactivate on disable)                  |

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One activated extension's bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivationInfo {
    pub ext_id: String,
    /// Unix epoch seconds when `activate()` returned successfully.
    pub activated_at: u64,
}

/// Runtime activation tracker. Cheap to share via `Arc<Mutex<_>>`.
#[derive(Debug, Default)]
pub struct LifecycleState {
    activated: HashMap<String, ActivationInfo>,
}

impl LifecycleState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `ext_id` has successfully activated. Returns
    /// `AlreadyActivated` if it was already in the map.
    pub fn mark_activated(&mut self, ext_id: &str) -> ExtensionResult<()> {
        if self.activated.contains_key(ext_id) {
            return Err(ExtensionError::AlreadyActivated(ext_id.to_string()));
        }
        self.activated.insert(
            ext_id.to_string(),
            ActivationInfo {
                ext_id: ext_id.to_string(),
                activated_at: now_seconds(),
            },
        );
        Ok(())
    }

    /// Drop the activation record. Returns `NotActivated` if the extension
    /// was not actually activated (a no-op is plausibly desirable but we
    /// surface it so callers can audit double-deactivate bugs).
    pub fn mark_deactivated(&mut self, ext_id: &str) -> ExtensionResult<()> {
        self.activated
            .remove(ext_id)
            .ok_or_else(|| ExtensionError::NotActivated(ext_id.to_string()))?;
        Ok(())
    }

    pub fn is_activated(&self, ext_id: &str) -> bool {
        self.activated.contains_key(ext_id)
    }

    pub fn info(&self, ext_id: &str) -> Option<&ActivationInfo> {
        self.activated.get(ext_id)
    }

    pub fn activated_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.activated.keys().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        ids
    }

    pub fn len(&self) -> usize {
        self.activated.len()
    }

    pub fn is_empty(&self) -> bool {
        self.activated.is_empty()
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_empty() {
        let s = LifecycleState::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert!(!s.is_activated("alice.x"));
    }

    #[test]
    fn activate_then_deactivate_round_trip() {
        let mut s = LifecycleState::new();
        s.mark_activated("alice.x").unwrap();
        assert!(s.is_activated("alice.x"));
        assert_eq!(s.len(), 1);
        assert!(s.info("alice.x").unwrap().activated_at > 0);
        s.mark_deactivated("alice.x").unwrap();
        assert!(!s.is_activated("alice.x"));
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn double_activate_errors() {
        let mut s = LifecycleState::new();
        s.mark_activated("alice.x").unwrap();
        let err = s.mark_activated("alice.x").unwrap_err();
        assert!(
            matches!(err, ExtensionError::AlreadyActivated(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn deactivate_unknown_errors() {
        let mut s = LifecycleState::new();
        let err = s.mark_deactivated("alice.x").unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotActivated(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn activated_ids_is_sorted_and_deduped() {
        let mut s = LifecycleState::new();
        s.mark_activated("charlie.z").unwrap();
        s.mark_activated("alice.a").unwrap();
        s.mark_activated("bob.b").unwrap();
        assert_eq!(s.activated_ids(), vec!["alice.a", "bob.b", "charlie.z"]);
    }
}
