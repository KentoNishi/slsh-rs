mod input;
mod key;
mod predict;
mod render;
mod screen;
mod ssh_args;
mod transport;

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::ResetColor;
use crossterm::terminal;
use input::InputEvent;
use predict::Overlay;
use render::Renderer;
use screen::{ActiveBuffer, Screen, Size};
use ssh_args::{LaunchMode, ParsedSshArgs};
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use transport::Transport;

#[cfg(unix)]
use std::os::fd::AsRawFd;

const REMOTE_REPAINT_QUIET: Duration = Duration::from_millis(16);
const REMOTE_REPAINT_MAX: Duration = Duration::from_millis(50);

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("slsh: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let args = env::args().skip(1).collect();
    let parsed = ssh_args::parse(args, io::stdin().is_terminal(), io::stdout().is_terminal());

    match parsed.mode {
        LaunchMode::Passthrough => run_passthrough(&parsed.forwarded_args),
        LaunchMode::Compositor => run_compositor(parsed),
    }
}

fn run_passthrough(args: &[String]) -> Result<i32> {
    let status = Command::new("ssh")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run ssh")?;
    Ok(transport::std_exit_code(status))
}

fn run_compositor(parsed: ParsedSshArgs) -> Result<i32> {
    let ssh_args = compositor_ssh_args(&parsed);

    let (cols, rows) = terminal::size().context("failed to read terminal size")?;
    let size = Size {
        cols: cols.max(1),
        rows: rows.max(1),
    };
    let mut screen = Screen::new_at(size, initial_terminal_cursor(size));
    let mut parser = vte::Parser::new();
    let mut predictor = predict::default_predictor(parsed.slsh.predict);
    let mut renderer = Renderer::new();
    let mut transport = if Transport::loopback_enabled() {
        Transport::spawn_loopback(&parsed.remote_command, cols.max(1), rows.max(1))?
    } else {
        Transport::spawn_ssh(&ssh_args, cols.max(1), rows.max(1))?
    };
    let mut terminal_guard = TerminalGuard::enter()?;
    let mut stdout = io::stdout();
    let mut key_trace = KeyTrace::from_env();
    key_trace.log(format_args!("predictor {}", predictor.name()));
    let mut pressed_keys = HashSet::new();
    let mut remote_coalescer = RemoteCoalescer::default();
    let mut terminal_queries = TerminalQueryParser::default();
    let mut waiting_for_resize_frame = false;
    #[cfg(not(windows))]
    let mut mouse_protocol = key::MouseProtocol::default();

    loop {
        let mut dirty = false;

        remote_coalescer.push(transport.drain_chunks(), Instant::now());
        if remote_coalescer.ready(Instant::now(), overlay_pending(predictor.overlay())) {
            let remote_output = remote_coalescer.take();

            let remote_update = apply_remote_bytes(
                &remote_output,
                &mut screen,
                &mut parser,
                predictor.as_mut(),
                &mut terminal_queries,
            );
            if !remote_update.terminal_responses.is_empty() {
                transport.write(&remote_update.terminal_responses)?;
            }
            waiting_for_resize_frame = false;
            #[cfg(not(windows))]
            mouse_protocol.feed(&remote_output);
            if remote_update.terminal_mode_changed {
                let terminal_output = strip_terminal_queries(&remote_output);
                stdout
                    .write_all(&terminal_output)
                    .context("failed to render ssh output")?;
                if remote_update.left_alternate {
                    stdout
                        .write_all(b"\x1b[0m")
                        .context("failed to reset terminal style")?;
                }
                stdout.flush().context("failed to flush ssh output")?;
                predictor.clear();
                if remote_update.left_alternate {
                    screen.reset_style();
                }
                renderer.sync_to_terminal(&screen, predictor.overlay());
                dirty = false;
            } else {
                dirty = true;
            }
        }

        while input::poll(Duration::from_millis(1)).context("failed to poll terminal input")? {
            match input::read().context("failed to read terminal input")? {
                Some(InputEvent::Key(key)) => {
                    let encoded = key::encode_key_with_mode(key, screen.application_cursor_keys());
                    let should_forward = match key.kind {
                        KeyEventKind::Press | KeyEventKind::Repeat => {
                            pressed_keys.insert(key_fingerprint(key));
                            true
                        }
                        KeyEventKind::Release => {
                            pressed_keys.remove(&key_fingerprint(key));
                            false
                        }
                    };
                    key_trace.log(format_args!(
                        "key {:?} forwarded {should_forward} bytes {:?} intent {:?}",
                        key, encoded.bytes, encoded.intent
                    ));
                    if should_forward {
                        if !encoded.bytes.is_empty() {
                            transport.write(&encoded.bytes)?;
                        }
                        if waiting_for_resize_frame {
                            predictor.clear();
                        } else {
                            predictor.on_key(encoded.intent, &screen);
                        }
                        key_trace.log(format_args!(
                            "predict cursor {:?} overlay {} overlay_cursor {:?}",
                            screen.cursor(),
                            predictor.overlay().cells.len(),
                            predictor.overlay().cursor
                        ));
                        dirty = true;
                    }
                }
                Some(InputEvent::Resize(cols, rows)) => {
                    let cols = cols.max(1);
                    let rows = rows.max(1);
                    key_trace.log(format_args!(
                        "resize {cols}x{rows} current {:?}",
                        screen.size()
                    ));
                    if screen.size() == (Size { cols, rows }) {
                        continue;
                    }
                    remote_coalescer.clear();
                    screen.resize_for_remote_reflow(Size { cols, rows });
                    transport.resize(cols, rows)?;
                    renderer.invalidate();
                    predictor.clear();
                    waiting_for_resize_frame = true;
                    dirty = true;
                }
                #[cfg(not(windows))]
                Some(InputEvent::Mouse(mouse)) => {
                    let bytes = key::encode_mouse(mouse, mouse_protocol);
                    key_trace.log(format_args!("mouse {:?} bytes {:?}", mouse, bytes));
                    if !bytes.is_empty() {
                        transport.write(&bytes)?;
                    }
                    predictor.clear();
                    dirty = true;
                }
                #[cfg(not(windows))]
                Some(InputEvent::Paste(text)) => {
                    let bytes = key::encode_paste(&text);
                    key_trace.log(format_args!("paste {:?} bytes {:?}", text, bytes));
                    if !bytes.is_empty() {
                        transport.write(&bytes)?;
                    }
                    predictor.clear();
                    dirty = true;
                }
                None => {}
            }
        }

        if dirty {
            let output = renderer.render(&screen, predictor.overlay());
            key_trace.log(format_args!("render bytes {}", output.len()));
            stdout
                .write_all(output.as_bytes())
                .context("failed to render terminal")?;
            stdout.flush().context("failed to flush terminal")?;
        }

        if let Some(status) = transport.try_wait()? {
            if !remote_coalescer.is_empty() {
                let remote_output = remote_coalescer.take();
                let remote_update = apply_remote_bytes(
                    &remote_output,
                    &mut screen,
                    &mut parser,
                    predictor.as_mut(),
                    &mut terminal_queries,
                );
                if !remote_update.terminal_responses.is_empty() {
                    transport.write(&remote_update.terminal_responses)?;
                }
                let output = renderer.render(&screen, predictor.overlay());
                stdout
                    .write_all(output.as_bytes())
                    .context("failed to render terminal")?;
                stdout.flush().context("failed to flush terminal")?;
            }
            terminal_guard
                .leave_after_screen(&mut stdout, &screen, predictor.overlay())
                .context("failed to restore terminal")?;
            return Ok(transport::pty_exit_code(status));
        }

        if !dirty {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteUpdate {
    left_alternate: bool,
    terminal_mode_changed: bool,
    terminal_responses: Vec<u8>,
}

#[derive(Debug, Default)]
struct RemoteCoalescer {
    bytes: Vec<u8>,
    first_at: Option<Instant>,
    last_at: Option<Instant>,
}

impl RemoteCoalescer {
    fn push(&mut self, chunks: Vec<Vec<u8>>, now: Instant) {
        for chunk in chunks {
            if chunk.is_empty() {
                continue;
            }
            if self.bytes.is_empty() {
                self.first_at = Some(now);
            }
            self.last_at = Some(now);
            self.bytes.extend_from_slice(&chunk);
        }
    }

    fn ready(&self, now: Instant, overlay_pending: bool) -> bool {
        if self.bytes.is_empty() {
            return false;
        }
        if !overlay_pending {
            return true;
        }
        self.last_at
            .is_some_and(|last_at| now.duration_since(last_at) >= REMOTE_REPAINT_QUIET)
            || self
                .first_at
                .is_some_and(|first_at| now.duration_since(first_at) >= REMOTE_REPAINT_MAX)
    }

    fn take(&mut self) -> Vec<u8> {
        self.first_at = None;
        self.last_at = None;
        std::mem::take(&mut self.bytes)
    }

    fn clear(&mut self) {
        self.first_at = None;
        self.last_at = None;
        self.bytes.clear();
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

fn overlay_pending(overlay: &Overlay) -> bool {
    !overlay.cells.is_empty() || overlay.cursor.is_some()
}

fn apply_remote_bytes(
    bytes: &[u8],
    screen: &mut Screen,
    parser: &mut vte::Parser,
    predictor: &mut dyn predict::PredictorPlugin,
    terminal_queries: &mut TerminalQueryParser,
) -> RemoteUpdate {
    let before_active = screen.active();
    let before_application_cursor = screen.application_cursor_keys();
    let mut terminal_responses = Vec::new();
    for byte in bytes {
        screen.feed(parser, std::slice::from_ref(byte));
        terminal_queries.push(*byte, screen, &mut terminal_responses);
    }
    predictor.reconcile(screen);

    let left_alternate = (before_active == ActiveBuffer::Alternate
        && screen.active() == ActiveBuffer::Primary)
        || contains_alternate_exit(bytes);
    let terminal_mode_changed = left_alternate
        || before_active != screen.active()
        || before_application_cursor != screen.application_cursor_keys()
        || contains_terminal_mode_change(bytes);

    RemoteUpdate {
        left_alternate,
        terminal_mode_changed,
        terminal_responses,
    }
}

#[derive(Debug, Default)]
struct TerminalQueryParser {
    state: TerminalQueryState,
}

#[derive(Debug, Default)]
enum TerminalQueryState {
    #[default]
    Ground,
    Escape,
    Csi(Vec<u8>),
    Osc(Vec<u8>),
    OscEscape(Vec<u8>),
}

impl TerminalQueryParser {
    fn push(&mut self, byte: u8, screen: &Screen, responses: &mut Vec<u8>) {
        let state = std::mem::take(&mut self.state);
        self.state = match state {
            TerminalQueryState::Ground => {
                if byte == 0x1b {
                    TerminalQueryState::Escape
                } else {
                    TerminalQueryState::Ground
                }
            }
            TerminalQueryState::Escape => match byte {
                b'[' => TerminalQueryState::Csi(Vec::new()),
                b']' => TerminalQueryState::Osc(Vec::new()),
                0x1b => TerminalQueryState::Escape,
                _ => TerminalQueryState::Ground,
            },
            TerminalQueryState::Csi(mut bytes) => {
                if (0x40..=0x7e).contains(&byte) {
                    if let Some(response) = csi_terminal_response(&bytes, byte, screen.cursor()) {
                        responses.extend_from_slice(response.as_bytes());
                    }
                    TerminalQueryState::Ground
                } else {
                    bytes.push(byte);
                    TerminalQueryState::Csi(bytes)
                }
            }
            TerminalQueryState::Osc(mut bytes) => match byte {
                0x07 => {
                    if let Some(response) = osc_terminal_response(&bytes) {
                        responses.extend_from_slice(response.as_bytes());
                    }
                    TerminalQueryState::Ground
                }
                0x1b => TerminalQueryState::OscEscape(bytes),
                _ => {
                    bytes.push(byte);
                    TerminalQueryState::Osc(bytes)
                }
            },
            TerminalQueryState::OscEscape(mut bytes) => {
                if byte == b'\\' {
                    if let Some(response) = osc_terminal_response(&bytes) {
                        responses.extend_from_slice(response.as_bytes());
                    }
                    TerminalQueryState::Ground
                } else {
                    bytes.push(0x1b);
                    bytes.push(byte);
                    TerminalQueryState::Osc(bytes)
                }
            }
        };
    }
}

fn csi_terminal_response(body: &[u8], final_byte: u8, cursor: screen::Cursor) -> Option<String> {
    match (body, final_byte) {
        (b"6", b'n') => Some(format!("\x1b[{};{}R", cursor.row + 1, cursor.col + 1)),
        (b"" | b"0", b'c') => Some("\x1b[?1;2c".into()),
        (b">", b'c') => Some("\x1b[>0;0;0c".into()),
        _ => None,
    }
}

fn osc_terminal_response(body: &[u8]) -> Option<&'static str> {
    match body {
        b"10;?" => Some("\x1b]10;rgb:ffff/ffff/ffff\x07"),
        b"11;?" => Some("\x1b]11;rgb:0000/0000/0000\x07"),
        b"12;?" => Some("\x1b]12;rgb:ffff/ffff/ffff\x07"),
        _ => None,
    }
}

fn strip_terminal_queries(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            out.push(bytes[index]);
            index += 1;
            continue;
        }

        if let Some(end) = csi_end(bytes, index) {
            let body = &bytes[index + 2..end];
            let final_byte = bytes[end];
            if csi_terminal_response(body, final_byte, screen::Cursor::default()).is_some() {
                index = end + 1;
                continue;
            }
        }

        if let Some(end) = osc_end(bytes, index) {
            let body_end = if bytes[end] == 0x07 { end } else { end - 1 };
            if osc_terminal_response(&bytes[index + 2..body_end]).is_some() {
                index = end + 1;
                continue;
            }
        }

        out.push(bytes[index]);
        index += 1;
    }
    out
}

fn csi_end(bytes: &[u8], index: usize) -> Option<usize> {
    if bytes.get(index..index + 2) != Some(b"\x1b[") {
        return None;
    }
    (index + 2..bytes.len()).find(|offset| (0x40..=0x7e).contains(&bytes[*offset]))
}

fn osc_end(bytes: &[u8], index: usize) -> Option<usize> {
    if bytes.get(index..index + 2) != Some(b"\x1b]") {
        return None;
    }
    let mut offset = index + 2;
    while offset < bytes.len() {
        match bytes[offset] {
            0x07 => return Some(offset),
            0x1b if bytes.get(offset + 1) == Some(&b'\\') => return Some(offset + 1),
            _ => offset += 1,
        }
    }
    None
}

fn compositor_ssh_args(parsed: &ParsedSshArgs) -> Vec<String> {
    let mut args = Vec::with_capacity(parsed.forwarded_args.len() + 1);
    args.push("-tt".into());
    args.extend(parsed.forwarded_args.iter().cloned());
    args
}

fn initial_terminal_cursor(size: Size) -> screen::Cursor {
    query_terminal_cursor(Duration::from_millis(250))
        .ok()
        .map(|(col, row)| screen::Cursor {
            row: row.min(size.rows.saturating_sub(1)),
            col: col.min(size.cols.saturating_sub(1)),
        })
        .unwrap_or_default()
}

#[cfg(windows)]
fn query_terminal_cursor(_timeout: Duration) -> io::Result<(u16, u16)> {
    crossterm::cursor::position()
}

#[cfg(unix)]
fn query_terminal_cursor(timeout: Duration) -> io::Result<(u16, u16)> {
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    let original = unsafe { original.assume_init() };
    let mut raw = original;
    unsafe { libc::cfmakeraw(&mut raw) };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let _restore = TermiosRestore {
        fd,
        termios: original,
    };

    let mut stdout = io::stdout();
    stdout.write_all(b"\x1b[6n")?;
    stdout.flush()?;

    read_cursor_response(fd, timeout)
}

#[cfg(unix)]
struct TermiosRestore {
    fd: i32,
    termios: libc::termios,
}

#[cfg(unix)]
impl Drop for TermiosRestore {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.termios) };
    }
}

#[cfg(unix)]
fn read_cursor_response(fd: i32, timeout: Duration) -> io::Result<(u16, u16)> {
    let deadline = Instant::now() + timeout;
    let mut response = Vec::with_capacity(32);
    while Instant::now() < deadline && response.len() < 32 {
        if !wait_readable(fd, deadline.saturating_duration_since(Instant::now()))? {
            break;
        }

        let mut byte = 0u8;
        let read = unsafe { libc::read(fd, (&mut byte as *mut u8).cast(), 1) };
        if read < 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            break;
        }
        response.push(byte);
        if byte == b'R' {
            break;
        }
    }

    parse_cursor_response(&response)
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "cursor position unavailable"))
}

#[cfg(unix)]
fn wait_readable(fd: i32, timeout: Duration) -> io::Result<bool> {
    let mut readfds = unsafe { std::mem::zeroed::<libc::fd_set>() };
    unsafe {
        libc::FD_ZERO(&mut readfds);
        libc::FD_SET(fd, &mut readfds);
    }

    let mut timeout = libc::timeval {
        tv_sec: timeout.as_secs().min(i64::MAX as u64) as _,
        tv_usec: timeout.subsec_micros() as _,
    };
    let ready = unsafe {
        libc::select(
            fd + 1,
            &mut readfds,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut timeout,
        )
    };
    if ready < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ready > 0)
    }
}

#[cfg(unix)]
fn parse_cursor_response(bytes: &[u8]) -> Option<(u16, u16)> {
    let response = std::str::from_utf8(bytes).ok()?;
    let body = response.strip_prefix("\x1b[")?.strip_suffix('R')?;
    let (row, col) = body.split_once(';')?;
    let row = row.parse::<u16>().ok()?.saturating_sub(1);
    let col = col.parse::<u16>().ok()?.saturating_sub(1);
    Some((col, row))
}

fn key_fingerprint(key: KeyEvent) -> String {
    let mut modifiers = key.modifiers;
    let code = match key.code {
        KeyCode::Char('\r' | '\n') | KeyCode::Enter => "Enter".to_string(),
        KeyCode::Char('\u{8}' | '\u{7f}') | KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Char(ch) => {
            modifiers.remove(KeyModifiers::SHIFT);
            format!("Char({ch})")
        }
        other => format!("{other:?}"),
    };
    format!("{modifiers:?}:{code}")
}

fn contains_alternate_exit(bytes: &[u8]) -> bool {
    [b"\x1b[?47l".as_slice(), b"\x1b[?1047l", b"\x1b[?1049l"]
        .iter()
        .any(|pattern| {
            bytes
                .windows(pattern.len())
                .any(|window| window == *pattern)
        })
}

fn contains_terminal_mode_change(bytes: &[u8]) -> bool {
    [
        b"\x1b[?47h".as_slice(),
        b"\x1b[?47l",
        b"\x1b[?1047h",
        b"\x1b[?1047l",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[?1h",
        b"\x1b[?1l",
    ]
    .iter()
    .any(|pattern| {
        bytes
            .windows(pattern.len())
            .any(|window| window == *pattern)
    }) || contains_private_mode_change(bytes, &[1000, 1002, 1003, 1005, 1006, 1015])
}

fn contains_private_mode_change(bytes: &[u8], modes: &[u16]) -> bool {
    let mut index = 0;
    while let Some(start) = find_bytes(&bytes[index..], b"\x1b[?") {
        index += start + 3;
        let params_start = index;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b';') {
            index += 1;
        }
        if index < bytes.len()
            && matches!(bytes[index], b'h' | b'l')
            && private_mode_params_contain(&bytes[params_start..index], modes)
        {
            return true;
        }
    }
    false
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn private_mode_params_contain(params: &[u8], modes: &[u16]) -> bool {
    params
        .split(|byte| *byte == b';')
        .filter_map(|param| std::str::from_utf8(param).ok()?.parse::<u16>().ok())
        .any(|mode| modes.contains(&mode))
}

struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        #[cfg(windows)]
        let _ = crossterm::ansi_support::supports_ansi();

        terminal::enable_raw_mode().context("failed to enable raw mode")?;
        Ok(Self { restored: false })
    }

    fn leave_after_screen(
        &mut self,
        stdout: &mut impl Write,
        screen: &Screen,
        overlay: &Overlay,
    ) -> Result<()> {
        stdout
            .write_all(terminal_restore_sequence(screen, overlay).as_bytes())
            .context("failed to write terminal restore sequence")?;
        stdout.flush().context("failed to flush terminal restore")?;
        terminal::disable_raw_mode().context("failed to disable raw mode")?;
        self.restored = true;
        Ok(())
    }
}

struct KeyTrace {
    file: Option<File>,
}

impl KeyTrace {
    fn from_env() -> Self {
        let file = env::var_os("SLSH_KEY_LOG")
            .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
        Self { file }
    }

    fn log(&mut self, args: fmt::Arguments<'_>) {
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{args}");
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        let _ = execute!(
            io::stdout(),
            crossterm::style::Print(fallback_terminal_restore_sequence()),
            ResetColor,
            Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn terminal_restore_sequence(screen: &Screen, overlay: &Overlay) -> String {
    let rows = screen.size().rows.max(1);
    let mut sequence = String::from(terminal_mode_restore_sequence());
    if should_scroll_for_restore(screen, overlay) {
        sequence.push_str(&format!("\x1b[{};1H\x1b[K\r\n", rows));
    } else {
        let row = restore_target_row(screen, overlay);
        sequence.push_str(&format!("\x1b[{};1H\x1b[K", row + 1));
    }
    sequence
}

fn fallback_terminal_restore_sequence() -> &'static str {
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1049l\x1b[0m\x1b[?25h\r\n"
}

fn terminal_mode_restore_sequence() -> &'static str {
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1049l\x1b[0m\x1b[?25h"
}

fn should_scroll_for_restore(screen: &Screen, overlay: &Overlay) -> bool {
    final_content_row(screen, overlay) == Some(screen.size().rows.saturating_sub(1))
}

fn restore_target_row(screen: &Screen, overlay: &Overlay) -> u16 {
    let rows = screen.size().rows.max(1);
    let content_row = final_content_row(screen, overlay);
    let cursor_row = restore_cursor_row(screen, overlay);

    if content_row.is_none_or(|row| cursor_row > row) && row_is_blank(screen, overlay, cursor_row) {
        return cursor_row.min(rows.saturating_sub(1));
    }

    content_row
        .map(|row| row.saturating_add(1).min(rows.saturating_sub(1)))
        .unwrap_or(cursor_row.min(rows.saturating_sub(1)))
}

fn restore_cursor_row(screen: &Screen, overlay: &Overlay) -> u16 {
    let mut row = screen.cursor().row;
    if let Some(cursor) = overlay.cursor {
        row = row.max(cursor.row);
    }
    row
}

fn final_content_row(screen: &Screen, overlay: &Overlay) -> Option<u16> {
    let mut row = None;
    for cell in &overlay.cells {
        row = Some(row.map_or(cell.pos.row, |row: u16| row.max(cell.pos.row)));
    }
    let cols = screen.size().cols.max(1) as usize;
    for (index, cell) in screen.cells().iter().enumerate() {
        if *cell != screen::Cell::default() {
            let cell_row = (index / cols) as u16;
            row = Some(row.map_or(cell_row, |row| row.max(cell_row)));
        }
    }
    row.map(|row| row.min(screen.size().rows.saturating_sub(1)))
}

fn row_is_blank(screen: &Screen, overlay: &Overlay, row: u16) -> bool {
    let cols = screen.size().cols;
    if (0..cols).any(|col| screen.cell(screen::Cursor { row, col }) != screen::Cell::default()) {
        return false;
    }
    !overlay.cells.iter().any(|cell| cell.pos.row == row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_uses_forwarded_args_with_forced_tty() {
        let parsed = ssh_args::parse(
            vec![
                "-p".into(),
                "2222".into(),
                "host".into(),
                "bash".into(),
                "-l".into(),
            ],
            true,
            true,
        );

        assert_eq!(
            compositor_ssh_args(&parsed),
            vec!["-tt", "-p", "2222", "host", "bash", "-l"]
        );
    }

    #[test]
    fn detects_alternate_screen_exit_in_chunk() {
        assert!(contains_alternate_exit(b"\x1b[?1049l"));
        assert!(contains_alternate_exit(b"abc\x1b[?1047ldef"));
        assert!(contains_alternate_exit(b"\x1b[?47l"));
        assert!(!contains_alternate_exit(b"\x1b[?1049h"));
    }

    #[test]
    fn detects_terminal_mode_changes_in_chunk() {
        assert!(contains_terminal_mode_change(b"\x1b[?1049h"));
        assert!(contains_terminal_mode_change(b"\x1b[?1049l"));
        assert!(contains_terminal_mode_change(b"\x1b[?1h"));
        assert!(contains_terminal_mode_change(b"\x1b[?1l"));
        assert!(contains_terminal_mode_change(b"\x1b[?1006h"));
        assert!(contains_terminal_mode_change(b"\x1b[?1000;1006h"));
        assert!(contains_terminal_mode_change(b"abc\x1b[?1002l"));
        assert!(!contains_terminal_mode_change(b"\x1b[31mred"));
        assert!(!contains_terminal_mode_change(b"\x1b[?25l"));
    }

    #[test]
    fn remote_repaint_burst_reconciles_after_final_screen_state() {
        let mut screen = Screen::new(Size { cols: 20, rows: 3 });
        let mut parser = vte::Parser::new();
        let mut terminal_queries = TerminalQueryParser::default();
        screen.feed(&mut parser, b"$ ");
        let mut predictor = predict::BasePredictor::new(true);
        predictor.on_key(key::KeyIntent::Printable('a'), &screen);

        let update = apply_remote_bytes(
            b"\x1b[H\x1b[2J\x1b[1;1H$ \x1b[1;3H",
            &mut screen,
            &mut parser,
            &mut predictor,
            &mut terminal_queries,
        );

        assert!(!update.terminal_mode_changed);
        assert!(update.terminal_responses.is_empty());
        assert_eq!(predictor.overlay.cells.len(), 1);
        assert_eq!(predictor.overlay.cells[0].cell.ch, 'a');
        assert_eq!(
            predictor.overlay.cursor,
            Some(screen::Cursor { row: 0, col: 3 })
        );
    }

    #[test]
    fn terminal_queries_are_answered_from_screen_state() {
        let mut screen = Screen::new(Size { cols: 80, rows: 24 });
        let mut parser = vte::Parser::new();
        let mut terminal_queries = TerminalQueryParser::default();
        let mut predictor = predict::BasePredictor::new(true);

        let update = apply_remote_bytes(
            b"\x1b[2;3H\x1b[6n\x1b[>c\x1b]10;?\x07\x1b]11;?\x1b\\",
            &mut screen,
            &mut parser,
            &mut predictor,
            &mut terminal_queries,
        );

        assert_eq!(
            update.terminal_responses,
            b"\x1b[2;3R\x1b[>0;0;0c\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:0000/0000/0000\x07"
        );
    }

    #[test]
    fn terminal_queries_are_stripped_from_raw_passthrough() {
        let stripped = strip_terminal_queries(
            b"before\x1b[6n\x1b[>c\x1b]10;?\x07\x1b]11;?\x1b\\after\x1b[31mred",
        );

        assert_eq!(stripped, b"beforeafter\x1b[31mred");
    }

    #[test]
    fn remote_coalescer_flushes_immediately_without_overlay() {
        let now = Instant::now();
        let mut coalescer = RemoteCoalescer::default();

        coalescer.push(vec![b"abc".to_vec()], now);

        assert!(coalescer.ready(now, false));
        assert_eq!(coalescer.take(), b"abc");
        assert!(coalescer.is_empty());
    }

    #[test]
    fn remote_coalescer_waits_for_quiet_while_overlay_pending() {
        let now = Instant::now();
        let mut coalescer = RemoteCoalescer::default();

        coalescer.push(vec![b"clear".to_vec()], now);
        assert!(!coalescer.ready(now + REMOTE_REPAINT_QUIET / 2, true));
        coalescer.push(vec![b"draw".to_vec()], now + REMOTE_REPAINT_QUIET / 2);

        assert!(!coalescer.ready(now + REMOTE_REPAINT_QUIET, true));
        assert!(coalescer.ready(now + REMOTE_REPAINT_QUIET * 2, true));
        assert_eq!(coalescer.take(), b"cleardraw");
    }

    #[test]
    fn remote_coalescer_caps_wait_while_overlay_pending() {
        let now = Instant::now();
        let mut coalescer = RemoteCoalescer::default();

        coalescer.push(vec![b"frame".to_vec()], now);

        assert!(coalescer.ready(now + REMOTE_REPAINT_MAX, true));
    }

    #[test]
    fn remote_coalescer_clear_drops_stale_resize_bytes() {
        let now = Instant::now();
        let mut coalescer = RemoteCoalescer::default();

        coalescer.push(vec![b"old frame".to_vec()], now);
        coalescer.clear();

        assert!(coalescer.is_empty());
        assert!(!coalescer.ready(now + REMOTE_REPAINT_MAX, true));
    }

    #[test]
    fn terminal_restore_moves_parent_prompt_below_content() {
        let mut screen = Screen::new(Size { cols: 20, rows: 5 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, b"hello");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };

        let sequence = terminal_restore_sequence(&screen, &overlay);

        assert!(sequence.contains("\x1b[2;1H\x1b[K"));
        assert!(!sequence.ends_with("\r\n"));
    }

    #[test]
    fn terminal_restore_accounts_for_overlay_below_confirmed_cursor() {
        let screen = Screen::new(Size { cols: 20, rows: 5 });
        let overlay = Overlay {
            enabled: true,
            cells: vec![predict::OverlayCell {
                pos: screen::Cursor { row: 2, col: 0 },
                cell: screen::Cell {
                    ch: 'x',
                    style: screen::Style::default(),
                },
                under: screen::Cell::default(),
                kind: predict::OverlayKind::Printable,
            }],
            cursor: None,
        };

        let sequence = terminal_restore_sequence(&screen, &overlay);

        assert!(sequence.contains("\x1b[4;1H\x1b[K"));
    }

    #[test]
    fn terminal_restore_scrolls_when_content_reaches_bottom() {
        let mut screen = Screen::new(Size { cols: 20, rows: 5 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, b"\x1b[5;1Hbottom");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };

        let sequence = terminal_restore_sequence(&screen, &overlay);

        assert!(sequence.contains("\x1b[5;1H\x1b[K\r\n"));
    }

    #[test]
    fn terminal_restore_uses_blank_cursor_row_after_remote_newline() {
        let mut screen = Screen::new(Size { cols: 20, rows: 5 });
        let mut parser = vte::Parser::new();
        screen.feed(&mut parser, b"command finished\r\nremote done\r\n");
        let overlay = Overlay {
            enabled: true,
            cells: Vec::new(),
            cursor: None,
        };

        let sequence = terminal_restore_sequence(&screen, &overlay);

        assert!(sequence.contains("\x1b[3;1H\x1b[K"));
        assert!(!sequence.contains("\x1b[4;1H\x1b[K"));
        assert!(!sequence.ends_with("\r\n"));
    }

    #[test]
    fn fallback_terminal_restore_leaves_parent_prompt_on_next_line() {
        assert!(fallback_terminal_restore_sequence().ends_with("\r\n"));
    }

    #[test]
    fn sizes_compare_exactly() {
        assert_eq!(Size { cols: 80, rows: 24 }, Size { cols: 80, rows: 24 });
        assert_ne!(Size { cols: 80, rows: 24 }, Size { cols: 81, rows: 24 });
    }

    #[cfg(unix)]
    #[test]
    fn parses_terminal_cursor_response() {
        assert_eq!(parse_cursor_response(b"\x1b[10;30R"), Some((29, 9)));
        assert_eq!(parse_cursor_response(b"nope"), None);
    }
}
