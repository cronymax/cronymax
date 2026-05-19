//! MessagePack-RPC frame codec.
//!
//! Phase 2 implements `Codec` using `rmp-serde` for serde-driven encode and
//! `rmpv` for the dynamic request envelope.

/// MessagePack-RPC type tag for the request envelope.
pub const TYPE_REQUEST: u8 = 0;
pub const TYPE_RESPONSE: u8 = 1;
pub const TYPE_NOTIFY: u8 = 2;

/// Reserved RPC method names.
pub mod method {
    pub const EXTENSION_ACTIVATE: &str = "extension/activate";
    pub const EXTENSION_DEACTIVATE: &str = "extension/deactivate";
    pub const COMMANDS_REGISTER: &str = "commands/register";
    pub const COMMANDS_EXECUTE: &str = "commands/execute";
    pub const EVENTS_PUBLISH: &str = "events/publish";
    pub const EVENTS_SUBSCRIBE: &str = "events/subscribe";
}
