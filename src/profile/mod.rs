// Profile management — persistence, webview context isolation.

pub mod store;
pub mod webview;

pub use store::{MemoryEntry, MemoryTag, ProfileManager};
