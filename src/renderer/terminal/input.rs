//! Keyboard/mouse → terminal input encoding.
//!
//! Maps winit KeyEvent to terminal escape sequences.

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Encode a winit key event into terminal bytes to send to the PTY.
/// Returns None if the key event should not produce PTY output.
pub fn encode_key(event: &KeyEvent, modifiers: &ModifiersState) -> Option<Vec<u8>> {
    if event.state != ElementState::Pressed {
        return None;
    }
    encode_key_logical(&event.logical_key, modifiers)
}

/// Encode a logical key + modifiers into terminal bytes.
/// This is the core encoding logic, usable without constructing a full KeyEvent.
pub fn encode_key_logical(key: &Key, modifiers: &ModifiersState) -> Option<Vec<u8>> {
    let alt = modifiers.alt_key();

    // Don't send characters to PTY when super (Cmd) is held — those are app shortcuts.
    if modifiers.super_key() {
        return None;
    }

    match key {
        Key::Character(c) => {
            let ch = c.chars().next()?;

            // Ctrl+letter produces control characters.
            if modifiers.control_key() && ch.is_ascii_alphabetic() {
                let ctrl_char = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                let mut bytes = Vec::new();
                if alt {
                    bytes.push(0x1b);
                }
                bytes.push(ctrl_char);
                return Some(bytes);
            }

            let mut bytes = Vec::new();
            if alt {
                bytes.push(0x1b);
            }
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            bytes.extend_from_slice(encoded.as_bytes());
            Some(bytes)
        }
        Key::Named(named) => encode_named_key(named, modifiers),
        _ => None,
    }
}

/// Encode named keys (arrows, function keys, etc.) into escape sequences.
fn encode_named_key(key: &NamedKey, modifiers: &ModifiersState) -> Option<Vec<u8>> {
    let alt = modifiers.alt_key();
    let shift = modifiers.shift_key();
    let _ctrl = modifiers.control_key();

    let seq: &[u8] = match key {
        NamedKey::Space => b" ",
        NamedKey::Enter => b"\r",
        NamedKey::Backspace => {
            if alt {
                return Some(b"\x1b\x7f".to_vec());
            }
            return Some(b"\x7f".to_vec());
        }
        NamedKey::Tab => {
            if shift {
                return Some(b"\x1b[Z".to_vec());
            }
            return Some(b"\t".to_vec());
        }
        NamedKey::Escape => b"\x1b",
        NamedKey::ArrowUp => b"\x1b[A",
        NamedKey::ArrowDown => b"\x1b[B",
        NamedKey::ArrowRight => b"\x1b[C",
        NamedKey::ArrowLeft => b"\x1b[D",
        NamedKey::Home => b"\x1b[H",
        NamedKey::End => b"\x1b[F",
        NamedKey::Insert => b"\x1b[2~",
        NamedKey::Delete => b"\x1b[3~",
        NamedKey::PageUp => b"\x1b[5~",
        NamedKey::PageDown => b"\x1b[6~",
        NamedKey::F1 => b"\x1bOP",
        NamedKey::F2 => b"\x1bOQ",
        NamedKey::F3 => b"\x1bOR",
        NamedKey::F4 => b"\x1bOS",
        NamedKey::F5 => b"\x1b[15~",
        NamedKey::F6 => b"\x1b[17~",
        NamedKey::F7 => b"\x1b[18~",
        NamedKey::F8 => b"\x1b[19~",
        NamedKey::F9 => b"\x1b[20~",
        NamedKey::F10 => b"\x1b[21~",
        NamedKey::F11 => b"\x1b[23~",
        NamedKey::F12 => b"\x1b[24~",
        _ => return None,
    };

    Some(seq.to_vec())
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            if let Err(e) = clipboard.set_text(text) {
                log::error!("Failed to copy to clipboard: {}", e);
            }
        }
        Err(e) => log::error!("Failed to access clipboard: {}", e),
    }
}

/// Read text from the system clipboard.
pub fn paste_from_clipboard() -> Option<String> {
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => match clipboard.get_text() {
            Ok(text) => Some(text),
            Err(e) => {
                log::error!("Failed to read clipboard: {}", e);
                None
            }
        },
        Err(e) => {
            log::error!("Failed to access clipboard: {}", e);
            None
        }
    }
}
