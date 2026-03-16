//! Overlay UI layer — Modal and Float egui-based child windows.
//!
//! These types sit above the renderer's low-level overlay infrastructure
//! (`renderer::overlay::Overlay<T>`, panel types, `Renderer` trait) and
//! provide the high-level UI rendering logic (egui drawing, event
//! handling, browser chrome, tooltip layout).

pub mod float;
pub mod modal;

pub use float::Float;
pub use modal::Modal;
