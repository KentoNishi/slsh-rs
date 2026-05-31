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
use screen::{Screen, Size};
use ssh_args::{LaunchMode, ParsedSshArgs};
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::time::Duration;
use transport::Transport;

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
    let mut screen = Screen::new(Size {
        cols: cols.max(1),
        rows: rows.max(1),
    });
    let mut parser = vte::Parser::new();
    let mut predictor = BasePredictor::new(parsed.slsh.predict);
    let mut renderer = Renderer::new();
    let mut transport = Transport::spawn(&ssh_args, cols.max(1), rows.max(1))?;
    let _terminal = TerminalGuard::enter()?;
    let mut stdout = io::stdout();
    let mut key_trace = KeyTrace::from_env();
    let mut pressed_keys = HashSet::new();
    let mut raw_synced = true;

    loop {
        let mut dirty = false;

        for chunk in transport.drain_chunks() {
            screen.feed(&mut parser, &chunk);
            predictor.reconcile(&screen);
            if raw_synced && predictor.overlay.cells.is_empty() {
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
                    let encoded = key::encode_key(key);
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
                        dirty = true;
                    }
                }
                Some(InputEvent::Resize(cols, rows)) => {
                    let cols = cols.max(1);
                    let rows = rows.max(1);
                    screen.resize(Size { cols, rows });
                    transport.resize(cols, rows)?;
                    renderer.invalidate();
                    predictor.clear();
                    dirty = true;
                    raw_synced = false;
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
        let _ = execute!(io::stdout(), ResetColor, Show);
        let _ = terminal::disable_raw_mode();
    }
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
}
