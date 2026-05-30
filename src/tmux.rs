use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    Output { pane: String, bytes: Vec<u8> },
    Exit,
    Error(String),
    Notification(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyIntent {
    Printable(char),
    Backspace,
    Nonlinear,
    TogglePrediction,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxKey {
    pub command: Option<String>,
    pub intent: KeyIntent,
}

pub fn persistent_launcher() -> String {
    "tmux -CC new-session -A -s slsh".to_string()
}

pub fn command_launcher(session_name: &str, remote_command: &[String]) -> String {
    let command = remote_command.join(" ");
    format!(
        "tmux -CC new-session -s {} {}",
        quote_tmux_word(session_name),
        quote_tmux_word(&command)
    )
}

pub fn parse_control_line(line: &str) -> Option<ControlEvent> {
    if line == "%exit" || line.starts_with("%exit ") {
        return Some(ControlEvent::Exit);
    }

    if let Some(rest) = line.strip_prefix("%error ") {
        return Some(ControlEvent::Error(rest.to_string()));
    }

    if let Some(rest) = line.strip_prefix("%output ") {
        let mut parts = rest.splitn(2, ' ');
        let pane = parts.next()?.to_string();
        let escaped = parts.next().unwrap_or_default();
        return Some(ControlEvent::Output {
            pane,
            bytes: decode_tmux_bytes(escaped),
        });
    }

    line.starts_with('%')
        .then(|| ControlEvent::Notification(line.to_string()))
}

pub fn key_to_tmux(event: KeyEvent, pane: Option<&str>) -> TmuxKey {
    let modifiers = event.modifiers;

    if matches!(event.code, KeyCode::Char('g' | 'G')) && modifiers == KeyModifiers::CONTROL {
        return TmuxKey {
            command: None,
            intent: KeyIntent::TogglePrediction,
        };
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(ch) = event.code {
            let lower = ch.to_ascii_lowercase();
            if lower.is_ascii_alphabetic() {
                return named_key(pane, &format!("C-{lower}"));
            }
        }
    }

    if modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(ch) = event.code {
            return named_key(pane, &format!("M-{ch}"));
        }
        return TmuxKey {
            command: None,
            intent: KeyIntent::Nonlinear,
        };
    }

    match event.code {
        KeyCode::Char(ch) => literal_key(pane, ch),
        KeyCode::Backspace => named_key(pane, "BSpace").with_intent(KeyIntent::Backspace),
        KeyCode::Enter => named_key(pane, "Enter"),
        KeyCode::Tab | KeyCode::BackTab => named_key(pane, "Tab"),
        KeyCode::Esc => named_key(pane, "Escape"),
        KeyCode::Left => named_key(pane, "Left"),
        KeyCode::Right => named_key(pane, "Right"),
        KeyCode::Up => named_key(pane, "Up"),
        KeyCode::Down => named_key(pane, "Down"),
        KeyCode::Delete => named_key(pane, "Delete"),
        KeyCode::Home => named_key(pane, "Home"),
        KeyCode::End => named_key(pane, "End"),
        KeyCode::PageUp => named_key(pane, "PageUp"),
        KeyCode::PageDown => named_key(pane, "PageDown"),
        KeyCode::F(n) => named_key(pane, &format!("F{n}")),
        _ => TmuxKey {
            command: None,
            intent: KeyIntent::Unsupported,
        },
    }
}

pub fn resize_command(cols: u16, rows: u16) -> String {
    format!("refresh-client -C {cols}x{rows}\n")
}

fn literal_key(pane: Option<&str>, ch: char) -> TmuxKey {
    let mut command = String::from("send-keys ");
    if let Some(pane) = pane {
        command.push_str("-t ");
        command.push_str(pane);
        command.push(' ');
    }
    command.push_str("-l -- ");
    command.push_str(&quote_tmux_word(&ch.to_string()));
    command.push('\n');

    TmuxKey {
        command: Some(command),
        intent: KeyIntent::Printable(ch),
    }
}

fn named_key(pane: Option<&str>, name: &str) -> TmuxKey {
    let mut command = String::from("send-keys ");
    if let Some(pane) = pane {
        command.push_str("-t ");
        command.push_str(pane);
        command.push(' ');
    }
    command.push_str(name);
    command.push('\n');

    TmuxKey {
        command: Some(command),
        intent: KeyIntent::Nonlinear,
    }
}

trait WithIntent {
    fn with_intent(self, intent: KeyIntent) -> Self;
}

impl WithIntent for TmuxKey {
    fn with_intent(mut self, intent: KeyIntent) -> Self {
        self.intent = intent;
        self
    }
}

fn quote_tmux_word(value: &str) -> String {
    let mut quoted = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn decode_tmux_bytes(value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut iter = value.as_bytes().iter().copied().peekable();

    while let Some(byte) = iter.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }

        match iter.peek().copied() {
            Some(b'\\') => {
                iter.next();
                bytes.push(b'\\');
            }
            Some(b'n') => {
                iter.next();
                bytes.push(b'\n');
            }
            Some(b'r') => {
                iter.next();
                bytes.push(b'\r');
            }
            Some(b't') => {
                iter.next();
                bytes.push(b'\t');
            }
            Some(next) if next.is_ascii_digit() => {
                let mut value = 0u8;
                for _ in 0..3 {
                    if let Some(digit) = iter.peek().copied() {
                        if (b'0'..=b'7').contains(&digit) {
                            value = value.saturating_mul(8).saturating_add(digit - b'0');
                            iter.next();
                        } else {
                            break;
                        }
                    }
                }
                bytes.push(value);
            }
            Some(other) => {
                iter.next();
                bytes.push(other);
            }
            None => bytes.push(b'\\'),
        }
    }

    bytes
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
    fn builds_launchers() {
        assert_eq!(persistent_launcher(), "tmux -CC new-session -A -s slsh");
        assert_eq!(
            command_launcher(
                "slsh-cmd-1",
                &["cd".into(), "repo".into(), "&&".into(), "bash".into()]
            ),
            "tmux -CC new-session -s 'slsh-cmd-1' 'cd repo && bash'"
        );
    }

    #[test]
    fn command_launcher_quotes_single_quotes() {
        assert_eq!(
            command_launcher("name", &["echo".into(), "it's".into()]),
            "tmux -CC new-session -s 'name' 'echo it'\\''s'"
        );
    }

    #[test]
    fn parses_output_escapes() {
        let parsed = parse_control_line("%output %1 hi\\012there\\\\x").unwrap();

        assert_eq!(
            parsed,
            ControlEvent::Output {
                pane: "%1".into(),
                bytes: b"hi\nthere\\x".to_vec()
            }
        );
    }

    #[test]
    fn parses_exit_and_errors() {
        assert_eq!(parse_control_line("%exit"), Some(ControlEvent::Exit));
        assert_eq!(
            parse_control_line("%error nope"),
            Some(ControlEvent::Error("nope".into()))
        );
        assert_eq!(
            parse_control_line("%session-changed $1 1"),
            Some(ControlEvent::Notification("%session-changed $1 1".into()))
        );
    }

    #[test]
    fn maps_printable_key() {
        let mapped = key_to_tmux(key(KeyCode::Char('x'), KeyModifiers::NONE), Some("%1"));

        assert_eq!(mapped.intent, KeyIntent::Printable('x'));
        assert_eq!(
            mapped.command.as_deref(),
            Some("send-keys -t %1 -l -- 'x'\n")
        );
    }

    #[test]
    fn maps_backspace_and_ctrl() {
        let backspace = key_to_tmux(key(KeyCode::Backspace, KeyModifiers::NONE), None);
        assert_eq!(backspace.intent, KeyIntent::Backspace);
        assert_eq!(backspace.command.as_deref(), Some("send-keys BSpace\n"));

        let ctrl_c = key_to_tmux(key(KeyCode::Char('c'), KeyModifiers::CONTROL), Some("%1"));
        assert_eq!(ctrl_c.intent, KeyIntent::Nonlinear);
        assert_eq!(ctrl_c.command.as_deref(), Some("send-keys -t %1 C-c\n"));
    }

    #[test]
    fn ctrl_g_toggles_prediction_locally() {
        let mapped = key_to_tmux(key(KeyCode::Char('g'), KeyModifiers::CONTROL), Some("%1"));

        assert_eq!(mapped.intent, KeyIntent::TogglePrediction);
        assert_eq!(mapped.command, None);
    }

    #[test]
    fn resize_command_uses_client_size() {
        assert_eq!(resize_command(80, 24), "refresh-client -C 80x24\n");
    }
}
