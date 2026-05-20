//! L1.5 platform-event bus with cross-process subscriber callbacks.
//!
//! Two emit paths:
//!
//! * **Platform → bus**: [`EventBus::emit_from_platform`] fans out a
//!   `cronymax.*` topic to every extension whose
//!   `capabilities.events.subscribe` declared that topic.
//! * **Extension → bus**: [`EventBus::emit_from_extension`] requires the
//!   extension's `capabilities.events.emit` patterns to cover the topic
//!   and forbids `cronymax.*`.
//!
//! Subscribers are closures — the host module wires each per-extension
//! subscription to `Connection::notify("events/publish", payload)` so
//! cross-process delivery falls out naturally. See spec §3 / §6.3 and
//! `cep-idl/v1/events.ts`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde_json::Value;

use super::error::{ExtensionError, ExtensionResult};

/// The 8 v1 platform topics. Add (never remove) variants for v1 patch
/// releases. Mirrored 1:1 with `PlatformTopic` in `cep-idl/v1/events.ts`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlatformTopic {
    SessionStarted,
    SessionEnded,
    MessageUserSent,
    MessageAssistantDelta,
    MessageAssistantDone,
    ToolInvoked,
    ToolCompleted,
    PermissionRequested,
}

impl PlatformTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStarted => "cronymax.session.started",
            Self::SessionEnded => "cronymax.session.ended",
            Self::MessageUserSent => "cronymax.message.user.sent",
            Self::MessageAssistantDelta => "cronymax.message.assistant.delta",
            Self::MessageAssistantDone => "cronymax.message.assistant.done",
            Self::ToolInvoked => "cronymax.tool.invoked",
            Self::ToolCompleted => "cronymax.tool.completed",
            Self::PermissionRequested => "cronymax.permission.requested",
        }
    }
}

/// What every subscriber receives.
#[derive(Clone, Debug)]
pub struct EventPayload {
    pub topic: String,
    /// `"cronymax"` for platform-emitted events; ext id for extension-
    /// emitted events. Useful for subscribers that care about provenance.
    pub publisher: String,
    pub data: Value,
}

/// Callback installed via [`EventBus::subscribe`]. Runs synchronously on
/// the emitter's thread, so subscribers should keep work minimal — the
/// host's wrapper hands off to an `Arc<Connection>` and returns.
pub type Listener = Arc<dyn Fn(&EventPayload) + Send + Sync + 'static>;

/// Drop-guard that removes its subscription from the bus.
#[must_use = "drop the guard immediately unsubscribes"]
pub struct SubscriptionGuard {
    id: u64,
    bus: Arc<RwLock<EventBusState>>,
}

impl std::fmt::Debug for SubscriptionGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionGuard")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.bus.write() {
            for subs in g.subscribers.values_mut() {
                subs.retain(|s| s.id != self.id);
            }
        }
    }
}

/// Cross-extension capability-gated pub/sub bus. Cheap to clone (Arc
/// internals); keep one instance per process.
#[derive(Clone, Debug, Default)]
pub struct EventBus {
    inner: Arc<RwLock<EventBusState>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Debug, Default)]
struct EventBusState {
    /// `topic` → list of subscribers.
    subscribers: HashMap<String, Vec<SubscriberEntry>>,
    /// `ext_id` → declared `events.subscribe` topics.
    subscribe_caps: HashMap<String, HashSet<String>>,
    /// `ext_id` → declared `events.emit` patterns (may end with `.*`).
    emit_caps: HashMap<String, Vec<EmitPattern>>,
}

struct SubscriberEntry {
    id: u64,
    ext_id: String,
    listener: Listener,
}

impl std::fmt::Debug for SubscriberEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriberEntry")
            .field("id", &self.id)
            .field("ext_id", &self.ext_id)
            .field("listener", &"<fn>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EmitPattern {
    /// Matches a single exact topic, e.g. `alice.x.foo`.
    Exact(String),
    /// Matches anything that starts with the prefix, e.g. `alice.x.`
    /// (from a manifest entry like `alice.x.*`).
    Prefix(String),
}

impl EmitPattern {
    fn from_manifest(raw: &str) -> Self {
        if let Some(stem) = raw.strip_suffix(".*") {
            Self::Prefix(format!("{stem}."))
        } else if raw == "*" {
            Self::Prefix(String::new())
        } else {
            Self::Exact(raw.to_string())
        }
    }

    fn matches(&self, topic: &str) -> bool {
        match self {
            Self::Exact(t) => t == topic,
            Self::Prefix(p) => topic.starts_with(p),
        }
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the capability whitelists for `ext_id` and replace any
    /// previous registration. Called by the host module right after
    /// `activate.ok`.
    pub fn register_extension(
        &self,
        ext_id: &str,
        subscribe: &[String],
        emit: &[String],
    ) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("event bus poisoned".into()))?;
        g.subscribe_caps
            .insert(ext_id.to_string(), subscribe.iter().cloned().collect());
        g.emit_caps.insert(
            ext_id.to_string(),
            emit.iter().map(|s| EmitPattern::from_manifest(s)).collect(),
        );
        Ok(())
    }

    /// Drop every subscription + cap for an extension (called on
    /// deactivate).
    pub fn unregister_extension(&self, ext_id: &str) -> ExtensionResult<()> {
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("event bus poisoned".into()))?;
        g.subscribe_caps.remove(ext_id);
        g.emit_caps.remove(ext_id);
        for subs in g.subscribers.values_mut() {
            subs.retain(|s| s.ext_id != ext_id);
        }
        Ok(())
    }

    /// Subscribe `ext_id` to `topic`. Errors if `topic` is not in the
    /// extension's declared `events.subscribe` list.
    pub fn subscribe<F>(
        &self,
        ext_id: &str,
        topic: &str,
        cb: F,
    ) -> ExtensionResult<SubscriptionGuard>
    where
        F: Fn(&EventPayload) + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut g = self
            .inner
            .write()
            .map_err(|_| ExtensionError::ManifestInvalid("event bus poisoned".into()))?;

        let allowed = g
            .subscribe_caps
            .get(ext_id)
            .map(|s| s.contains(topic))
            .unwrap_or(false);
        if !allowed {
            return Err(ExtensionError::CapabilityDenied(format!(
                "extension `{ext_id}` did not declare events.subscribe `{topic}`"
            )));
        }

        let entry = SubscriberEntry {
            id,
            ext_id: ext_id.to_string(),
            listener: Arc::new(cb),
        };
        g.subscribers
            .entry(topic.to_string())
            .or_default()
            .push(entry);

        Ok(SubscriptionGuard {
            id,
            bus: self.inner.clone(),
        })
    }

    /// Emit from cronymax core — bypasses extension caps. Used by chat /
    /// tool dispatcher / permission UI to surface `cronymax.*` events.
    pub fn emit_from_platform(&self, topic: PlatformTopic, data: Value) {
        self.fanout(&EventPayload {
            topic: topic.as_str().to_string(),
            publisher: "cronymax".into(),
            data,
        });
    }

    /// Emit from an extension. Subject to two checks:
    ///
    /// 1. `topic` must not start with `cronymax.` (reserved for platform)
    /// 2. `ext_id`'s `events.emit` patterns must cover `topic`
    pub fn emit_from_extension(
        &self,
        ext_id: &str,
        topic: &str,
        data: Value,
    ) -> ExtensionResult<()> {
        if topic.starts_with("cronymax.") || topic == "cronymax" {
            return Err(ExtensionError::NamespaceReserved(format!(
                "topic `{topic}` is reserved for the platform"
            )));
        }
        {
            let g = self
                .inner
                .read()
                .map_err(|_| ExtensionError::ManifestInvalid("event bus poisoned".into()))?;
            let patterns = g.emit_caps.get(ext_id);
            let allowed = patterns
                .map(|ps| ps.iter().any(|p| p.matches(topic)))
                .unwrap_or(false);
            if !allowed {
                return Err(ExtensionError::CapabilityDenied(format!(
                    "extension `{ext_id}` did not declare events.emit for `{topic}`"
                )));
            }
        }
        self.fanout(&EventPayload {
            topic: topic.to_string(),
            publisher: ext_id.to_string(),
            data,
        });
        Ok(())
    }

    /// Number of active subscribers for `topic`. Useful for tests and
    /// optimistic emit (skip serialising if nobody is listening).
    pub fn subscriber_count(&self, topic: &str) -> usize {
        self.inner
            .read()
            .map(|g| g.subscribers.get(topic).map(|v| v.len()).unwrap_or(0))
            .unwrap_or(0)
    }

    fn fanout(&self, payload: &EventPayload) {
        // Clone listeners under the lock, then drop the lock before
        // invoking — listeners might re-enter the bus.
        let listeners: Vec<Listener> = {
            let g = match self.inner.read() {
                Ok(g) => g,
                Err(_) => return,
            };
            g.subscribers
                .get(&payload.topic)
                .map(|subs| subs.iter().map(|s| s.listener.clone()).collect())
                .unwrap_or_default()
        };
        for l in listeners {
            l(payload);
        }
    }
}

/// Placeholder bus type kept for back-compat; new code should use
/// [`EventBus`] directly. `Default::default()` returns an empty bus.
pub type PlatformEventBus = EventBus;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn emit_pattern_parser() {
        assert_eq!(
            EmitPattern::from_manifest("alice.x.foo"),
            EmitPattern::Exact("alice.x.foo".into())
        );
        assert_eq!(
            EmitPattern::from_manifest("alice.x.*"),
            EmitPattern::Prefix("alice.x.".into())
        );
        assert_eq!(
            EmitPattern::from_manifest("*"),
            EmitPattern::Prefix("".into())
        );
    }

    #[test]
    fn emit_pattern_matching() {
        let exact = EmitPattern::Exact("a.b".into());
        assert!(exact.matches("a.b"));
        assert!(!exact.matches("a.b.c"));
        assert!(!exact.matches("a"));

        let prefix = EmitPattern::Prefix("a.b.".into());
        assert!(prefix.matches("a.b.x"));
        assert!(prefix.matches("a.b.x.y.z"));
        assert!(!prefix.matches("a.b"));
        assert!(!prefix.matches("a.c"));
    }

    #[test]
    fn topic_names_are_stable() {
        assert_eq!(
            PlatformTopic::MessageAssistantDone.as_str(),
            "cronymax.message.assistant.done"
        );
    }

    fn caps(subscribe: &[&str], emit: &[&str]) -> (Vec<String>, Vec<String>) {
        (
            subscribe.iter().map(|s| s.to_string()).collect(),
            emit.iter().map(|s| s.to_string()).collect(),
        )
    }

    #[test]
    fn subscribe_requires_declared_cap() {
        let bus = EventBus::new();
        let (sub, em) = caps(&["cronymax.message.assistant.done"], &[]);
        bus.register_extension("alice.x", &sub, &em).unwrap();

        // Declared topic — OK.
        let _g = bus
            .subscribe("alice.x", "cronymax.message.assistant.done", |_| {})
            .unwrap();

        // Undeclared topic — denied.
        let err = bus
            .subscribe("alice.x", "cronymax.session.started", |_| {})
            .unwrap_err();
        assert!(
            matches!(err, ExtensionError::CapabilityDenied(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn subscribe_without_registration_is_denied() {
        let bus = EventBus::new();
        // alice.x never called register_extension
        let err = bus
            .subscribe("alice.x", "cronymax.session.started", |_| {})
            .unwrap_err();
        assert!(matches!(err, ExtensionError::CapabilityDenied(_)));
    }

    #[test]
    fn platform_emit_fans_out_to_matching_subscribers() {
        let bus = EventBus::new();
        let (sub, em) = caps(&["cronymax.session.started"], &[]);
        bus.register_extension("alice.x", &sub, &em).unwrap();
        bus.register_extension("bob.y", &sub, &em).unwrap();

        let got: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let got_a = got.clone();
        let _g1 = bus
            .subscribe("alice.x", "cronymax.session.started", move |p| {
                got_a.lock().unwrap().push(format!("alice:{}", p.publisher));
            })
            .unwrap();
        let got_b = got.clone();
        let _g2 = bus
            .subscribe("bob.y", "cronymax.session.started", move |p| {
                got_b.lock().unwrap().push(format!("bob:{}", p.publisher));
            })
            .unwrap();

        bus.emit_from_platform(PlatformTopic::SessionStarted, json!({"id": 1}));

        let mut entries = got.lock().unwrap().clone();
        entries.sort();
        assert_eq!(entries, vec!["alice:cronymax", "bob:cronymax"]);
    }

    #[test]
    fn drop_guard_unsubscribes() {
        let bus = EventBus::new();
        let (sub, em) = caps(&["cronymax.session.started"], &[]);
        bus.register_extension("alice.x", &sub, &em).unwrap();

        let counter: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let counter_c = counter.clone();
        let g = bus
            .subscribe("alice.x", "cronymax.session.started", move |_| {
                *counter_c.lock().unwrap() += 1;
            })
            .unwrap();

        bus.emit_from_platform(PlatformTopic::SessionStarted, Value::Null);
        assert_eq!(*counter.lock().unwrap(), 1);

        drop(g);
        bus.emit_from_platform(PlatformTopic::SessionStarted, Value::Null);
        assert_eq!(*counter.lock().unwrap(), 1, "no more fires after drop");
    }

    #[test]
    fn unregister_drops_subscriptions() {
        let bus = EventBus::new();
        let (sub, em) = caps(&["cronymax.session.started"], &[]);
        bus.register_extension("alice.x", &sub, &em).unwrap();

        let counter: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let counter_c = counter.clone();
        let _g = bus
            .subscribe("alice.x", "cronymax.session.started", move |_| {
                *counter_c.lock().unwrap() += 1;
            })
            .unwrap();

        bus.unregister_extension("alice.x").unwrap();
        bus.emit_from_platform(PlatformTopic::SessionStarted, Value::Null);
        assert_eq!(*counter.lock().unwrap(), 0);
    }

    #[test]
    fn extension_emit_respects_exact_cap() {
        let bus = EventBus::new();
        let (sub, em) = caps(&[], &["alice.x.foo"]);
        bus.register_extension("alice.x", &sub, &em).unwrap();

        bus.emit_from_extension("alice.x", "alice.x.foo", json!(1))
            .unwrap();
        let err = bus
            .emit_from_extension("alice.x", "alice.x.bar", json!(2))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::CapabilityDenied(_)));
    }

    #[test]
    fn extension_emit_respects_wildcard_cap() {
        let bus = EventBus::new();
        let (sub, em) = caps(&[], &["alice.x.*"]);
        bus.register_extension("alice.x", &sub, &em).unwrap();
        bus.emit_from_extension("alice.x", "alice.x.anything", json!(1))
            .unwrap();
        bus.emit_from_extension("alice.x", "alice.x.deep.nested", json!(2))
            .unwrap();
        // Outside the prefix — denied.
        assert!(bus
            .emit_from_extension("alice.x", "bob.y.foo", json!(3))
            .is_err());
    }

    #[test]
    fn extension_emit_rejects_cronymax_topics() {
        let bus = EventBus::new();
        let (sub, em) = caps(&[], &["*"]);
        bus.register_extension("alice.x", &sub, &em).unwrap();
        let err = bus
            .emit_from_extension("alice.x", "cronymax.session.started", json!(1))
            .unwrap_err();
        assert!(matches!(err, ExtensionError::NamespaceReserved(_)));
    }

    #[test]
    fn cross_extension_subscribe_to_extension_emit() {
        // alice.x emits topic; bob.y subscribes to it.
        let bus = EventBus::new();
        let (a_sub, a_em) = caps(&[], &["alice.x.*"]);
        let (b_sub, b_em) = caps(&["alice.x.beat"], &[]);
        bus.register_extension("alice.x", &a_sub, &a_em).unwrap();
        bus.register_extension("bob.y", &b_sub, &b_em).unwrap();

        let got: Arc<Mutex<Option<EventPayload>>> = Arc::new(Mutex::new(None));
        let got_c = got.clone();
        let _g = bus
            .subscribe("bob.y", "alice.x.beat", move |p| {
                *got_c.lock().unwrap() = Some(p.clone());
            })
            .unwrap();

        bus.emit_from_extension("alice.x", "alice.x.beat", json!({"n": 7}))
            .unwrap();
        let pl = got
            .lock()
            .unwrap()
            .clone()
            .expect("subscriber must have fired");
        assert_eq!(pl.publisher, "alice.x");
        assert_eq!(pl.data, json!({"n": 7}));
    }

    #[test]
    fn subscriber_count_reflects_active_listeners() {
        let bus = EventBus::new();
        let (sub, em) = caps(&["cronymax.session.started"], &[]);
        bus.register_extension("alice.x", &sub, &em).unwrap();
        assert_eq!(bus.subscriber_count("cronymax.session.started"), 0);
        let g = bus
            .subscribe("alice.x", "cronymax.session.started", |_| {})
            .unwrap();
        assert_eq!(bus.subscriber_count("cronymax.session.started"), 1);
        drop(g);
        assert_eq!(bus.subscriber_count("cronymax.session.started"), 0);
    }
}
