#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${ROOT}/dist"
LINUX_TARGET="${SLSH_LINUX_TARGET:-x86_64-unknown-linux-musl}"
WINDOWS_TARGET="${SLSH_WINDOWS_TARGET:-x86_64-pc-windows-msvc}"

mkdir -p "${DIST}"

echo "building Linux ${LINUX_TARGET}"
cargo build --release --locked --target "${LINUX_TARGET}"
install -m 0755 "${ROOT}/target/${LINUX_TARGET}/release/slsh" "${DIST}/slsh"

if ! command -v cargo-xwin >/dev/null 2>&1; then
    echo "cargo-xwin is required for local Windows MSVC builds." >&2
    echo "Install it with: cargo install cargo-xwin --locked" >&2
    exit 1
fi

echo "building Windows ${WINDOWS_TARGET}"
cargo xwin build --release --locked --target "${WINDOWS_TARGET}"
cp "${ROOT}/target/${WINDOWS_TARGET}/release/slsh.exe" "${DIST}/slsh.exe"

sha256sum "${DIST}/slsh" "${DIST}/slsh.exe"
