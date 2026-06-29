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
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-x86_64 && echo 'abf11c3ffdc599bc5ce4d11d38c6ecac5c73d8d8a247352e2425945afd6966ed  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-linux-aarch64 && echo '0580c53d6aa390ad0ed699bde8fb1e012334eedd5f5ef89634e5a40c45ba157b  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-x86_64 && echo 'c9f18fe5c00ee16d27f485c87df2b11002056ebb41e164638960f84a715f84ec  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-macos-aarch64 && echo '9846d798fa36554a3677f86591ed032fff0883dbe1139a183b1e69b94161f927  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine 'e2ea0058d57d2ee8ac23fc5993ecbb75b699c10b68a198680914f67084b6cf8a'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh-rs/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '8735456a99df775d174c2546915d2e886bdbbe458ebd5f6bd4aa860daf75abf3'){exit 1}
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
