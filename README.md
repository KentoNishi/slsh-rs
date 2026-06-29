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
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-x86_64 && echo '812a2820cca3fd300d3656d3c4aed1cb2a90f74d0d3eb8a2e1172b037ea50cbe  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-aarch64 && echo '512a15de007d6ab6b4a667b49c5d9e47679000c1686fc00efd32d7f2ec0b5d10  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-x86_64 && echo '9c830aa2570a7b57fd4d744a8e0c0e5fd815bdcf5b75569bbacf04056f5b2b63  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-aarch64 && echo 'c65a133826f1718c2ee390a9f0ff0b48b88193bd03f8e089aa758e64bd503f25  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '09b521351141692be1f1ccc457cc045ba21812a28b68cfb69110f3e79860ae05'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '3289a6b79e635353b84ce1d766948429adb9ba51c839ffe28c372e1a3944e4a8'){exit 1}
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
