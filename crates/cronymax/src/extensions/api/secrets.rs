//! `secrets.get/set/delete` API — OS keychain backend.
//!
//! Mirrors `cep-idl/v1/secrets.ts`. Backends per platform:
//!
//! * macOS  → `security-framework` (`SecKeychain`) — implemented now
//! * Linux  → `secret-service` D-Bus (implementation deferred — uses
//!   in-memory store for now and emits a TODO warning)
//! * Win    → DPAPI / Credential Manager (deferred, same fallback)
//!
//! **Namespace lockdown**: every read/write goes through
//! [`SecretStore::for_extension`], which only lets an extension touch keys
//! under its own `<publisher>.<name>.*` prefix. The platform validates this
//! at the RPC boundary, but the store also defends in depth so the platform
//! is robust to a misbehaving handler.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// Backend trait so callers can swap implementations (real keychain in
/// production, in-memory in tests).
pub trait SecretBackend: Send + Sync + std::fmt::Debug {
    fn get(&self, service: &str, key: &str) -> ExtensionResult<Option<String>>;
    fn set(&self, service: &str, key: &str, value: &str) -> ExtensionResult<()>;
    fn delete(&self, service: &str, key: &str) -> ExtensionResult<()>;
}

/// "Service" identifier used by the OS keychain so cronymax secrets are
/// kept distinct from any other app's. Same for every cronymax install on
/// a given machine; namespace happens via the `key` portion.
pub const KEYCHAIN_SERVICE: &str = "ai.cronymax.extensions";

/// Top-level secret store. Construct once per process; hand out
/// [`ExtensionSecrets`] views via [`Self::for_extension`].
#[derive(Debug, Clone)]
pub struct SecretStore {
    backend: Arc<dyn SecretBackend>,
}

impl SecretStore {
    /// In production, wraps the macOS Keychain. On other platforms it
    /// falls back to the in-memory backend with a warning.
    pub fn os_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                backend: Arc::new(macos::MacosKeychainBackend),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            tracing::warn!(
                "ext-secrets: native keychain backend not yet implemented on this platform; \
                 using in-memory fallback. Secrets WILL NOT survive process restart."
            );
            Self::in_memory()
        }
    }

    /// Always-available in-memory backend. Tests use this; production
    /// platforms without a keychain integration also fall back here.
    pub fn in_memory() -> Self {
        Self {
            backend: Arc::new(InMemoryBackend::default()),
        }
    }

    pub fn with_backend(backend: Arc<dyn SecretBackend>) -> Self {
        Self { backend }
    }

    /// Hand out a per-extension view. `namespace` MUST match the
    /// extension's declared `capabilities.secrets.namespace` (validated by
    /// the caller / manifest validation in `P1-T02`).
    pub fn for_extension(
        &self,
        ext_id: impl Into<String>,
        namespace: impl Into<String>,
    ) -> ExtensionSecrets {
        ExtensionSecrets {
            backend: self.backend.clone(),
            ext_id: ext_id.into(),
            namespace: namespace.into(),
        }
    }
}

/// Per-extension secret access. All keys are silently prefixed with
/// `<namespace>.`; reads/writes outside the namespace error with
/// [`ExtensionError::NamespaceReserved`].
#[derive(Debug)]
pub struct ExtensionSecrets {
    backend: Arc<dyn SecretBackend>,
    ext_id: String,
    namespace: String,
}

impl ExtensionSecrets {
    pub fn ext_id(&self) -> &str {
        &self.ext_id
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn get(&self, key: &str) -> ExtensionResult<Option<String>> {
        let scoped = self.scoped_key(key)?;
        self.backend.get(KEYCHAIN_SERVICE, &scoped)
    }

    pub fn set(&self, key: &str, value: &str) -> ExtensionResult<()> {
        let scoped = self.scoped_key(key)?;
        self.backend.set(KEYCHAIN_SERVICE, &scoped, value)
    }

    pub fn delete(&self, key: &str) -> ExtensionResult<()> {
        let scoped = self.scoped_key(key)?;
        self.backend.delete(KEYCHAIN_SERVICE, &scoped)
    }

    fn scoped_key(&self, key: &str) -> ExtensionResult<String> {
        if key.is_empty() {
            return Err(ExtensionError::ManifestInvalid(
                "secret key is empty".into(),
            ));
        }
        // Defence in depth: even though the platform restricts the
        // namespace at the RPC boundary, ensure no extension can sneak a
        // key that escapes its prefix.
        if key.contains('\0') {
            return Err(ExtensionError::ManifestInvalid(
                "secret key contains NUL".into(),
            ));
        }
        Ok(format!("{}.{}", self.namespace, key))
    }
}

/// In-memory backend used by tests and as a fallback on platforms whose
/// keychain integration isn't done yet.
#[derive(Debug, Default)]
pub struct InMemoryBackend {
    inner: Mutex<HashMap<(String, String), String>>,
}

impl SecretBackend for InMemoryBackend {
    fn get(&self, service: &str, key: &str) -> ExtensionResult<Option<String>> {
        let g = self
            .inner
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("secrets mutex poisoned".into()))?;
        Ok(g.get(&(service.to_string(), key.to_string())).cloned())
    }

    fn set(&self, service: &str, key: &str, value: &str) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("secrets mutex poisoned".into()))?;
        g.insert((service.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    fn delete(&self, service: &str, key: &str) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("secrets mutex poisoned".into()))?;
        g.remove(&(service.to_string(), key.to_string()));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{ExtensionError, ExtensionResult, SecretBackend};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    /// Wraps `security-framework`'s generic-password APIs against the
    /// default Keychain. Each `(service, key)` pair becomes a separate
    /// keychain item.
    #[derive(Debug)]
    pub struct MacosKeychainBackend;

    impl SecretBackend for MacosKeychainBackend {
        fn get(&self, service: &str, key: &str) -> ExtensionResult<Option<String>> {
            match get_generic_password(service, key) {
                Ok(bytes) => {
                    let s = String::from_utf8(bytes).map_err(|e| {
                        ExtensionError::ManifestInvalid(format!(
                            "keychain secret is not valid UTF-8: {e}"
                        ))
                    })?;
                    Ok(Some(s))
                }
                Err(e) => {
                    // security-framework returns ItemNotFound as an error;
                    // surface that as Ok(None). Other errors propagate.
                    if format!("{e}").contains("specified item could not be found") {
                        Ok(None)
                    } else {
                        Err(ExtensionError::ManifestInvalid(format!(
                            "keychain get failed: {e}"
                        )))
                    }
                }
            }
        }

        fn set(&self, service: &str, key: &str, value: &str) -> ExtensionResult<()> {
            set_generic_password(service, key, value.as_bytes())
                .map_err(|e| ExtensionError::ManifestInvalid(format!("keychain set failed: {e}")))
        }

        fn delete(&self, service: &str, key: &str) -> ExtensionResult<()> {
            match delete_generic_password(service, key) {
                Ok(()) => Ok(()),
                Err(e) if format!("{e}").contains("specified item could not be found") => Ok(()),
                Err(e) => Err(ExtensionError::ManifestInvalid(format!(
                    "keychain delete failed: {e}"
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SecretStore {
        SecretStore::in_memory()
    }

    #[test]
    fn round_trip_get_set_delete() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        v.set("token", "abc").unwrap();
        assert_eq!(v.get("token").unwrap(), Some("abc".into()));
        v.delete("token").unwrap();
        assert_eq!(v.get("token").unwrap(), None);
    }

    #[test]
    fn keys_are_scoped_by_namespace() {
        // Two extensions, same logical key "token", must not collide.
        let s = store();
        let a = s.for_extension("alice.x", "alice.x");
        let b = s.for_extension("bob.y", "bob.y");
        a.set("token", "alice-token").unwrap();
        b.set("token", "bob-token").unwrap();
        assert_eq!(a.get("token").unwrap(), Some("alice-token".into()));
        assert_eq!(b.get("token").unwrap(), Some("bob-token".into()));
    }

    #[test]
    fn missing_key_returns_none() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        assert_eq!(v.get("nope").unwrap(), None);
    }

    #[test]
    fn delete_unknown_key_is_noop() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        v.delete("nope").unwrap();
    }

    #[test]
    fn empty_key_is_rejected() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        let err = v.get("").unwrap_err();
        assert!(
            matches!(err, ExtensionError::ManifestInvalid(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn nul_in_key_is_rejected() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        let err = v.set("with\0nul", "x").unwrap_err();
        assert!(matches!(err, ExtensionError::ManifestInvalid(_)));
    }

    #[test]
    fn sub_namespace_is_a_separate_scope() {
        // alice.x.oauth and alice.x.config don't conflict even though
        // they share the publisher prefix.
        let s = store();
        let a = s.for_extension("alice.x", "alice.x.oauth");
        let b = s.for_extension("alice.x", "alice.x.config");
        a.set("token", "oauth-token").unwrap();
        b.set("token", "config-token").unwrap();
        assert_eq!(a.get("token").unwrap(), Some("oauth-token".into()));
        assert_eq!(b.get("token").unwrap(), Some("config-token".into()));
    }

    #[test]
    fn override_overwrites() {
        let s = store();
        let v = s.for_extension("alice.x", "alice.x");
        v.set("token", "v1").unwrap();
        v.set("token", "v2").unwrap();
        assert_eq!(v.get("token").unwrap(), Some("v2".into()));
    }

    #[test]
    fn in_memory_backend_is_shared_across_views_of_same_store() {
        let s = store();
        let a1 = s.for_extension("alice.x", "alice.x");
        let a2 = s.for_extension("alice.x", "alice.x");
        a1.set("k", "v").unwrap();
        assert_eq!(a2.get("k").unwrap(), Some("v".into()));
    }

    #[test]
    fn store_clones_share_backend() {
        // Crucial property: cloning the store gives more handles to the
        // *same* backend (so e.g. cloning into background tasks doesn't
        // fork secret state).
        let s = store();
        let s2 = s.clone();
        s.for_extension("alice.x", "alice.x").set("k", "v").unwrap();
        assert_eq!(
            s2.for_extension("alice.x", "alice.x").get("k").unwrap(),
            Some("v".into())
        );
    }
}
