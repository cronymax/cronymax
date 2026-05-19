//! `activationEvents` matching engine.
//!
//! **Phase 1 implements this.** Today provides only the event-shape enum so
//! downstream modules can already write `match` arms.

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
    /// Parse the manifest string form. Returns `None` for unknown prefixes.
    pub fn parse(_raw: &str) -> Option<Self> {
        // Phase 1 (P1-T04) will fill this in.
        None
    }
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
    /// Whether this trigger satisfies the given declared event.
    pub fn matches(&self, _decl: &ActivationEvent) -> bool {
        // Phase 1 (P1-T04) will fill this in.
        false
    }
}
