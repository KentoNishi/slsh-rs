use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, ExitStatus, MasterPty, PtySize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::ExitStatus as ProcessExitStatus;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

const DEBUG_NETWORK_DELAY_ENV: &str = "SLSH_DELAY_MS";

pub struct Transport {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    chunks: Receiver<Vec<u8>>,
    debug_network_delay: Duration,
}

impl Transport {
    pub fn spawn(args: &[String], cols: u16, rows: u16) -> Result<Self> {
        let debug_network_delay = debug_network_delay();
        let pty = native_pty_system();
        let pair = pty
            .openpty(pty_size(cols, rows))
            .context("failed to open ssh pty")?;
        let mut command = CommandBuilder::new("ssh");
        command.args(args);
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
            .context("failed to spawn ssh")?;
        let reader = pair
            .master
            .try_clone_reader()
            .context("failed to open ssh pty reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("failed to open ssh pty writer")?;
        let (tx, rx) = mpsc::channel();
        let _ = thread::spawn(move || read_stream(reader, tx, debug_network_delay));

        Ok(Self {
            child,
            master: pair.master,
            writer,
            chunks: rx,
            debug_network_delay,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        sleep_debug_delay(self.debug_network_delay);
        self.writer
            .write_all(bytes)
            .context("failed to write ssh pty")?;
        self.writer.flush().context("failed to flush ssh pty")
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        sleep_debug_delay(self.debug_network_delay);
        self.master
            .resize(pty_size(cols, rows))
            .context("failed to resize ssh pty")
    }

    pub fn drain_chunks(&mut self) -> Vec<Vec<u8>> {
        let mut chunks = Vec::new();
        while let Ok(chunk) = self.chunks.try_recv() {
            chunks.push(chunk);
        }
        chunks
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child.try_wait().context("failed to poll ssh child")
    }
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
}
