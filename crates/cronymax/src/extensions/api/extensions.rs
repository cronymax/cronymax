//! `extensions.getExtension` / `extensions.all` API.
//!
//! VS Code-style cross-extension hand-off via `Extension.exports` — no
//! schema, no semver, the contract is between the two extensions only.
//! Mirrors `cep-idl/v1/extensions.ts`.
//!
//! This module holds the **exports map** (which extension published what
//! shape) and provides a view-builder that joins it against the on-disk
//! registry + the runtime activation state.

use std::collections::HashMap;
use std::sync::RwLock;

use serde_json::Value;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// A snapshot of one extension as seen from another extension. Built by
/// joining [`ExportsRegistry`] against the on-disk
/// [`crate::extensions::registry::ExtensionRegistry`] and the runtime
/// [`super::lifecycle::LifecycleState`].
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionView {
    pub id: String,
    /// `true` ↔ Node host has reported a successful `activate()`.
    pub is_active: bool,
    /// What `activate()` returned. `Value::Null` until activation completes.
    pub exports: Value,
    /// Cached version string from the manifest, for the view of consuming
    /// extensions that want to compatibility-check before reading exports.
    pub version: String,
}

/// In-memory map of `ext_id → exports`. `set_exports` is called by the host
/// module when an extension's `activate()` returns; `clear_exports` is
/// called on deactivate.
#[derive(Debug, Default)]
pub struct ExportsRegistry {
    inner: RwLock<HashMap<String, Value>>,
}

impl ExportsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record what `ext_id`'s `activate()` returned.
    pub fn set_exports(&self, ext_id: &str, exports: Value) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("exports registry poisoned".into()))?;
        g.insert(ext_id.to_string(), exports);
        Ok(())
    }

    /// Drop the recorded exports for `ext_id` (e.g. on deactivate / disable).
    pub fn clear_exports(&self, ext_id: &str) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("exports registry poisoned".into()))?;
        g.remove(ext_id);
        Ok(())
    }

    /// Read the recorded exports for `ext_id`, or `Value::Null` if the
    /// extension has not yet activated.
    pub fn exports_for(&self, ext_id: &str) -> ExtensionResult<Value> {
        let g = self
            .inner
            .read()
            .map_err(|_| ExtensionError::ManifestInvalid("exports registry poisoned".into()))?;
        Ok(g.get(ext_id).cloned().unwrap_or(Value::Null))
    }
}

/// Build an [`ExtensionView`] from the three sources of truth. The host
/// module owns the registry / lifecycle / exports state and calls this
/// when an extension RPCs `extensions.getExtension`.
pub fn view_of(
    ext_id: &str,
    on_disk_version: Option<&str>,
    is_activated: bool,
    exports: &ExportsRegistry,
) -> ExtensionResult<Option<ExtensionView>> {
    let Some(version) = on_disk_version else {
        return Ok(None);
    };
    Ok(Some(ExtensionView {
        id: ext_id.to_string(),
        is_active: is_activated,
        exports: exports.exports_for(ext_id)?,
        version: version.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fresh_registry_returns_null_for_unknown_id() {
        let r = ExportsRegistry::new();
        assert_eq!(r.exports_for("alice.x").unwrap(), Value::Null);
    }

    #[test]
    fn set_get_round_trip() {
        let r = ExportsRegistry::new();
        r.set_exports("alice.x", json!({"api": "v1"})).unwrap();
        assert_eq!(r.exports_for("alice.x").unwrap(), json!({"api": "v1"}));
    }

    #[test]
    fn set_overwrites_previous() {
        let r = ExportsRegistry::new();
        r.set_exports("alice.x", json!(1)).unwrap();
        r.set_exports("alice.x", json!(2)).unwrap();
        assert_eq!(r.exports_for("alice.x").unwrap(), json!(2));
    }

    #[test]
    fn clear_drops_exports() {
        let r = ExportsRegistry::new();
        r.set_exports("alice.x", json!(42)).unwrap();
        r.clear_exports("alice.x").unwrap();
        assert_eq!(r.exports_for("alice.x").unwrap(), Value::Null);
    }

    #[test]
    fn clear_unknown_is_a_noop() {
        let r = ExportsRegistry::new();
        r.clear_exports("nope.ext").unwrap();
    }

    #[test]
    fn view_of_returns_none_when_not_installed() {
        let r = ExportsRegistry::new();
        let v = view_of("ghost.ext", None, false, &r).unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn view_of_joins_state_when_installed_and_active() {
        let r = ExportsRegistry::new();
        r.set_exports("alice.x", json!({"hello": true})).unwrap();
        let v = view_of("alice.x", Some("1.2.3"), true, &r)
            .unwrap()
            .unwrap();
        assert_eq!(v.id, "alice.x");
        assert_eq!(v.version, "1.2.3");
        assert!(v.is_active);
        assert_eq!(v.exports, json!({"hello": true}));
    }

    #[test]
    fn view_of_installed_but_not_active_has_null_exports() {
        let r = ExportsRegistry::new();
        // No set_exports — the extension hasn't activated yet.
        let v = view_of("alice.x", Some("0.1.0"), false, &r)
            .unwrap()
            .unwrap();
        assert!(!v.is_active);
        assert_eq!(v.exports, Value::Null);
    }

    #[test]
    fn concurrent_set_and_read_dont_deadlock() {
        use std::sync::Arc;
        use std::thread;
        let r = Arc::new(ExportsRegistry::new());
        let mut handles = Vec::new();
        for i in 0..16 {
            let r = r.clone();
            handles.push(thread::spawn(move || {
                if i % 2 == 0 {
                    r.set_exports(&format!("e.{i}"), json!(i)).unwrap();
                } else {
                    let _ = r.exports_for(&format!("e.{}", i - 1)).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }
}
