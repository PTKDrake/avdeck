//! Host keys to the W3C key values the emulator understands (`KeyboardEvent.key`). Keys are sent
//! unshifted; Shift, Control and Alt go down and up on their own, as on a physical keyboard.

use egui::Key;

pub fn key_value(key: Key) -> Option<String> {
    let value = match key {
        Key::ArrowDown => "ArrowDown",
        Key::ArrowLeft => "ArrowLeft",
        Key::ArrowRight => "ArrowRight",
        Key::ArrowUp => "ArrowUp",
        Key::Escape => "Escape",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::Enter => "Enter",
        Key::Space => " ",
        Key::Insert => "Insert",
        Key::Delete => "Delete",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Semicolon => ";",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Backtick => "`",
        Key::Quote => "'",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        // Letters and digits: `A` -> `a`, `Num1` -> `1`.
        other => {
            let name = other.name();
            return (name.chars().count() == 1).then(|| name.to_lowercase());
        }
    };
    Some(value.to_owned())
}
