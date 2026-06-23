# slsh documentation

`slsh` is a drop-in SSH wrapper with local latency compensation for interactive
terminal sessions.

It uses the system `ssh` binary, keeps a local model of the remote terminal,
and renders temporary local predictions while remote echo is still in flight.
Remote output is always the source of truth.

## Basic usage

Use `slsh` like `ssh`:

```sh
slsh user@host
slsh -p 2222 user@host
slsh -i ~/.ssh/id_ed25519 user@host
slsh user@host htop
```

Most SSH arguments are forwarded directly to `ssh`.

For interactive terminal sessions, `slsh` starts an SSH session in a local PTY,
forwards keys immediately, and draws predicted printable input at the current
cursor.

For noninteractive sessions, `slsh` runs plain `ssh` passthrough.

## Install

<!-- INSTALL-COMMANDS:START -->

Linux x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-x86_64 && echo 'df12e475bcab5b29465f05a89599b37ebe923d49004050ef27f51369c375b378  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-aarch64 && echo '1d8617665fa0ce529f1cef484154d96750078046f1eebc9a074144ced41e23ac  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-x86_64 && echo 'd63f5f13f14ea04836add237e72a211fd5a3066614e3d64f3342aa7932f3dffe  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-aarch64 && echo 'c23f62a9fcd06fc428d632b9f606f788d40206473765c109f78df8211a112b0c  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine 'e472db7c6e290dfdeef678ba8aa73e41de00a31e34f36d0e93f0c05d6094d23f'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '682e296d709fd940295624ed596cf87588cb98d924429271ef228e48d9dd5c83'){exit 1}
```

Each command downloads the latest release asset, checks its SHA-256, and installs `slsh` into the platform PATH.

<!-- INSTALL-COMMANDS:END -->

Build from source:

```sh
cargo build --release --locked
target/release/slsh user@host
```

## Options

Disable local prediction:

```sh
slsh --slsh-no-predict user@host
```

`--slsh-no-predict` is consumed by `slsh`; all other SSH arguments are forwarded.

## Passthrough mode

`slsh` falls back to plain `ssh` when there is no interactive terminal to
compose.

Examples include:

- stdin or stdout is not a TTY;
- no host was provided;
- `ssh -N`;
- `ssh -T`;
- `ssh -G`;
- `ssh -V`;
- `ssh -s`;
- `ssh -n`;
- `ssh -f`;
- `ssh -W ...`;
- `ssh -O ...`.

Remote commands can still use the compositor when SSH allocates a terminal:

```sh
slsh user@host vim
slsh user@host htop
```

## Environment variables

Add artificial transport delay:

```sh
SLSH_DELAY_MS=100 slsh user@host
```

Run against the local shell instead of SSH:

```sh
SLSH_LOOPBACK=1 SLSH_DELAY_MS=100 target/release/slsh ignored-host
```

Write key forwarding diagnostics:

```sh
SLSH_KEY_LOG=/tmp/slsh-keys.log slsh user@host
```

Select a compiled-in predictor:

```sh
SLSH_PREDICTOR=example-application slsh user@host
```

The example predictor currently delegates to the base predictor. It is included
as a minimal pattern for application-specific predictors.

## Prediction model

Prediction is local and disposable.

When printable input is typed, `slsh` sends the real key bytes to SSH
immediately and draws a faint local overlay. When remote output arrives, the
overlay is reconciled against the confirmed screen.

The overlay is cleared when remote output contradicts it, when cursor movement
makes the prediction unsafe, or when input is nonlinear.

Enter keeps the current predicted command visible until remote echo confirms or
contradicts it. `slsh` does not predict command output.

## Rendering model

`slsh` keeps:

- confirmed screen state;
- overlay state;
- the last locally drawn frame.

Each render composes confirmed state plus overlay state, diffs that composed
frame against the last frame, and writes only the changed terminal cells.

Full redraws are reserved for startup, resize, and explicit recovery.

## Predictors

Predictor code lives in `src/predict`.

The base predictor is in:

```text
src/predict/base.rs
```

Application predictors live in:

```text
src/predict/applications/
```

Predictors implement `PredictorPlugin`:

```rust
pub trait PredictorPlugin {
    fn name(&self) -> &'static str;
    fn overlay(&self) -> &Overlay;
    fn on_key(&mut self, intent: KeyIntent, screen: &Screen);
    fn reconcile(&mut self, screen: &Screen);
    fn clear(&mut self);
}
```

Keep predictors small, deterministic, and compiled in.

## Development

Run tests:

```sh
cargo test --locked
```

Run the PTY smoke test:

```sh
python3 scripts/smoke_local_pty.py
```

Run the loopback smoke test:

```sh
python3 scripts/smoke_loopback.py
```

If a local sshd is available:

```sh
python3 scripts/smoke_local_sshd.py
```

Before sending a change, run:

```sh
cargo fmt
cargo test --locked
python3 scripts/smoke_local_pty.py
python3 scripts/smoke_loopback.py
```

Changes to input handling, prediction, terminal parsing, rendering, PTY
transport, or SSH launch behavior should include at least one smoke test run.
