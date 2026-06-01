//! `activationEvents` matching engine.
//!
//! Two surfaces:
//!
//! * [`ActivationEvent::parse`] — turns the raw string from a manifest into
//!   a typed event. Used by [`super::manifest::Manifest::validate`] so an
//!   unknown prefix is rejected at install time.
//! * [`Trigger::matches`] — runtime helper the host calls when something
//!   happens (startup / command invoked / agent provider selected / view
//!   opened) to decide which declared events fire. `*` is the wildcard
//!   event — discouraged, but matches every trigger.

use super::error::{ExtensionError, ExtensionResult};

/// All recognised `activationEvents` patterns in v1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationEvent {
    /// `onStartup` — eager activation when cronymax starts (discouraged).
    Startup,
    /// `*` — same as Startup, but warned at install time.
    Star,
    /// `onCommand:<id>`.
    Command(String),
    /// `onAgentProvider:<id>` — user picked this provider in chat / flow.
    AgentProvider(String),
    /// `onView:<id>` — sidebar/settings view became visible.
    View(String),
}

impl ActivationEvent {
    /// Parse the manifest string form. Returns `None` for unknown prefixes
    /// or empty `<id>` tails (e.g. `onCommand:` with nothing after the
    /// colon).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "onStartup" => Some(Self::Startup),
            "*" => Some(Self::Star),
            _ => {
                if let Some(id) = raw.strip_prefix("onCommand:") {
                    Self::with_nonempty(id, Self::Command)
                } else if let Some(id) = raw.strip_prefix("onAgentProvider:") {
                    Self::with_nonempty(id, Self::AgentProvider)
                } else if let Some(id) = raw.strip_prefix("onView:") {
                    Self::with_nonempty(id, Self::View)
                } else {
                    None
                }
            }
        }
    }

    fn with_nonempty(id: &str, ctor: impl FnOnce(String) -> Self) -> Option<Self> {
        if id.is_empty() {
            None
        } else {
            Some(ctor(id.to_string()))
        }
    }

    /// `true` for events that fire on platform startup with no user
    /// interaction (`onStartup` and `*`). Useful for the install-time
    /// consent UI: eager events should be highlighted because they let the
    /// extension run code immediately.
    pub fn is_eager(&self) -> bool {
        matches!(self, Self::Startup | Self::Star)
    }
}

/// Parse every entry in `manifest.activationEvents` and surface the first
/// unrecognised one as `ExtensionError::UnknownActivationEvent`. Used by
/// `Manifest::validate`.
pub fn parse_all(raw_events: &[String]) -> ExtensionResult<Vec<ActivationEvent>> {
    raw_events
        .iter()
        .map(|raw| {
            ActivationEvent::parse(raw)
                .ok_or_else(|| ExtensionError::UnknownActivationEvent(raw.clone()))
        })
        .collect()
}

/// A trigger that occurred at runtime (something the platform did) which may
/// activate one or more extensions.
#[derive(Clone, Debug)]
pub enum Trigger {
    Startup,
    Command(String),
    AgentProviderSelected(String),
    ViewOpened(String),
}

impl Trigger {
    /// Whether this trigger satisfies the given declared event. `*` matches
    /// every trigger.
    pub fn matches(&self, decl: &ActivationEvent) -> bool {
        if matches!(decl, ActivationEvent::Star) {
            return true;
        }
        match (self, decl) {
            (Trigger::Startup, ActivationEvent::Startup) => true,
            (Trigger::Command(a), ActivationEvent::Command(b)) => a == b,
            (Trigger::AgentProviderSelected(a), ActivationEvent::AgentProvider(b)) => a == b,
            (Trigger::ViewOpened(a), ActivationEvent::View(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_prefixes() {
        assert_eq!(
            ActivationEvent::parse("onStartup"),
            Some(ActivationEvent::Startup)
        );
        assert_eq!(ActivationEvent::parse("*"), Some(ActivationEvent::Star));
        assert_eq!(
            ActivationEvent::parse("onCommand:alice.hello"),
            Some(ActivationEvent::Command("alice.hello".into())),
        );
        assert_eq!(
            ActivationEvent::parse("onAgentProvider:alice.agent"),
            Some(ActivationEvent::AgentProvider("alice.agent".into())),
        );
        assert_eq!(
            ActivationEvent::parse("onView:alice.sidebar"),
            Some(ActivationEvent::View("alice.sidebar".into())),
        );
    }

    #[test]
    fn parse_rejects_empty_id_tail() {
        assert!(ActivationEvent::parse("onCommand:").is_none());
        assert!(ActivationEvent::parse("onAgentProvider:").is_none());
        assert!(ActivationEvent::parse("onView:").is_none());
    }

    #[test]
    fn parse_rejects_unknown_prefix() {
        assert!(ActivationEvent::parse("onLaunch").is_none());
        assert!(ActivationEvent::parse("").is_none());
        assert!(
            ActivationEvent::parse("oncommand:lower").is_none(),
            "case-sensitive"
        );
        assert!(
            ActivationEvent::parse("onCommandalice").is_none(),
            "prefix without colon is not Command",
        );
    }

    #[test]
    fn parse_treats_id_as_opaque() {
        // The id portion can contain arbitrary characters — the matcher
        // compares them verbatim.
        let ev = ActivationEvent::parse("onCommand:a.b.c-1.2_3").unwrap();
        assert_eq!(ev, ActivationEvent::Command("a.b.c-1.2_3".into()));
    }

    #[test]
    fn is_eager_classification() {
        assert!(ActivationEvent::Startup.is_eager());
        assert!(ActivationEvent::Star.is_eager());
        assert!(!ActivationEvent::Command("x".into()).is_eager());
        assert!(!ActivationEvent::AgentProvider("x".into()).is_eager());
        assert!(!ActivationEvent::View("x".into()).is_eager());
    }

    #[test]
    fn parse_all_collects_all() {
        let raw = vec!["onStartup".into(), "onCommand:foo".into()];
        let parsed = parse_all(&raw).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn parse_all_surfaces_first_unknown_event() {
        let raw = vec![
            "onStartup".into(),
            "onWeirdEvent".into(),
            "onCommand:foo".into(),
        ];
        let err = parse_all(&raw).unwrap_err();
        match err {
            ExtensionError::UnknownActivationEvent(ev) => assert_eq!(ev, "onWeirdEvent"),
            other => panic!("expected UnknownActivationEvent, got {other:?}"),
        }
    }

    #[test]
    fn trigger_matches_startup() {
        assert!(Trigger::Startup.matches(&ActivationEvent::Startup));
        assert!(!Trigger::Startup.matches(&ActivationEvent::Command("any".into())));
    }

    #[test]
    fn trigger_matches_command_by_id() {
        let t = Trigger::Command("alice.hello".into());
        assert!(t.matches(&ActivationEvent::Command("alice.hello".into())));
        assert!(!t.matches(&ActivationEvent::Command("bob.bye".into())));
        assert!(!t.matches(&ActivationEvent::AgentProvider("alice.hello".into())));
    }

    #[test]
    fn trigger_matches_agent_provider_by_id() {
        let t = Trigger::AgentProviderSelected("alice.agent".into());
        assert!(t.matches(&ActivationEvent::AgentProvider("alice.agent".into())));
        assert!(!t.matches(&ActivationEvent::AgentProvider("other".into())));
    }

    #[test]
    fn trigger_matches_view_by_id() {
        let t = Trigger::ViewOpened("alice.sidebar".into());
        assert!(t.matches(&ActivationEvent::View("alice.sidebar".into())));
        assert!(!t.matches(&ActivationEvent::View("other".into())));
    }

    #[test]
    fn star_matches_every_trigger() {
        assert!(Trigger::Startup.matches(&ActivationEvent::Star));
        assert!(Trigger::Command("anything".into()).matches(&ActivationEvent::Star));
        assert!(Trigger::AgentProviderSelected("x".into()).matches(&ActivationEvent::Star));
        assert!(Trigger::ViewOpened("x".into()).matches(&ActivationEvent::Star));
    }

    #[test]
    fn cross_kind_triggers_never_match() {
        let cmd = Trigger::Command("x".into());
        assert!(!cmd.matches(&ActivationEvent::Startup));
        assert!(!cmd.matches(&ActivationEvent::View("x".into())));
    }
}
