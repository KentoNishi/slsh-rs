mod predict;
mod render;
mod screen;
mod ssh_args;
mod tmux;
mod transport;

use anyhow::{Context, Result};
use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::{Print, ResetColor};
use crossterm::terminal;
use predict::BasePredictor;
use render::Renderer;
use screen::{Screen, Size};
use ssh_args::{LaunchMode, ParsedSshArgs};
use std::collections::HashSet;
use std::env;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tmux::ControlEvent;
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
    Ok(exit_code(status))
}

fn run_compositor(parsed: ParsedSshArgs) -> Result<i32> {
    let host = parsed.host.as_ref().context("missing ssh host")?;
    let session_name = make_session_name();
    let launcher = if parsed.remote_command.is_empty() {
        tmux::shell_launcher(&session_name)
    } else {
        tmux::command_launcher(&session_name, &parsed.remote_command)
    };

    let mut ssh_args = parsed.ssh_options.clone();
    ssh_args.push("-tt".into());
    ssh_args.push(host.clone());
    ssh_args.push(launcher);

    let (cols, rows) = terminal::size().context("failed to read terminal size")?;
    let mut screen = Screen::new(Size {
        cols: cols.max(1),
        rows: rows.max(1),
    });
    let mut parser = vte::Parser::new();
    let mut predictor = BasePredictor::new(parsed.slsh.predict);
    let mut renderer = Renderer::new();
    let mut transport = Transport::spawn(&ssh_args)?;
    let mut pending_lines = wait_for_control_start(&mut transport)?;
    let _terminal = TerminalGuard::enter()?;
    let mut active_pane: Option<String> = Some(session_name);
    let mut stdout = io::stdout();
    let mut done = false;
    let mut key_trace = KeyTrace::from_env();
    let mut pressed_keys = HashSet::new();

    while !done {
        let mut dirty = false;

        pending_lines.extend(transport.drain_lines());
        for line in pending_lines.drain(..) {
            match tmux::parse_control_line(&line) {
                Some(ControlEvent::Output { pane, bytes }) => {
                    active_pane = Some(pane);
                    screen.feed(&mut parser, &bytes);
                    predictor.reconcile(&screen);
                    dirty = true;
                }
                Some(ControlEvent::Exit) => done = true,
                Some(ControlEvent::Error(error)) => {
                    transport.kill();
                    anyhow::bail!("tmux error: {error}");
                }
                Some(ControlEvent::Notification(_)) | None => {}
            }
        }

        while event::poll(Duration::from_millis(1)).context("failed to poll terminal input")? {
            match event::read().context("failed to read terminal input")? {
                Event::Key(key) => {
                    let mapped = tmux::key_to_tmux(key, active_pane.as_deref());
                    let should_forward = match key.kind {
                        KeyEventKind::Press | KeyEventKind::Repeat => {
                            pressed_keys.insert(key_fingerprint(key));
                            true
                        }
                        KeyEventKind::Release => !pressed_keys.remove(&key_fingerprint(key)),
                    };
                    key_trace.log(format_args!(
                        "key {:?} pane {:?} forwarded {should_forward} command {:?} intent {:?}",
                        key,
                        active_pane,
                        mapped.command.as_deref(),
                        mapped.intent
                    ));
                    if should_forward {
                        if let Some(command) = mapped.command {
                            transport.write_command(&command)?;
                        }
                        predictor.on_key(mapped.intent, &screen);
                        dirty = true;
                    }
                }
                Event::Resize(cols, rows) => {
                    screen.resize(Size {
                        cols: cols.max(1),
                        rows: rows.max(1),
                    });
                    renderer.invalidate();
                    predictor.clear();
                    transport.write_command(&tmux::resize_command(cols.max(1), rows.max(1)))?;
                    dirty = true;
                }
                Event::Paste(text) => {
                    let command = tmux::paste_to_tmux(&text, active_pane.as_deref());
                    key_trace.log(format_args!(
                        "paste {:?} pane {:?} command {:?}",
                        text, active_pane, command
                    ));
                    if !command.is_empty() {
                        transport.write_command(&command)?;
                    }
                    predictor.clear();
                    dirty = true;
                }
                _ => {}
            }
        }

        if dirty {
            let output = renderer.render(&screen, &predictor.overlay);
            stdout
                .write_all(output.as_bytes())
                .context("failed to render terminal")?;
            stdout.flush().context("failed to flush terminal")?;
        }

        if let Some(status) = transport.try_wait()? {
            return Ok(exit_code(status));
        }

        if !dirty {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    Ok(exit_code(transport.wait()?))
}

fn wait_for_control_start(transport: &mut Transport) -> Result<Vec<String>> {
    loop {
        let lines = transport.drain_lines();
        if lines.iter().any(|line| tmux::is_control_line(line)) {
            return Ok(lines);
        }
        if let Some(status) = transport.try_wait()? {
            anyhow::bail!("ssh exited before tmux control mode started with status {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn make_session_name() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("slsh-{}-{millis}", std::process::id())
}

fn exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(255)
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
        execute!(io::stdout(), Print("\x1b[?7l\x1b[?25l\x1b[2J\x1b[H"))
            .context("failed to prepare terminal")?;
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
        let _ = execute!(io::stdout(), Print("\x1b[?7h"), ResetColor, Show);
        let _ = terminal::disable_raw_mode();
    }
}
