#!/usr/bin/env python3
import fcntl
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import termios
import time


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def main() -> int:
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    marker = b"SLSHLOOPBACKOK"
    command_marker = b"SLSHLOOPBACKCMDOK"
    output = run_loopback_shell(marker.decode())
    command_output = run_loopback_command(command_marker.decode())

    failed = []
    if marker not in output:
        failed.append("interactive shell marker")
    if command_marker not in command_output:
        failed.append("remote command marker")

    if failed:
        sys.stderr.write("loopback smoke failed: marker missing\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
        sys.stderr.write("\nCommand bytes:\n")
        sys.stderr.buffer.write(command_output)
        sys.stderr.write("\n")
        return 1

    print("loopback smoke passed")
    return 0


def run_loopback_shell(marker: str) -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "ignored-host"]
    env = os.environ.copy()
    env["SLSH_LOOPBACK"] = "1"
    env.setdefault("SHELL", "/bin/sh")

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    termios.tcflush(fd, termios.TCIOFLUSH)
    os.set_blocking(fd, False)
    fcntl_rows_cols(fd, 24, 80)

    output = b""
    sent_echo = False
    sent_exit = False
    ready_at = time.time() + 0.5
    deadline = time.time() + 10
    try:
        while time.time() < deadline:
            try:
                waited, _ = os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                return output
            if waited == pid:
                return output

            readable, _, _ = select.select([fd], [], [], 0.05)
            if readable:
                try:
                    chunk = os.read(fd, 4096)
                except BlockingIOError:
                    chunk = b""
                except OSError:
                    return output
                output += chunk

            if not sent_echo and time.time() >= ready_at:
                os.write(fd, f"echo {marker}\r".encode())
                sent_echo = True
            elif sent_echo and not sent_exit and marker.encode() in output:
                os.write(fd, b"exit\r")
                sent_exit = True
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def run_loopback_command(marker: str) -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "ignored-host", "echo", marker]
    env = os.environ.copy()
    env["SLSH_LOOPBACK"] = "1"
    env.setdefault("SHELL", "/bin/sh")
    return run_and_collect(argv, env)


def run_and_collect(argv: list[str], env: dict[str, str]) -> bytes:
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    os.set_blocking(fd, False)
    fcntl_rows_cols(fd, 24, 80)
    output = b""
    deadline = time.time() + 10
    try:
        while time.time() < deadline:
            try:
                waited, _ = os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                return output
            if waited == pid:
                return output

            readable, _, _ = select.select([fd], [], [], 0.05)
            if readable:
                try:
                    output += os.read(fd, 4096)
                except BlockingIOError:
                    pass
                except OSError:
                    return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def fcntl_rows_cols(fd: int, rows: int, cols: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


if __name__ == "__main__":
    raise SystemExit(main())
