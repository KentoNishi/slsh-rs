<h1 align="center">
  <code>slsh</code>: ssh without keyboard lag
</h1>

<p align="center">
  <a href="https://github.com/KentoNishi/slsh/releases/latest">
    <img src="https://img.shields.io/github/v/release/KentoNishi/slsh?label=latest%20version&style=flat" alt="latest version">
  </a>
  <a href="LICENSE">
    <img src="https://img.shields.io/badge/license-MIT-green?style=flat" alt="license: MIT">
  </a>
  <img src="https://img.shields.io/badge/platforms-linux%20%7C%20macOS%20%7C%20windows-blue?style=flat" alt="platforms: linux, macOS, windows">
  <img src="https://img.shields.io/badge/architectures-x64%20%7C%20arm64-blue?style=flat" alt="architectures: x64, arm64">
  <a href="https://github.com/KentoNishi/slsh">
    <img src="https://img.shields.io/badge/source%20code-rust-orange?style=flat" alt="source code: rust">
  </a>
</p>

<p align="center">
  <a href="https://github.com/KentoNishi/slsh/releases/latest">View Releases</a>
  /
  <a href="DOCUMENTATION.md">View Documentation</a>
</p>

<p align="center">
  <img src="assets/slsh-promo.gif" alt="slsh latency compensation demo">
</p>

`slsh` is a drop-in SSH wrapper with local latency compensation for interactive
terminal sessions. It uses your system `ssh` binary, forwards real keypresses to
the remote side immediately, and renders temporary local predictions while
remote echo is still in flight.

## Installation

Download the archive for your platform from the latest release:

| Platform | x64 | ARM64 |
| --- | --- | --- |
| Linux | `slsh-linux-x86_64.tar.gz` | `slsh-linux-aarch64.tar.gz` |
| macOS | `slsh-macos-x86_64.tar.gz` | `slsh-macos-aarch64.tar.gz` |
| Windows | `slsh-windows-x86_64.zip` | `slsh-windows-aarch64.zip` |

Linux/macOS:

```sh
ASSET=slsh-linux-x86_64.tar.gz
BASE=https://github.com/KentoNishi/slsh/releases/latest/download

curl -LO "$BASE/$ASSET"
curl -LO "$BASE/SHA256SUMS"

grep " $ASSET$" SHA256SUMS | sha256sum -c -
tar -xzf "$ASSET"
sudo install -m 0755 slsh /usr/local/bin/slsh
```

On macOS, use `shasum -a 256 -c -` instead of `sha256sum -c -` if
`sha256sum` is not installed.

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

Build from source instead:

```sh
cargo build --release --locked
target/release/slsh user@host
```

## Usage

Use `slsh` like `ssh`:

```sh
slsh user@host
slsh -p 2222 user@host
slsh -i ~/.ssh/id_ed25519 user@host
slsh user@host htop
```

Disable prediction for one session:

```sh
slsh --slsh-no-predict user@host
```

For noninteractive SSH modes, `slsh` falls back to plain `ssh` passthrough.

Read [DOCUMENTATION.md](DOCUMENTATION.md) for environment variables,
passthrough details, predictors, and development commands.
