//! `commands.register` / `commands.execute` RPC handler state.
//!
//! Mirrors `cep-idl/v1/commands.ts`. The registry tracks "which extension
//! claims which command id" and refuses duplicates / `cronymax.*`.
//! Cross-process *dispatch* (sending an `execute` request to the owning
//! extension's RPC client) lands in `P2-T03` / `P4-T*` — this module
//! provides the lookup primitives the dispatcher needs.

use std::collections::HashMap;

use crate::extensions::error::{ExtensionError, ExtensionResult};

#[derive(Debug, Default)]
pub struct CommandRegistry {
    /// `command_id` → owning extension id. One command has exactly one
    /// owner; the `register` call rejects collisions.
    owners: HashMap<String, String>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim `command_id` for `ext_id`. Idempotent for the same owner;
    /// errors if a *different* extension already owns it, and rejects any
    /// `cronymax.*` namespace from third-party extensions.
    pub fn register(&mut self, ext_id: &str, command_id: &str) -> ExtensionResult<()> {
        if command_id.starts_with("cronymax.") || command_id == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "command id `{command_id}` is in the platform namespace"
            )));
        }
        match self.owners.get(command_id) {
            Some(existing) if existing == ext_id => Ok(()),
            Some(existing) => Err(ExtensionError::BadContribution {
                point: "cronymax.command".into(),
                ext_id: ext_id.to_string(),
                reason: format!("command `{command_id}` is already owned by `{existing}`"),
            }),
            None => {
                self.owners
                    .insert(command_id.to_string(), ext_id.to_string());
                Ok(())
            }
        }
    }

    /// Release a single command. No-op if `command_id` is unknown; errors
    /// if `ext_id` doesn't own it.
    pub fn unregister(&mut self, ext_id: &str, command_id: &str) -> ExtensionResult<()> {
        match self.owners.get(command_id) {
            Some(owner) if owner == ext_id => {
                self.owners.remove(command_id);
                Ok(())
            }
            Some(other) => Err(ExtensionError::BadContribution {
                point: "cronymax.command".into(),
                ext_id: ext_id.to_string(),
                reason: format!("command `{command_id}` is owned by `{other}`, not `{ext_id}`"),
            }),
            None => Ok(()),
        }
    }

    /// Release every command owned by `ext_id`. Used on deactivate.
    pub fn unregister_all_for(&mut self, ext_id: &str) {
        self.owners.retain(|_, owner| owner != ext_id);
    }

    pub fn owner_of(&self, command_id: &str) -> Option<&str> {
        self.owners.get(command_id).map(|s| s.as_str())
    }

    pub fn commands_for(&self, ext_id: &str) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .owners
            .iter()
            .filter(|(_, o)| *o == ext_id)
            .map(|(k, _)| k.as_str())
            .collect();
        v.sort_unstable();
        v
    }

    pub fn all(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<(&str, &str)> = self
            .owners
            .iter()
            .map(|(k, o)| (k.as_str(), o.as_str()))
            .collect();
        v.sort_unstable();
        v
    }

    pub fn len(&self) -> usize {
        self.owners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_lookup() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.x.hello").unwrap();
        assert_eq!(r.owner_of("alice.x.hello"), Some("alice.x"));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn register_rejects_cronymax_namespace() {
        let mut r = CommandRegistry::new();
        let err = r.register("alice.x", "cronymax.builtin").unwrap_err();
        assert!(
            matches!(err, ExtensionError::NamespaceReserved(_)),
            "got {err:?}"
        );
        // bare `cronymax` also rejected
        let err = r.register("alice.x", "cronymax").unwrap_err();
        assert!(
            matches!(err, ExtensionError::NamespaceReserved(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn register_same_owner_is_idempotent() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.x.hi").unwrap();
        r.register("alice.x", "alice.x.hi").unwrap();
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn register_collision_with_other_extension_errors() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "shared.cmd").unwrap();
        let err = r.register("bob.y", "shared.cmd").unwrap_err();
        assert!(
            matches!(err, ExtensionError::BadContribution { .. }),
            "got {err:?}"
        );
        // ownership unchanged
        assert_eq!(r.owner_of("shared.cmd"), Some("alice.x"));
    }

    #[test]
    fn unregister_releases_owned_command() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.x.hi").unwrap();
        r.unregister("alice.x", "alice.x.hi").unwrap();
        assert!(r.owner_of("alice.x.hi").is_none());
    }

    #[test]
    fn unregister_rejects_other_extension() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.x.hi").unwrap();
        let err = r.unregister("bob.y", "alice.x.hi").unwrap_err();
        assert!(
            matches!(err, ExtensionError::BadContribution { .. }),
            "got {err:?}"
        );
        // ownership unchanged
        assert_eq!(r.owner_of("alice.x.hi"), Some("alice.x"));
    }

    #[test]
    fn unregister_unknown_command_is_noop() {
        let mut r = CommandRegistry::new();
        // No command registered at all — unregister succeeds silently.
        r.unregister("alice.x", "no.such").unwrap();
    }

    #[test]
    fn unregister_all_for_drops_only_that_extension() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.a").unwrap();
        r.register("alice.x", "alice.b").unwrap();
        r.register("bob.y", "bob.a").unwrap();

        r.unregister_all_for("alice.x");
        assert!(r.owner_of("alice.a").is_none());
        assert!(r.owner_of("alice.b").is_none());
        assert_eq!(r.owner_of("bob.a"), Some("bob.y"));
    }

    #[test]
    fn commands_for_returns_only_that_extensions_commands_sorted() {
        let mut r = CommandRegistry::new();
        r.register("alice.x", "alice.bbb").unwrap();
        r.register("alice.x", "alice.aaa").unwrap();
        r.register("alice.x", "alice.ccc").unwrap();
        r.register("bob.y", "bob.bbb").unwrap();
        assert_eq!(
            r.commands_for("alice.x"),
            vec!["alice.aaa", "alice.bbb", "alice.ccc"]
        );
        assert_eq!(r.commands_for("bob.y"), vec!["bob.bbb"]);
        assert!(r.commands_for("unknown").is_empty());
    }

    #[test]
    fn all_listing_is_stable_sorted() {
        let mut r = CommandRegistry::new();
        r.register("bob.y", "bob.a").unwrap();
        r.register("alice.x", "alice.a").unwrap();
        assert_eq!(r.all(), vec![("alice.a", "alice.x"), ("bob.a", "bob.y")],);
    }
}
