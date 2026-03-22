//! Overlay UI layer — Modal and Float egui-based child windows.
//!
//! These types sit above the renderer's `OverlayWindow` infrastructure
//! and own their own `ChildPanel` creation via `ChildPanelConfig`.
//! They provide the high-level UI rendering logic (egui drawing, event
//! handling, browser chrome, tooltip layout).

pub mod float;
pub mod modal;

pub use float::Float;
pub use modal::Modal;
