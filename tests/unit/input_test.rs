use cronymax::renderer::terminal::input;

use winit::keyboard::{Key, ModifiersState, NamedKey, SmolStr};

#[test]
fn test_character_input() {
    let key = Key::Character(SmolStr::new("a"));
    let mods = ModifiersState::empty();
    assert_eq!(input::encode_key_logical(&key, &mods), Some(b"a".to_vec()));
}

#[test]
fn test_enter_key() {
    let key = Key::Named(NamedKey::Enter);
    let mods = ModifiersState::empty();
    assert_eq!(input::encode_key_logical(&key, &mods), Some(b"\r".to_vec()));
}

#[test]
fn test_backspace() {
    let key = Key::Named(NamedKey::Backspace);
    let mods = ModifiersState::empty();
    assert_eq!(
        input::encode_key_logical(&key, &mods),
        Some(b"\x7f".to_vec())
    );
}

#[test]
fn test_escape() {
    let key = Key::Named(NamedKey::Escape);
    let mods = ModifiersState::empty();
    assert_eq!(
        input::encode_key_logical(&key, &mods),
        Some(b"\x1b".to_vec())
    );
}

#[test]
fn test_arrow_keys() {
    let mods = ModifiersState::empty();

    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::ArrowUp), &mods),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::ArrowDown), &mods),
        Some(b"\x1b[B".to_vec())
    );
    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::ArrowRight), &mods),
        Some(b"\x1b[C".to_vec())
    );
    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::ArrowLeft), &mods),
        Some(b"\x1b[D".to_vec())
    );
}

#[test]
fn test_function_keys() {
    let mods = ModifiersState::empty();

    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::F1), &mods),
        Some(b"\x1bOP".to_vec())
    );
    assert_eq!(
        input::encode_key_logical(&Key::Named(NamedKey::F12), &mods),
        Some(b"\x1b[24~".to_vec())
    );
}

#[test]
fn test_ctrl_c() {
    let key = Key::Character(SmolStr::new("c"));
    let mods = ModifiersState::CONTROL;
    assert_eq!(input::encode_key_logical(&key, &mods), Some(vec![0x03]));
}

#[test]
fn test_ctrl_d() {
    let key = Key::Character(SmolStr::new("d"));
    let mods = ModifiersState::CONTROL;
    assert_eq!(input::encode_key_logical(&key, &mods), Some(vec![0x04]));
}

#[test]
fn test_tab() {
    let key = Key::Named(NamedKey::Tab);
    let mods = ModifiersState::empty();
    assert_eq!(input::encode_key_logical(&key, &mods), Some(b"\t".to_vec()));
}

#[test]
fn test_shift_tab() {
    let key = Key::Named(NamedKey::Tab);
    let mods = ModifiersState::SHIFT;
    assert_eq!(
        input::encode_key_logical(&key, &mods),
        Some(b"\x1b[Z".to_vec())
    );
}
