use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, ExitStatus, MasterPty, PtySize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::ExitStatus as ProcessExitStatus;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const DEBUG_NETWORK_DELAY_ENV: &str = "SLSH_DELAY_MS";
const LOOPBACK_ENV: &str = "SLSH_LOOPBACK";

pub struct Transport {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writes: Sender<Vec<u8>>,
    chunks: Receiver<Vec<u8>>,
    reader_done: Receiver<()>,
    reader_finished: bool,
    exit_status: Option<ExitStatus>,
    exit_drain_deadline: Option<Instant>,
    debug_network_delay: Duration,
}

impl Transport {
    pub fn spawn_ssh(args: &[String], cols: u16, rows: u16) -> Result<Self> {
        let mut command = CommandBuilder::new("ssh");
        command.args(args);
        Transport::spawn(command, cols, rows, "ssh")
    }

    pub fn spawn_loopback(remote_command: &[String], cols: u16, rows: u16) -> Result<Self> {
        Transport::spawn(
            loopback_command(remote_command),
            cols,
            rows,
            "loopback shell",
        )
    }

    pub fn loopback_enabled() -> bool {
        flag_enabled(std::env::var_os(LOOPBACK_ENV))
    }

    fn spawn(command: CommandBuilder, cols: u16, rows: u16, context: &str) -> Result<Self> {
        let debug_network_delay = debug_network_delay();
        let pty = native_pty_system();
        let pair = pty
            .openpty(pty_size(cols, rows))
            .with_context(|| format!("failed to open {context} pty"))?;
        let mut command = command;
        if std::env::var_os("TERM")
            .and_then(|term| term.into_string().ok())
            .filter(|term| !term.is_empty() && term != "dumb")
            .is_none()
        {
            command.env("TERM", "xterm-256color");
        }

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("failed to spawn {context}"))?;
        let reader = pair
            .master
            .try_clone_reader()
            .with_context(|| format!("failed to open {context} pty reader"))?;
        let writer = pair
            .master
            .take_writer()
            .with_context(|| format!("failed to open {context} pty writer"))?;
        let (tx, rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let (write_tx, write_rx) = mpsc::channel();
        let _ = thread::spawn(move || {
            read_stream(reader, tx, debug_network_delay);
            let _ = done_tx.send(());
        });
        let _ = thread::spawn(move || write_stream(writer, write_rx, debug_network_delay));

        Ok(Self {
            child,
            master: pair.master,
            writes: write_tx,
            chunks: rx,
            reader_done: done_rx,
            reader_finished: false,
            exit_status: None,
            exit_drain_deadline: None,
            debug_network_delay,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.writes
            .send(bytes.to_vec())
            .context("failed to queue transport write")
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        sleep_debug_delay(self.debug_network_delay);
        self.master
            .resize(pty_size(cols, rows))
            .context("failed to resize transport pty")
    }

    pub fn drain_chunks(&mut self) -> Vec<Vec<u8>> {
        let mut chunks = Vec::new();
        while let Ok(chunk) = self.chunks.try_recv() {
            chunks.push(chunk);
        }
        chunks
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        if self.exit_status.is_none() {
            if let Some(status) = self
                .child
                .try_wait()
                .context("failed to poll transport child")?
            {
                self.exit_status = Some(status);
                self.exit_drain_deadline =
                    Some(Instant::now() + self.debug_network_delay + Duration::from_millis(200));
            }
        }

        if self.exit_status.is_some() && !self.reader_finished {
            let mut reader_just_finished = false;
            match self.reader_done.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => {
                    self.reader_finished = true;
                    reader_just_finished = true;
                }
                Err(TryRecvError::Empty) => {}
            }
            if reader_just_finished {
                return Ok(None);
            }
            if !self.reader_finished && should_wait_for_reader(self.exit_drain_deadline) {
                return Ok(None);
            }
        }

        Ok(self.exit_status.take())
    }
}

#[cfg(windows)]
fn should_wait_for_reader(deadline: Option<Instant>) -> bool {
    deadline.is_none_or(|deadline| Instant::now() < deadline)
}

#[cfg(not(windows))]
fn should_wait_for_reader(_deadline: Option<Instant>) -> bool {
    true
}

pub fn std_exit_code(status: ProcessExitStatus) -> i32 {
    status.code().unwrap_or(255)
}

pub fn pty_exit_code(status: ExitStatus) -> i32 {
    status.exit_code().min(255) as i32
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn debug_network_delay() -> Duration {
    parse_debug_network_delay(std::env::var_os(DEBUG_NETWORK_DELAY_ENV))
}

fn parse_debug_network_delay(value: Option<OsString>) -> Duration {
    value
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_default()
}

fn sleep_debug_delay(delay: Duration) {
    if !delay.is_zero() {
        thread::sleep(delay);
    }
}

fn write_stream(
    mut stream: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    debug_network_delay: Duration,
) {
    while let Ok(bytes) = rx.recv() {
        sleep_debug_delay(debug_network_delay);
        if stream
            .write_all(&bytes)
            .and_then(|_| stream.flush())
            .is_err()
        {
            break;
        }
    }
}

fn flag_enabled(value: Option<OsString>) -> bool {
    value
        .and_then(|value| value.into_string().ok())
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !value.is_empty() && value != "0" && value != "false" && value != "off" && value != "no"
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn loopback_command(remote_command: &[String]) -> CommandBuilder {
    let shell = loopback_shell();
    let mut command = CommandBuilder::new(&shell);
    if !remote_command.is_empty() {
        command.arg("-lc");
        command.arg(shell_join(remote_command));
    }
    command
}

#[cfg(windows)]
fn loopback_command(remote_command: &[String]) -> CommandBuilder {
    let shell = loopback_shell();
    let mut command = CommandBuilder::new(&shell);
    if !remote_command.is_empty() {
        command.arg("/C");
        command.arg(windows_shell_join(remote_command));
    }
    command
}

#[cfg(windows)]
fn loopback_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
}

#[cfg(not(windows))]
fn loopback_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

#[cfg(not(windows))]
fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(not(windows))]
fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".into();
    }
    if arg
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"_+-./:=@%".contains(&byte))
    {
        return arg.into();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn windows_shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| windows_shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn windows_shell_quote(arg: &str) -> String {
    if arg.is_empty() || arg.bytes().any(|byte| byte.is_ascii_whitespace()) {
        format!("\"{}\"", arg.replace('"', "\"\""))
    } else {
        arg.into()
    }
}

fn read_stream(mut stream: impl Read, tx: Sender<Vec<u8>>, debug_network_delay: Duration) {
    let mut buffer = [0; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(len) => {
                sleep_debug_delay(debug_network_delay);
                if tx.send(buffer[..len].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_debug_network_delay() {
        assert_eq!(parse_debug_network_delay(None), Duration::ZERO);
        assert_eq!(
            parse_debug_network_delay(Some(OsString::from("25"))),
            Duration::from_millis(25)
        );
        assert_eq!(
            parse_debug_network_delay(Some(OsString::from("nope"))),
            Duration::ZERO
        );
        assert_eq!(
            parse_debug_network_delay(Some(OsString::from("-1"))),
            Duration::ZERO
        );
    }

    #[test]
    fn parses_loopback_flag() {
        assert!(!flag_enabled(None));
        assert!(!flag_enabled(Some(OsString::from(""))));
        assert!(!flag_enabled(Some(OsString::from("0"))));
        assert!(!flag_enabled(Some(OsString::from("false"))));
        assert!(!flag_enabled(Some(OsString::from("off"))));
        assert!(!flag_enabled(Some(OsString::from("no"))));
        assert!(flag_enabled(Some(OsString::from("1"))));
        assert!(flag_enabled(Some(OsString::from("true"))));
        assert!(flag_enabled(Some(OsString::from("yes"))));
    }

    #[cfg(not(windows))]
    #[test]
    fn quotes_loopback_shell_command() {
        assert_eq!(
            shell_join(&["echo".into(), "hello world".into(), "that's".into()]),
            "echo 'hello world' 'that'\\''s'"
        );
    }
}
