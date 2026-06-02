#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path


ASSETS = {
    "slsh-linux-x86_64": {
        "label": "Linux x86_64",
        "shell": "sh",
        "command": "sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-linux-x86_64 && echo '{sha}  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh",
    },
    "slsh-linux-aarch64": {
        "label": "Linux ARM64",
        "shell": "sh",
        "command": "sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-linux-aarch64 && echo '{sha}  /usr/local/bin/slsh' | sha256sum -c - && sudo chmod +x /usr/local/bin/slsh",
    },
    "slsh-macos-x86_64": {
        "label": "macOS x86_64",
        "shell": "sh",
        "command": "sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-macos-x86_64 && echo '{sha}  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh",
    },
    "slsh-macos-aarch64": {
        "label": "macOS ARM64",
        "shell": "sh",
        "command": "sudo curl -fsSLo /usr/local/bin/slsh https://github.com/KentoNishi/slsh/releases/latest/download/slsh-macos-aarch64 && echo '{sha}  /usr/local/bin/slsh' | shasum -a 256 -c - && sudo chmod +x /usr/local/bin/slsh",
    },
    "slsh-windows-x86_64.exe": {
        "label": "Windows x86_64 (PowerShell)",
        "shell": "powershell",
        "command": "iwr https://github.com/KentoNishi/slsh/releases/latest/download/slsh-windows-x86_64.exe -OutFile $env:LOCALAPPDATA\\Microsoft\\WindowsApps\\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\\Microsoft\\WindowsApps\\slsh.exe).Hash -ine '{sha}'){{exit 1}}",
    },
    "slsh-windows-aarch64.exe": {
        "label": "Windows ARM64 (PowerShell)",
        "shell": "powershell",
        "command": "iwr https://github.com/KentoNishi/slsh/releases/latest/download/slsh-windows-aarch64.exe -OutFile $env:LOCALAPPDATA\\Microsoft\\WindowsApps\\slsh.exe; if((Get-FileHash $env:LOCALAPPDATA\\Microsoft\\WindowsApps\\slsh.exe).Hash -ine '{sha}'){{exit 1}}",
    },
}

START = "<!-- INSTALL-COMMANDS:START -->"
END = "<!-- INSTALL-COMMANDS:END -->"


def read_checksums(path: Path) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) >= 2:
            checksums[parts[1]] = parts[0]
    missing = [asset for asset in ASSETS if asset not in checksums]
    if missing:
        raise SystemExit(f"missing checksums for: {', '.join(missing)}")
    return checksums


def section(checksums: dict[str, str]) -> str:
    lines = [START, ""]
    for asset, spec in ASSETS.items():
        lines.append(f"{spec['label']}:")
        lines.append("")
        lines.append(f"```{spec['shell']}")
        lines.append(spec["command"].format(sha=checksums[asset]))
        lines.append("```")
        lines.append("")
    lines.append(
        "Each command downloads the latest release asset, checks its SHA-256, "
        "and installs `slsh` into the platform PATH."
    )
    lines.append("")
    lines.append(END)
    return "\n".join(lines)


def replace_section(path: Path, replacement: str) -> None:
    text = path.read_text()
    start = text.index(START)
    end = text.index(END, start) + len(END)
    path.write_text(text[:start] + replacement + text[end:])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checksums", type=Path)
    parser.add_argument("docs", nargs="+", type=Path)
    args = parser.parse_args()

    replacement = section(read_checksums(args.checksums))
    for doc in args.docs:
        replace_section(doc, replacement)


if __name__ == "__main__":
    main()
