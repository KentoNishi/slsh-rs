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
use predict::BasePredictor;
use render::Renderer;
use screen::{ActiveBuffer, Screen, Size};
use ssh_args::{LaunchMode, ParsedSshArgs};
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use transport::Transport;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::time::Instant;

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
    let mut predictor = BasePredictor::new(parsed.slsh.predict);
    let mut renderer = Renderer::new();
    let mut transport = if Transport::loopback_enabled() {
        Transport::spawn_loopback(&parsed.remote_command, cols.max(1), rows.max(1))?
    } else {
        Transport::spawn_ssh(&ssh_args, cols.max(1), rows.max(1))?
    };
    let _terminal = TerminalGuard::enter()?;
    let mut stdout = io::stdout();
    let mut key_trace = KeyTrace::from_env();
    let mut pressed_keys = HashSet::new();
    let mut raw_synced = true;
    #[cfg(not(windows))]
    let mut mouse_protocol = key::MouseProtocol::default();

    loop {
        let mut dirty = false;

        for chunk in transport.drain_chunks() {
            let before_active = screen.active();
            let before_application_cursor = screen.application_cursor_keys();
            for byte in &chunk {
                screen.feed(&mut parser, std::slice::from_ref(byte));
                predictor.reconcile(&screen);
            }
            #[cfg(not(windows))]
            mouse_protocol.feed(&chunk);
            let left_alternate = (before_active == ActiveBuffer::Alternate
                && screen.active() == ActiveBuffer::Primary)
                || contains_alternate_exit(&chunk);
            let terminal_mode_changed = left_alternate
                || before_active != screen.active()
                || before_application_cursor != screen.application_cursor_keys()
                || contains_terminal_mode_change(&chunk);
            if terminal_mode_changed {
                stdout
                    .write_all(&chunk)
                    .context("failed to render ssh output")?;
                if left_alternate {
                    stdout
                        .write_all(b"\x1b[0m")
                        .context("failed to reset terminal style")?;
                }
                stdout.flush().context("failed to flush ssh output")?;
                predictor.clear();
                if left_alternate {
                    screen.reset_style();
                }
                renderer.sync_to_terminal(&screen, &predictor.overlay);
                raw_synced = true;
                dirty = false;
            } else if raw_synced && predictor.overlay.cells.is_empty() {
                stdout
                    .write_all(&chunk)
                    .context("failed to render ssh output")?;
                stdout.flush().context("failed to flush ssh output")?;
                renderer.sync_to_terminal(&screen, &predictor.overlay);
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
                        predictor.on_key(encoded.intent, &screen);
                        key_trace.log(format_args!(
                            "predict cursor {:?} overlay {} overlay_cursor {:?}",
                            screen.cursor(),
                            predictor.overlay.cells.len(),
                            predictor.overlay.cursor
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
                    screen.resize(Size { cols, rows });
                    transport.resize(cols, rows)?;
                    renderer.invalidate();
                    predictor.clear();
                    dirty = true;
                    raw_synced = false;
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
            let output = renderer.render(&screen, &predictor.overlay);
            key_trace.log(format_args!("render bytes {}", output.len()));
            stdout
                .write_all(output.as_bytes())
                .context("failed to render terminal")?;
            stdout.flush().context("failed to flush terminal")?;
            raw_synced = predictor.overlay.cells.is_empty();
        }

        if let Some(status) = transport.try_wait()? {
            return Ok(transport::pty_exit_code(status));
        }

        if !dirty {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
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

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        #[cfg(windows)]
        let _ = crossterm::ansi_support::supports_ansi();

        terminal::enable_raw_mode().context("failed to enable raw mode")?;
        Ok(Self)
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
        let _ = execute!(
            io::stdout(),
            crossterm::style::Print(terminal_restore_sequence()),
            ResetColor,
            Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn terminal_restore_sequence() -> &'static str {
    "\x1b[0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1049l\r\n"
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
    fn terminal_restore_leaves_parent_prompt_on_next_line() {
        assert!(terminal_restore_sequence().ends_with("\r\n"));
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
