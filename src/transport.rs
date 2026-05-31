use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};

pub struct Transport {
    child: Child,
    stdin: ChildStdin,
    chunks: Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
}

impl Transport {
    pub fn spawn(args: &[String]) -> Result<Self> {
        let mut child = Command::new("ssh")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to spawn ssh")?;

        let stdin = child.stdin.take().context("failed to open ssh stdin")?;
        let stdout = child.stdout.take().context("failed to open ssh stdout")?;
        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut stdout = stdout;
            let mut buffer = [0; 8192];
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(len) => {
                        if tx.send(buffer[..len].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            chunks: rx,
            reader: Some(reader),
        })
    }

    pub fn write_command(&mut self, command: &str) -> Result<()> {
        self.stdin
            .write_all(command.as_bytes())
            .context("failed to write tmux command")?;
        self.stdin.flush().context("failed to flush tmux command")
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

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    pub fn wait(mut self) -> Result<ExitStatus> {
        let status = self.child.wait().context("failed waiting for ssh child")?;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        Ok(status)
    }
}
