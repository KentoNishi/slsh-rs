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
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-x86_64 && echo '7414422ca5a25581f55b80af3ac3633c0e843f8ab1d6e8b266b729d6f4aae1ab  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-aarch64 && echo 'ac5da8b37a5272f1b87a8bef3977247d9f835a6098908d9da8774bc2716089ae  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-x86_64 && echo '967f505bc969f6f710e26b85c9aa69c389b3835fc335359cffdb160a0ca9cc0b  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-aarch64 && echo '4ca390941b4a38352087ad5c94a4f774438e3b9ee255e450d91b31e58cb54103  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '3e9e0a9ebb108c965823da38783c5a7c2341b7f1fafa97b48bdbf8244e2853bd'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '9c69a57d5d3197daa134eff0a30406d5859fcb6542464bec4d9fc99a5ded1b42'){exit 1}
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
