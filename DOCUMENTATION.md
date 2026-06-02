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

Download a release archive from:

```text
https://github.com/KentoNishi/slsh/releases/latest
```

Release assets are named by operating system and architecture:

| Platform | x64 | ARM64 |
| --- | --- | --- |
| Linux | `slsh-linux-x86_64.tar.gz` | `slsh-linux-aarch64.tar.gz` |
| macOS | `slsh-macos-x86_64.tar.gz` | `slsh-macos-aarch64.tar.gz` |
| Windows | `slsh-windows-x86_64.zip` | `slsh-windows-aarch64.zip` |

Each release also includes `SHA256SUMS`.

Linux:

```sh
ASSET=slsh-linux-x86_64.tar.gz
BASE=https://github.com/KentoNishi/slsh/releases/latest/download

curl -LO "$BASE/$ASSET"
curl -LO "$BASE/SHA256SUMS"
grep " $ASSET$" SHA256SUMS | sha256sum -c -

tar -xzf "$ASSET"
sudo install -m 0755 slsh /usr/local/bin/slsh
```

macOS:

```sh
ASSET=slsh-macos-aarch64.tar.gz
BASE=https://github.com/KentoNishi/slsh/releases/latest/download

curl -LO "$BASE/$ASSET"
curl -LO "$BASE/SHA256SUMS"
grep " $ASSET$" SHA256SUMS | shasum -a 256 -c -

tar -xzf "$ASSET"
sudo install -m 0755 slsh /usr/local/bin/slsh
```

Windows PowerShell:

```powershell
$Asset = "slsh-windows-x86_64.zip"
$Base = "https://github.com/KentoNishi/slsh/releases/latest/download"

Invoke-WebRequest "$Base/$Asset" -OutFile $Asset
Invoke-WebRequest "$Base/SHA256SUMS" -OutFile SHA256SUMS

$Expected = (Select-String -Path SHA256SUMS -Pattern " $Asset$").Line.Split(" ")[0]
$Actual = (Get-FileHash $Asset -Algorithm SHA256).Hash.ToLower()
if ($Actual -ne $Expected) { throw "checksum mismatch" }

New-Item -ItemType Directory "$HOME\bin" -Force | Out-Null
Expand-Archive $Asset -DestinationPath "$HOME\bin" -Force

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$HOME\bin*") {
  [Environment]::SetEnvironmentVariable("Path", "$UserPath;$HOME\bin", "User")
}
```

Open a new terminal after changing `PATH`.

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
