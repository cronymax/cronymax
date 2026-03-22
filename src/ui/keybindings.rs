//! Keybinding → action mapping (pure input logic, no AppState dependency).

use super::actions::KeyAction;
use winit::keyboard::ModifiersState;

/// Map a winit key event + modifier state to a [`KeyAction`].
///
/// Returns `None` if the event does not match any known binding.
pub fn match_keybinding(
    event: &winit::event::KeyEvent,
    modifiers: &ModifiersState,
) -> Option<KeyAction> {
    use winit::keyboard::{Key, NamedKey};

    if event.state != winit::event::ElementState::Pressed {
        return None;
    }

    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    let super_key = modifiers.super_key();

    if super_key {
        match &event.logical_key {
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("c") => {
                return Some(KeyAction::Copy);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("v") => {
                return Some(KeyAction::Paste);
            }
            Key::Character(c) if c.as_str() == "," => return Some(KeyAction::ToggleSettings),
            _ => {}
        }
    }

    if ctrl && shift {
        match &event.logical_key {
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("t") => {
                return Some(KeyAction::NewChat);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("w") => {
                return Some(KeyAction::CloseTab);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("c") => {
                return Some(KeyAction::Copy);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("v") => {
                return Some(KeyAction::Paste);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("d") => {
                return Some(KeyAction::SplitHorizontal);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("p") => {
                return Some(KeyAction::CommandMode);
            }
            Key::Character(c) if c.as_str().eq_ignore_ascii_case("f") => {
                return Some(KeyAction::ToggleFilter);
            }
            Key::Named(NamedKey::Tab) => return Some(KeyAction::PrevTab),
            Key::Character(c) if c.as_str() == "=" || c.as_str() == "+" => {
                return Some(KeyAction::FontSizeUp);
            }
            Key::Character(c) if c.as_str() == "-" => return Some(KeyAction::FontSizeDown),
            _ => {}
        }
    }

    if ctrl && !shift {
        match &event.logical_key {
            Key::Named(NamedKey::Tab) => return Some(KeyAction::NextTab),
            Key::Named(NamedKey::PageUp) => return Some(KeyAction::ScrollPageUp),
            Key::Named(NamedKey::PageDown) => return Some(KeyAction::ScrollPageDown),
            Key::Character(c) if c.as_str() == "=" || c.as_str() == "+" => {
                return Some(KeyAction::FontSizeUp);
            }
            Key::Character(c) if c.as_str() == "-" => return Some(KeyAction::FontSizeDown),
            _ => {}
        }
    }

    if shift && !ctrl {
        match &event.logical_key {
            Key::Named(NamedKey::PageUp) => return Some(KeyAction::ScrollPageUp),
            Key::Named(NamedKey::PageDown) => return Some(KeyAction::ScrollPageDown),
            _ => {}
        }
    }

    None
}
