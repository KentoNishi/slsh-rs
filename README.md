<h1 align="center">
  <code>slsh</code>: ssh without keyboard lag
</h1>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat" alt="license: MIT">
  <a href="https://github.com/KentoNishi/slsh-rs"><img src="https://img.shields.io/badge/source%20code-rust-orange?style=flat" alt="source code: rust"></a>
  <img src="https://img.shields.io/badge/platforms-linux%20%7C%20macOS%20%7C%20windows-blue?style=flat" alt="platforms: linux, macOS, windows">
  <img src="https://img.shields.io/badge/architectures-x64%20%7C%20arm64-blue?style=flat" alt="architectures: x64, arm64">
</p>

<p align="center">
  <a href="https://slsh-rs.github.io">slsh-rs.github.io</a>
  /
  <a href="https://github.com/KentoNishi/slsh-rs/releases/latest">Download Latest Release</a>
  /
  <a href="DOCUMENTATION.md">Read Documentation</a>
</p>

<p align="center">
  <a href="https://github.com/KentoNishi/slsh-rs/releases/latest"><img src="https://img.shields.io/github/v/release/KentoNishi/slsh-rs?label=latest%20version&color=green&style=flat" alt="latest version"></a>
  <a href="https://github.com/KentoNishi/slsh-rs/actions/workflows/terminal-tests.yml"><img src="https://img.shields.io/github/actions/workflow/status/KentoNishi/slsh-rs/terminal-tests.yml?branch=master&label=terminal%20tests&style=flat" alt="terminal tests"></a>
  <a href="https://github.com/KentoNishi/slsh-rs/actions/workflows/build-binaries.yml"><img src="https://img.shields.io/github/actions/workflow/status/KentoNishi/slsh-rs/build-binaries.yml?label=release%20binaries&style=flat" alt="release binaries"></a>
</p>

<p align="center">
  <img src="assets/slsh-promo.gif" alt="slsh latency compensation demo">
</p>

`slsh` is a drop-in SSH wrapper with local latency compensation for interactive
terminal sessions. It uses your system `ssh` binary, forwards real keypresses to
the remote side immediately, and renders temporary local predictions while
remote echo is still in flight.

## Installation

<!-- INSTALL-COMMANDS:START -->

Linux x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-x86_64 && echo '6dc5ec2dd0b3168cfdf7245e1a8e49595dbd5912c6f420c0db39d428cf3d231c  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-aarch64 && echo '8dc6ac8bacca37d48094b5538dc59391d0b8678053589ad49218861c5fe4b530  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-x86_64 && echo 'fdd519b08ff5fdf4215eb85d77f70d0dbe4e7ec4760041e09f81eb18b7dc8e99  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-aarch64 && echo '977032346669a14ad5ece222cae2c435d4e58063d59bd6cbb7caf1cbde72ce86  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '1eca9f5418ab50bcb73ae59d83034d7214f3b804113507b2db60bcd887f02e87'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '32d205409e587e894a6be01437c256bc1e57af3a8fa68d1b4a14f2aede29d806'){exit 1}
```

Each command downloads the latest release asset, checks its SHA-256, and installs `slsh` into the platform PATH.

<!-- INSTALL-COMMANDS:END -->

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
