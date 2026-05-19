//! L1.5 platform-event bus.
//!
//! **Phase 5 implements this.** Events flow:
//!
//! ```text
//! cronymax core (chat panel / tool dispatcher / permission ui)
//!   │  emit
//!   ▼
//! PlatformEventBus
//!   │  fan out to subscribers whose extension declared the topic in
//!   │  capabilities.events.subscribe
//!   ▼
//! Node host RPC notify("events/publish", topic, payload)
//! ```
//!
//! See `cep-idl/v1/events.ts` for the v1 topic list and payload shapes.

/// The 8 v1 platform topics. Add (never remove) variants for v1 patch
/// releases. Mirrored 1:1 with `PlatformTopic` in
/// `cep-idl/v1/events.ts`.
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

/// Placeholder bus type. Phase 5 (`P5-T01`) replaces with the real impl.
#[derive(Debug, Default)]
pub struct PlatformEventBus;

impl PlatformEventBus {
    pub fn new() -> Self {
        Self
    }
}
