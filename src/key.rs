use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
#[cfg(any(not(windows), test))]
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyIntent {
    Printable(char),
    Backspace,
    Submit,
    Nonlinear,
    TogglePrediction,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedKey {
    pub bytes: Vec<u8>,
    pub intent: KeyIntent,
}

pub fn encode_key_with_mode(event: KeyEvent, application_cursor_keys: bool) -> EncodedKey {
    let modifiers = event.modifiers;

    if matches!(event.code, KeyCode::Char('g' | 'G')) && modifiers == KeyModifiers::CONTROL {
        return local(KeyIntent::TogglePrediction);
    }
    if event.code == KeyCode::Char('\u{7}') && modifiers.is_empty() {
        return local(KeyIntent::TogglePrediction);
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(ch) = event.code {
            if let Some(byte) = control_byte(ch) {
                return modified_bytes(vec![byte], modifiers, KeyIntent::Nonlinear);
            }
        }
    }

    let encoded = match event.code {
        KeyCode::Char('\r' | '\n') | KeyCode::Enter => {
            if let Some(sequence) = modified_other_key(13, modifiers) {
                bytes(sequence, KeyIntent::Nonlinear)
            } else {
                modified_bytes(vec![b'\r'], modifiers, KeyIntent::Submit)
            }
        }
        KeyCode::Char('\t') | KeyCode::Tab => {
            if let Some(sequence) = modified_other_key(9, modifiers) {
                bytes(sequence, KeyIntent::Nonlinear)
            } else {
                modified_bytes(vec![b'\t'], modifiers, KeyIntent::Nonlinear)
            }
        }
        KeyCode::Char('\u{8}' | '\u{7f}') | KeyCode::Backspace => match xterm_modifier(modifiers) {
            Some(5) => bytes(vec![0x17], KeyIntent::Backspace),
            Some(_) => modified_bytes(vec![0x7f], modifiers, KeyIntent::Backspace),
            None => bytes(vec![0x7f], KeyIntent::Backspace),
        },
        KeyCode::BackTab => bytes(b"\x1b[Z".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Esc => {
            if let Some(sequence) = modified_other_key(27, modifiers) {
                bytes(sequence, KeyIntent::Nonlinear)
            } else {
                bytes(vec![0x1b], KeyIntent::Nonlinear)
            }
        }
        KeyCode::Left => cursor_key('D', modifiers, application_cursor_keys),
        KeyCode::Right => cursor_key('C', modifiers, application_cursor_keys),
        KeyCode::Up => cursor_key('A', modifiers, application_cursor_keys),
        KeyCode::Down => cursor_key('B', modifiers, application_cursor_keys),
        KeyCode::Delete => tilde_key(3, modifiers),
        KeyCode::Insert => tilde_key(2, modifiers),
        KeyCode::Home => csi_key('H', modifiers),
        KeyCode::End => csi_key('F', modifiers),
        KeyCode::KeypadBegin => csi_key('E', modifiers),
        KeyCode::PageUp => tilde_key(5, modifiers),
        KeyCode::PageDown => tilde_key(6, modifiers),
        KeyCode::F(n) => function_key(n, modifiers),
        KeyCode::Char(ch) if !ch.is_control() => {
            let mut text = Vec::new();
            text.extend(ch.to_string().as_bytes());
            modified_bytes(text, modifiers, KeyIntent::Printable(ch))
        }
        KeyCode::Char(ch) => match control_byte(ch) {
            Some(byte) => modified_bytes(vec![byte], modifiers, KeyIntent::Nonlinear),
            None => local(KeyIntent::Unsupported),
        },
        _ => local(KeyIntent::Unsupported),
    };

    encoded
}

#[cfg(any(not(windows), test))]
pub fn encode_mouse(event: MouseEvent) -> Vec<u8> {
    let (mut code, suffix) = mouse_code(event.kind);
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if has_escape_modifier(event.modifiers) {
        code += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }

    format!(
        "\x1b[<{};{};{}{}",
        code,
        event.column.saturating_add(1),
        event.row.saturating_add(1),
        suffix
    )
    .into_bytes()
}

#[cfg(any(not(windows), test))]
pub fn encode_paste(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(b'\r');
            }
            '\n' => out.push(b'\r'),
            '\t' => out.push(b'\t'),
            '\u{8}' | '\u{7f}' => out.push(0x7f),
            _ if ch.is_control() => {
                if let Some(byte) = control_byte(ch) {
                    out.push(byte);
                }
            }
            _ => out.extend(ch.to_string().as_bytes()),
        }
    }
    out
}

#[cfg(any(not(windows), test))]
fn mouse_code(kind: MouseEventKind) -> (u16, char) {
    let press = match kind {
        MouseEventKind::Down(button) => return (mouse_button(button), 'M'),
        MouseEventKind::Up(button) => return (mouse_button(button), 'm'),
        MouseEventKind::Drag(button) => mouse_button(button) + 32,
        MouseEventKind::Moved => 35,
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
    };
    (press, 'M')
}

#[cfg(any(not(windows), test))]
fn mouse_button(button: MouseButton) -> u16 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

fn bytes(bytes: Vec<u8>, intent: KeyIntent) -> EncodedKey {
    EncodedKey { bytes, intent }
}

fn local(intent: KeyIntent) -> EncodedKey {
    EncodedKey {
        bytes: Vec::new(),
        intent,
    }
}

fn modified_bytes(bytes: Vec<u8>, modifiers: KeyModifiers, intent: KeyIntent) -> EncodedKey {
    if has_escape_modifier(modifiers) && !bytes.is_empty() {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend_from_slice(&bytes);
        return EncodedKey {
            bytes: prefixed,
            intent: KeyIntent::Nonlinear,
        };
    }
    EncodedKey { bytes, intent }
}

fn csi_key(final_byte: char, modifiers: KeyModifiers) -> EncodedKey {
    let sequence = match xterm_modifier(modifiers) {
        Some(modifier) => format!("\x1b[1;{modifier}{final_byte}"),
        None => format!("\x1b[{final_byte}"),
    };
    bytes(sequence.into_bytes(), KeyIntent::Nonlinear)
}

fn cursor_key(
    final_byte: char,
    modifiers: KeyModifiers,
    application_cursor_keys: bool,
) -> EncodedKey {
    if application_cursor_keys && xterm_modifier(modifiers).is_none() {
        return bytes(
            format!("\x1bO{final_byte}").into_bytes(),
            KeyIntent::Nonlinear,
        );
    }
    csi_key(final_byte, modifiers)
}

fn tilde_key(number: u8, modifiers: KeyModifiers) -> EncodedKey {
    let sequence = match xterm_modifier(modifiers) {
        Some(modifier) => format!("\x1b[{number};{modifier}~"),
        None => format!("\x1b[{number}~"),
    };
    bytes(sequence.into_bytes(), KeyIntent::Nonlinear)
}

fn modified_other_key(codepoint: u8, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    xterm_modifier(modifiers)
        .map(|modifier| format!("\x1b[27;{modifier};{codepoint}~").into_bytes())
}

fn xterm_modifier(modifiers: KeyModifiers) -> Option<u8> {
    let mut value = 1;
    if modifiers.contains(KeyModifiers::SHIFT) {
        value += 1;
    }
    if has_escape_modifier(modifiers) {
        value += 2;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        value += 4;
    }
    (value > 1).then_some(value)
}

fn has_escape_modifier(modifiers: KeyModifiers) -> bool {
    modifiers.intersects(KeyModifiers::ALT | KeyModifiers::META)
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some(ch as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => {
            let value = ch as u32;
            (0x01..=0x1f).contains(&value).then_some(value as u8)
        }
    }
}

fn function_key(n: u8, modifiers: KeyModifiers) -> EncodedKey {
    let (base, effective_modifiers) = if (13..=24).contains(&n) {
        (n - 12, modifiers | KeyModifiers::SHIFT)
    } else {
        (n, modifiers)
    };

    match base {
        1 => function_csi_or_ss3('P', effective_modifiers),
        2 => function_csi_or_ss3('Q', effective_modifiers),
        3 => function_csi_or_ss3('R', effective_modifiers),
        4 => function_csi_or_ss3('S', effective_modifiers),
        5 => tilde_key(15, effective_modifiers),
        6 => tilde_key(17, effective_modifiers),
        7 => tilde_key(18, effective_modifiers),
        8 => tilde_key(19, effective_modifiers),
        9 => tilde_key(20, effective_modifiers),
        10 => tilde_key(21, effective_modifiers),
        11 => tilde_key(23, effective_modifiers),
        12 => tilde_key(24, effective_modifiers),
        _ => local(KeyIntent::Unsupported),
    }
}

fn function_csi_or_ss3(final_byte: char, modifiers: KeyModifiers) -> EncodedKey {
    let sequence = match xterm_modifier(modifiers) {
        Some(modifier) => format!("\x1b[1;{modifier}{final_byte}"),
        None => format!("\x1bO{final_byte}"),
    };
    bytes(sequence.into_bytes(), KeyIntent::Nonlinear)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn enc(code: KeyCode, modifiers: KeyModifiers) -> EncodedKey {
        encode_key_with_mode(key(code, modifiers), false)
    }

    #[test]
    fn encodes_printable_key() {
        let encoded = enc(KeyCode::Char('x'), KeyModifiers::NONE);

        assert_eq!(encoded.bytes, b"x");
        assert_eq!(encoded.intent, KeyIntent::Printable('x'));
    }

    #[test]
    fn encodes_enter_backspace_and_ctrl() {
        let enter = enc(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(enter.bytes, b"\r");
        assert_eq!(enter.intent, KeyIntent::Submit);
        assert_eq!(enc(KeyCode::Backspace, KeyModifiers::NONE).bytes, &[0x7f]);
        assert_eq!(
            enc(KeyCode::Char('c'), KeyModifiers::CONTROL).bytes,
            &[0x03]
        );
        assert_eq!(
            enc(KeyCode::Char('\u{18}'), KeyModifiers::NONE).bytes,
            &[0x18]
        );
    }

    #[test]
    fn encodes_cursor_keys() {
        assert_eq!(enc(KeyCode::Left, KeyModifiers::NONE).bytes, b"\x1b[D");
        assert_eq!(enc(KeyCode::Delete, KeyModifiers::NONE).bytes, b"\x1b[3~");
    }

    #[test]
    fn encodes_application_cursor_keys() {
        assert_eq!(
            encode_key_with_mode(key(KeyCode::Up, KeyModifiers::NONE), true).bytes,
            b"\x1bOA"
        );
        assert_eq!(
            encode_key_with_mode(key(KeyCode::Down, KeyModifiers::NONE), true).bytes,
            b"\x1bOB"
        );
        assert_eq!(
            encode_key_with_mode(key(KeyCode::Up, KeyModifiers::CONTROL), true).bytes,
            b"\x1b[1;5A"
        );
    }

    #[test]
    fn encodes_modified_navigation_keys() {
        assert_eq!(
            enc(KeyCode::Left, KeyModifiers::CONTROL).bytes,
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key_with_mode(
                key(
                    KeyCode::Right,
                    KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL
                ),
                false
            )
            .bytes,
            b"\x1b[1;8C"
        );
        assert_eq!(
            enc(KeyCode::Delete, KeyModifiers::CONTROL).bytes,
            b"\x1b[3;5~"
        );
        assert_eq!(enc(KeyCode::Home, KeyModifiers::ALT).bytes, b"\x1b[1;3H");
        assert_eq!(
            enc(KeyCode::KeypadBegin, KeyModifiers::CONTROL).bytes,
            b"\x1b[1;5E"
        );
    }

    #[test]
    fn encodes_modified_function_keys() {
        assert_eq!(enc(KeyCode::F(1), KeyModifiers::NONE).bytes, b"\x1bOP");
        assert_eq!(
            enc(KeyCode::F(1), KeyModifiers::CONTROL).bytes,
            b"\x1b[1;5P"
        );
        assert_eq!(enc(KeyCode::F(5), KeyModifiers::ALT).bytes, b"\x1b[15;3~");
        assert_eq!(enc(KeyCode::F(13), KeyModifiers::NONE).bytes, b"\x1b[1;2P");
    }

    #[test]
    fn preserves_alt_modified_text_and_controls() {
        assert_eq!(enc(KeyCode::Char('x'), KeyModifiers::ALT).bytes, b"\x1bx");
        assert_eq!(
            encode_key_with_mode(
                key(
                    KeyCode::Char('c'),
                    KeyModifiers::ALT | KeyModifiers::CONTROL
                ),
                false
            )
            .bytes,
            &[0x1b, 0x03]
        );
        assert_eq!(
            enc(KeyCode::Enter, KeyModifiers::CONTROL).bytes,
            b"\x1b[27;5;13~"
        );
    }

    #[test]
    fn ctrl_g_toggles_prediction_locally() {
        let encoded = enc(KeyCode::Char('g'), KeyModifiers::CONTROL);

        assert_eq!(encoded.intent, KeyIntent::TogglePrediction);
        assert!(encoded.bytes.is_empty());

        let encoded = enc(KeyCode::Char('\u{7}'), KeyModifiers::NONE);

        assert_eq!(encoded.intent, KeyIntent::TogglePrediction);
        assert!(encoded.bytes.is_empty());
    }

    #[test]
    fn paste_normalizes_newlines_and_controls() {
        assert_eq!(encode_paste("echo hi\r\n"), b"echo hi\r");
        assert_eq!(encode_paste("ab\u{7f}\u{3}"), &[b'a', b'b', 0x7f, 0x03]);
    }

    #[test]
    fn encodes_sgr_mouse_events() {
        assert_eq!(
            encode_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 9,
                row: 4,
                modifiers: KeyModifiers::NONE,
            }),
            b"\x1b[<0;10;5M"
        );
        assert_eq!(
            encode_mouse(MouseEvent {
                kind: MouseEventKind::Up(MouseButton::Left),
                column: 9,
                row: 4,
                modifiers: KeyModifiers::NONE,
            }),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(
            encode_mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::CONTROL,
            }),
            b"\x1b[<81;1;1M"
        );
        assert_eq!(
            encode_mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Right),
                column: 2,
                row: 3,
                modifiers: KeyModifiers::SHIFT | KeyModifiers::ALT,
            }),
            b"\x1b[<46;3;4M"
        );
    }
}
