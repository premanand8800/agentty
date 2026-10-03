//! Keyboard input to the bytes a terminal program expects (xterm conventions).

use alacritty_terminal::term::TermMode;
use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
fn modifier_param(m: ModifiersState) -> u8 {
    1 + m.shift_key() as u8 + 2 * m.alt_key() as u8 + 4 * m.control_key() as u8
}

fn csi_cursor(letter: char, m: ModifiersState, mode: TermMode) -> Vec<u8> {
    let p = modifier_param(m);
    if p > 1 {
        format!("\x1b[1;{p}{letter}").into_bytes()
    } else if mode.contains(TermMode::APP_CURSOR) {
        format!("\x1bO{letter}").into_bytes()
    } else {
        format!("\x1b[{letter}").into_bytes()
    }
}

fn csi_tilde(code: u8, m: ModifiersState) -> Vec<u8> {
    let p = modifier_param(m);
    if p > 1 { format!("\x1b[{code};{p}~") } else { format!("\x1b[{code}~") }.into_bytes()
}

pub fn encode(event: &KeyEvent, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
    let alt_prefix = |mut v: Vec<u8>| {
        if mods.alt_key() {
            v.insert(0, 0x1b);
        }
        v
    };
    match &event.logical_key {
        Key::Named(named) => {
            let bytes = match named {
                NamedKey::Enter => alt_prefix(b"\r".to_vec()),
                NamedKey::Backspace => alt_prefix(if mods.control_key() { vec![0x08] } else { vec![0x7f] }),
                NamedKey::Tab => {
                    if mods.shift_key() {
                        b"\x1b[Z".to_vec()
                    } else {
                        b"\t".to_vec()
                    }
                }
                NamedKey::Escape => b"\x1b".to_vec(),
                NamedKey::Space => {
                    if mods.control_key() {
                        vec![0]
                    } else {
                        alt_prefix(b" ".to_vec())
                    }
                }
                NamedKey::ArrowUp => csi_cursor('A', mods, mode),
                NamedKey::ArrowDown => csi_cursor('B', mods, mode),
                NamedKey::ArrowRight => csi_cursor('C', mods, mode),
                NamedKey::ArrowLeft => csi_cursor('D', mods, mode),
                NamedKey::Home => csi_cursor('H', mods, mode),
                NamedKey::End => csi_cursor('F', mods, mode),
                NamedKey::Insert => csi_tilde(2, mods),
                NamedKey::Delete => csi_tilde(3, mods),
                NamedKey::PageUp => csi_tilde(5, mods),
                NamedKey::PageDown => csi_tilde(6, mods),
                NamedKey::F1 => b"\x1bOP".to_vec(),
                NamedKey::F2 => b"\x1bOQ".to_vec(),
                NamedKey::F3 => b"\x1bOR".to_vec(),
                NamedKey::F4 => b"\x1bOS".to_vec(),
                NamedKey::F5 => csi_tilde(15, mods),
                NamedKey::F6 => csi_tilde(17, mods),
                NamedKey::F7 => csi_tilde(18, mods),
                NamedKey::F8 => csi_tilde(19, mods),
                NamedKey::F9 => csi_tilde(20, mods),
                NamedKey::F10 => csi_tilde(21, mods),
                NamedKey::F11 => csi_tilde(23, mods),
                NamedKey::F12 => csi_tilde(24, mods),
                _ => return None,
            };
            Some(bytes)
        }
        Key::Character(s) => {
            if mods.control_key() {
                let c = s.chars().next()?.to_ascii_lowercase();
                let byte = match c {
                    'a'..='z' => c as u8 - b'a' + 1,
                    '@' | '2' => 0,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '-' | '7' => 0x1f,
                    '8' | '?' => 0x7f,
                    _ => return event.text.as_ref().map(|t| alt_prefix(t.as_bytes().to_vec())),
                };
                return Some(alt_prefix(vec![byte]));
            }
            let text = event.text.as_ref().map(|t| t.to_string()).unwrap_or_else(|| s.to_string());
            Some(alt_prefix(text.into_bytes()))
        }
        _ => event.text.as_ref().map(|t| t.as_bytes().to_vec()),
    }
}

/// Wrap pasted text for programs that asked for bracketed paste (most agent TUIs do).
pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    let clean = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let clean = clean.replace('\x1b', "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        clean.into_bytes()
    }
}
