use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    Output { pane: String, bytes: Vec<u8> },
    Exit,
    Error(String),
    Notification(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Raw(Vec<u8>),
    Control(ControlEvent),
}

#[derive(Debug, Default)]
pub struct StreamParser {
    buffer: Vec<u8>,
    control_started: bool,
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

pub fn shell_launcher(session_name: &str) -> String {
    format!("tmux -CC new-session -s {}", quote_tmux_word(session_name))
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
    let line = normalize_control_line(line)?;

    if line == "%exit" || line.starts_with("%exit ") {
        return Some(ControlEvent::Exit);
    }

    if let Some(rest) = line.strip_prefix("%error ") {
        return Some(ControlEvent::Error(rest.to_string()));
    }

    if let Some(rest) = line.strip_prefix("%output ") {
        let mut parts = rest.splitn(2, ' ');
        let pane = parts.next()?.to_string();
        let escaped = unquote_output(parts.next().unwrap_or_default());
        return Some(ControlEvent::Output {
            pane,
            bytes: decode_tmux_bytes(&escaped),
        });
    }

    line.starts_with('%')
        .then(|| ControlEvent::Notification(line.to_string()))
}

impl StreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<StreamEvent> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();

        loop {
            if !self.control_started {
                match find_control_start(&self.buffer) {
                    Some(0) => self.control_started = true,
                    Some(index) => {
                        events.push(StreamEvent::Raw(self.buffer.drain(..index).collect()));
                        self.control_started = true;
                    }
                    None => {
                        let emit = raw_emit_len(&self.buffer);
                        if emit > 0 {
                            events.push(StreamEvent::Raw(self.buffer.drain(..emit).collect()));
                        }
                        break;
                    }
                }
            }

            let Some(line_end) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            let line: Vec<u8> = self.buffer.drain(..=line_end).collect();
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches('\n');
            if let Some(event) = parse_control_line(text) {
                events.push(StreamEvent::Control(event));
            } else {
                events.push(StreamEvent::Raw(line));
            }
        }

        events
    }
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

    if let KeyCode::Char(ch) = event.code {
        match ch {
            '\r' | '\n' => return hex_key(pane, 0x0d),
            '\t' => return named_key(pane, "Tab"),
            '\u{8}' | '\u{7f}' => {
                return named_key(pane, "BSpace").with_intent(KeyIntent::Backspace);
            }
            _ => {
                if let Some(mapped) = raw_control_key(ch, pane) {
                    return mapped;
                }
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
        KeyCode::Enter => hex_key(pane, 0x0d),
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

fn raw_control_key(ch: char, pane: Option<&str>) -> Option<TmuxKey> {
    match ch as u32 {
        0x01..=0x1a => {
            let letter = (b'a' + (ch as u8) - 1) as char;
            if letter == 'g' {
                Some(TmuxKey {
                    command: None,
                    intent: KeyIntent::TogglePrediction,
                })
            } else {
                Some(named_key(pane, &format!("C-{letter}")))
            }
        }
        0x1b => Some(named_key(pane, "Escape")),
        0x1c => Some(named_key(pane, "C-\\")),
        0x1d => Some(named_key(pane, "C-]")),
        0x1e => Some(named_key(pane, "C-^")),
        0x1f => Some(named_key(pane, "C-_")),
        _ => None,
    }
}

pub fn resize_command(cols: u16, rows: u16) -> String {
    format!("refresh-client -C {cols}x{rows}\n")
}

#[cfg(any(not(windows), test))]
pub fn paste_to_tmux(text: &str, pane: Option<&str>) -> String {
    let mut command = String::new();
    let mut literal = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                flush_literal(&mut command, pane, &mut literal);
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                command.push_str(&hex_key(pane, 0x0d).command.unwrap_or_default());
            }
            '\n' => {
                flush_literal(&mut command, pane, &mut literal);
                command.push_str(&hex_key(pane, 0x0d).command.unwrap_or_default());
            }
            '\t' => {
                flush_literal(&mut command, pane, &mut literal);
                command.push_str(&named_key(pane, "Tab").command.unwrap_or_default());
            }
            '\u{8}' | '\u{7f}' => {
                flush_literal(&mut command, pane, &mut literal);
                command.push_str(&named_key(pane, "BSpace").command.unwrap_or_default());
            }
            _ if ch.is_control() => {
                flush_literal(&mut command, pane, &mut literal);
                if let Some(mapped) = raw_control_key(ch, pane) {
                    command.push_str(&mapped.command.unwrap_or_default());
                }
            }
            _ => literal.push(ch),
        }
    }

    flush_literal(&mut command, pane, &mut literal);
    command
}

#[cfg(any(not(windows), test))]
fn flush_literal(command: &mut String, pane: Option<&str>, literal: &mut String) {
    if literal.is_empty() {
        return;
    }
    command.push_str(&literal_text(pane, literal));
    literal.clear();
}

fn literal_key(pane: Option<&str>, ch: char) -> TmuxKey {
    let command = literal_text(pane, &ch.to_string());

    TmuxKey {
        command: Some(command),
        intent: KeyIntent::Printable(ch),
    }
}

fn literal_text(pane: Option<&str>, text: &str) -> String {
    let mut command = String::from("send-keys ");
    if let Some(pane) = pane {
        command.push_str("-t ");
        command.push_str(pane);
        command.push(' ');
    }
    command.push_str("-l -- ");
    command.push_str(&quote_tmux_word(text));
    command.push('\n');
    command
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

fn hex_key(pane: Option<&str>, byte: u8) -> TmuxKey {
    let mut command = String::from("send-keys ");
    if let Some(pane) = pane {
        command.push_str("-t ");
        command.push_str(pane);
        command.push(' ');
    }
    command.push_str(&format!("-H {byte:02x}\n"));

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

fn normalize_control_line(line: &str) -> Option<&str> {
    let line = line.trim_end_matches('\r');
    let start = line.find('%')?;
    let line = &line[start..];
    Some(line.strip_suffix("\x1b\\").unwrap_or(line))
}

fn find_control_start(bytes: &[u8]) -> Option<usize> {
    const DCS_PREFIX: &[u8] = b"\x1bP1000p%";
    bytes
        .windows(DCS_PREFIX.len())
        .position(|window| window == DCS_PREFIX)
        .or_else(|| {
            bytes.iter().enumerate().find_map(|(index, byte)| {
                (*byte == b'%' && (index == 0 || matches!(bytes[index - 1], b'\r' | b'\n')))
                    .then_some(index)
            })
        })
}

fn raw_emit_len(bytes: &[u8]) -> usize {
    const DCS_PREFIX: &[u8] = b"\x1bP1000p%";
    let keep = (1..DCS_PREFIX.len())
        .rev()
        .find(|len| bytes.ends_with(&DCS_PREFIX[..*len]))
        .unwrap_or(0);
    bytes.len().saturating_sub(keep)
}

fn unquote_output(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
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
            Some(b'"') => {
                iter.next();
                bytes.push(b'"');
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
        assert_eq!(
            shell_launcher("slsh-shell-1"),
            "tmux -CC new-session -s 'slsh-shell-1'"
        );
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
        let parsed = parse_control_line("%output %1 \"hi\\012there\\\\x\"").unwrap();

        assert_eq!(
            parsed,
            ControlEvent::Output {
                pane: "%1".into(),
                bytes: b"hi\nthere\\x".to_vec()
            }
        );
    }

    #[test]
    fn parses_dcs_wrapped_output() {
        let parsed =
            parse_control_line("\x1bP1000p%output %5 \"\\033[31mred\\033[0m\\015\\012\"\x1b\\")
                .unwrap();

        assert_eq!(
            parsed,
            ControlEvent::Output {
                pane: "%5".into(),
                bytes: b"\x1b[31mred\x1b[0m\r\n".to_vec()
            }
        );
    }

    #[test]
    fn parses_unquoted_output() {
        let parsed = parse_control_line("%output %7 bash-5.0# ").unwrap();

        assert_eq!(
            parsed,
            ControlEvent::Output {
                pane: "%7".into(),
                bytes: b"bash-5.0# ".to_vec()
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
    fn stream_parser_splits_raw_preamble_from_control() {
        let mut parser = StreamParser::new();
        let events = parser.push(b"Welcome\r\nprompt# \x1bP1000p%output %1 hi\r\n");

        assert_eq!(
            events,
            vec![
                StreamEvent::Raw(b"Welcome\r\nprompt# ".to_vec()),
                StreamEvent::Control(ControlEvent::Output {
                    pane: "%1".into(),
                    bytes: b"hi".to_vec()
                })
            ]
        );
    }

    #[test]
    fn stream_parser_handles_split_control_prefix() {
        let mut parser = StreamParser::new();
        assert_eq!(
            parser.push(b"abc\x1bP100"),
            vec![StreamEvent::Raw(b"abc".to_vec())]
        );
        let events = parser.push(b"0p%exit\r\n");

        assert_eq!(events, vec![StreamEvent::Control(ControlEvent::Exit)]);
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
    fn maps_raw_control_bytes() {
        let enter = key_to_tmux(key(KeyCode::Char('\r'), KeyModifiers::NONE), Some("%1"));
        assert_eq!(enter.command.as_deref(), Some("send-keys -t %1 -H 0d\n"));

        let ctrl_c = key_to_tmux(key(KeyCode::Char('\u{3}'), KeyModifiers::NONE), Some("%1"));
        assert_eq!(ctrl_c.command.as_deref(), Some("send-keys -t %1 C-c\n"));
    }

    #[test]
    fn maps_paste_text() {
        assert_eq!(
            paste_to_tmux("echo hi\r\n", Some("%1")),
            "send-keys -t %1 -l -- 'echo hi'\nsend-keys -t %1 -H 0d\n"
        );
    }

    #[test]
    fn maps_paste_controls() {
        assert_eq!(
            paste_to_tmux("ab\u{7f}\u{3}", None),
            "send-keys -l -- 'ab'\nsend-keys BSpace\nsend-keys C-c\n"
        );
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
