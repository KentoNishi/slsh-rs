use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyIntent {
    Printable(char),
    Backspace,
    Nonlinear,
    TogglePrediction,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedKey {
    pub bytes: Vec<u8>,
    pub intent: KeyIntent,
}

pub fn encode_key(event: KeyEvent) -> EncodedKey {
    let modifiers = event.modifiers;

    if matches!(event.code, KeyCode::Char('g' | 'G')) && modifiers == KeyModifiers::CONTROL {
        return local(KeyIntent::TogglePrediction);
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(ch) = event.code {
            if let Some(byte) = control_byte(ch) {
                return bytes(vec![byte], KeyIntent::Nonlinear);
            }
        }
    }

    let alt = modifiers.contains(KeyModifiers::ALT);
    let encoded = match event.code {
        KeyCode::Char('\r' | '\n') | KeyCode::Enter => bytes(vec![b'\r'], KeyIntent::Nonlinear),
        KeyCode::Char('\t') | KeyCode::Tab => bytes(vec![b'\t'], KeyIntent::Nonlinear),
        KeyCode::Char('\u{8}' | '\u{7f}') | KeyCode::Backspace => {
            bytes(vec![0x7f], KeyIntent::Backspace)
        }
        KeyCode::BackTab => bytes(b"\x1b[Z".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Esc => bytes(vec![0x1b], KeyIntent::Nonlinear),
        KeyCode::Left => bytes(b"\x1b[D".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Right => bytes(b"\x1b[C".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Up => bytes(b"\x1b[A".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Down => bytes(b"\x1b[B".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Delete => bytes(b"\x1b[3~".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Insert => bytes(b"\x1b[2~".to_vec(), KeyIntent::Nonlinear),
        KeyCode::Home => bytes(b"\x1b[H".to_vec(), KeyIntent::Nonlinear),
        KeyCode::End => bytes(b"\x1b[F".to_vec(), KeyIntent::Nonlinear),
        KeyCode::PageUp => bytes(b"\x1b[5~".to_vec(), KeyIntent::Nonlinear),
        KeyCode::PageDown => bytes(b"\x1b[6~".to_vec(), KeyIntent::Nonlinear),
        KeyCode::F(n) => function_key(n),
        KeyCode::Char(ch) if !ch.is_control() => {
            let mut text = Vec::new();
            text.extend(ch.to_string().as_bytes());
            bytes(text, KeyIntent::Printable(ch))
        }
        _ => local(KeyIntent::Unsupported),
    };

    if alt && !encoded.bytes.is_empty() {
        let mut prefixed = Vec::with_capacity(encoded.bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend_from_slice(&encoded.bytes);
        return bytes(prefixed, KeyIntent::Nonlinear);
    }

    encoded
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

fn bytes(bytes: Vec<u8>, intent: KeyIntent) -> EncodedKey {
    EncodedKey { bytes, intent }
}

fn local(intent: KeyIntent) -> EncodedKey {
    EncodedKey {
        bytes: Vec::new(),
        intent,
    }
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

fn function_key(n: u8) -> EncodedKey {
    let sequence = match n {
        1 => "\x1bOP",
        2 => "\x1bOQ",
        3 => "\x1bOR",
        4 => "\x1bOS",
        5 => "\x1b[15~",
        6 => "\x1b[17~",
        7 => "\x1b[18~",
        8 => "\x1b[19~",
        9 => "\x1b[20~",
        10 => "\x1b[21~",
        11 => "\x1b[23~",
        12 => "\x1b[24~",
        _ => return local(KeyIntent::Unsupported),
    };
    bytes(sequence.as_bytes().to_vec(), KeyIntent::Nonlinear)
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

    #[test]
    fn encodes_printable_key() {
        let encoded = encode_key(key(KeyCode::Char('x'), KeyModifiers::NONE));

        assert_eq!(encoded.bytes, b"x");
        assert_eq!(encoded.intent, KeyIntent::Printable('x'));
    }

    #[test]
    fn encodes_enter_backspace_and_ctrl() {
        assert_eq!(
            encode_key(key(KeyCode::Enter, KeyModifiers::NONE)).bytes,
            b"\r"
        );
        assert_eq!(
            encode_key(key(KeyCode::Backspace, KeyModifiers::NONE)).bytes,
            &[0x7f]
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)).bytes,
            &[0x03]
        );
    }

    #[test]
    fn encodes_cursor_keys() {
        assert_eq!(
            encode_key(key(KeyCode::Left, KeyModifiers::NONE)).bytes,
            b"\x1b[D"
        );
        assert_eq!(
            encode_key(key(KeyCode::Delete, KeyModifiers::NONE)).bytes,
            b"\x1b[3~"
        );
    }

    #[test]
    fn ctrl_g_toggles_prediction_locally() {
        let encoded = encode_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));

        assert_eq!(encoded.intent, KeyIntent::TogglePrediction);
        assert!(encoded.bytes.is_empty());
    }

    #[test]
    fn paste_normalizes_newlines_and_controls() {
        assert_eq!(encode_paste("echo hi\r\n"), b"echo hi\r");
        assert_eq!(encode_paste("ab\u{7f}\u{3}"), &[b'a', b'b', 0x7f, 0x03]);
    }
}
