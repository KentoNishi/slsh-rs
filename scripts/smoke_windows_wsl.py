#!/usr/bin/env python3
import fcntl
import os
import pty
import select
import signal
import stat
import struct
import subprocess
import sys
import termios
import time


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
HOST = os.environ.get("SLSH_WINDOWS_HOST", "wsl")


def main() -> int:
    exe = windows_binary()
    make_executable(exe)
    marker = b"ZZSLSHWINOK"
    output = run_slsh(exe, marker)

    if output.count(marker) < 2:
        sys.stderr.write("windows WSL smoke failed: command did not execute\n")
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
        sys.stderr.write("\n")
        return 1

    print("windows WSL smoke passed")
    return 0


def windows_binary() -> str:
    if os.environ.get("SLSH_EXE"):
        return os.environ["SLSH_EXE"]

    exe = os.path.join(ROOT, "target", "release", "slsh.exe")
    if os.path.exists(exe):
        return exe

    raise SystemExit("set SLSH_EXE to the Windows slsh.exe path")


def make_executable(path: str) -> None:
    mode = os.stat(path).st_mode
    os.chmod(path, mode | stat.S_IXUSR)


def run_slsh(exe: str, marker: bytes) -> bytes:
    argv = [
        exe,
        "--slsh-no-predict",
        HOST,
        "bash",
        "--noprofile",
        "--norc",
        "-i",
    ]

    pid, fd = pty.fork()
    if pid == 0:
        os.execv(argv[0], argv)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    output = b""
    cpr_replies = 0
    stage = "wait_prompt"
    deadline = time.time() + 30

    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.05)
            if readable:
                try:
                    chunk = os.read(fd, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                output += chunk
                cpr_replies = answer_cursor_position_requests(fd, output, cpr_replies)

            if stage == "wait_prompt" and prompt_seen(output):
                os.write(fd, b"echo ZZSLSHWINx\x7fOK\r")
                stage = "wait_marker"
            elif stage == "wait_marker" and output.count(marker) >= 2:
                os.write(fd, b"exit\r")
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def answer_cursor_position_requests(fd: int, output: bytes, replies: int) -> int:
    requested = output.count(b"\x1b[6n")
    while replies < requested:
        os.write(fd, b"\x1b[1;1R")
        replies += 1
    return replies


def prompt_seen(output: bytes) -> bool:
    return b"bash" in output or b"#" in output or b"$" in output


if __name__ == "__main__":
    raise SystemExit(main())
