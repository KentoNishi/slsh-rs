<h1 align="center">
  <code>slsh</code>: ssh without keyboard lag
</h1>

<p align="center">
  <a href="https://github.com/KentoNishi/slsh/releases/latest">
    <img src="https://img.shields.io/github/v/release/KentoNishi/slsh?label=latest%20version&style=flat" alt="latest version">
  </a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat" alt="license: MIT">
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

<!-- INSTALL-COMMANDS:START -->

Linux x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-linux-x86_64 && echo 'cd583d82d98b49552c46ea0c0f49428be2f2bf29c48dda1940cc8e72aa8f28aa  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

Linux ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-linux-aarch64 && echo '06cd28aa615c7c2609561047afe2f3b0d3c2d7335e6bbe2a82dd1afb3d6b9540  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS x86_64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-macos-x86_64 && echo '3b46871ae572501a455585d6b44aecd0e429e965ba2a2be742071a02145b1a9a  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

macOS ARM64:

```sh
sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-macos-aarch64 && echo 'd17ec9f5c06a3b7bbd7738b372732de08ef6b3378a1bdd5343573bada9ebf2a2  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh
```

Windows x86_64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '14548b396ab4349e0799c484d50c5213bc3d66f825b810f3defd76e9ef3447cc'){exit 1}
```

Windows ARM64 (PowerShell):

```powershell
iwr https://github.com/KentoNishi/slsh/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\Microsoft\WindowsApps\slsh.exe).Hash -ine '5950caf2a26c3c874940f7886aa151ea9fadf357eb9b7ab2baba0daf61022499'){exit 1}
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
