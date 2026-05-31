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
    delayed_echo_output = run_delayed_local_echo()
    seeded_cursor_output = run_seeded_cursor_overlay()

    failed = []
    if marker not in output:
        failed.append("interactive shell marker")
    if command_marker not in command_output:
        failed.append("remote command marker")
    if b"SLSHLAG" not in delayed_echo_output:
        failed.append("delayed local echo")
    if b"\x1b[10;" not in seeded_cursor_output or b"\x1b[1;" in seeded_cursor_output:
        failed.append("startup cursor seeded overlay")

    if failed:
        sys.stderr.write("loopback smoke failed: marker missing\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
        sys.stderr.write("\nCommand bytes:\n")
        sys.stderr.buffer.write(command_output)
        sys.stderr.write("\nDelayed echo bytes:\n")
        sys.stderr.buffer.write(delayed_echo_output)
        sys.stderr.write("\nSeeded cursor bytes:\n")
        sys.stderr.buffer.write(seeded_cursor_output)
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


def run_delayed_local_echo() -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "ignored-host"]
    env = os.environ.copy()
    env["SLSH_LOOPBACK"] = "1"
    env["SLSH_DELAY_MS"] = "1000"
    env.setdefault("SHELL", "/bin/sh")

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    termios.tcflush(fd, termios.TCIOFLUSH)
    os.set_blocking(fd, False)
    fcntl_rows_cols(fd, 24, 80)

    output = b""
    sent = False
    send_at = time.time() + 1.25
    capture_until = None
    deadline = time.time() + 4
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.02)
            if readable:
                try:
                    output += os.read(fd, 4096)
                except BlockingIOError:
                    pass
                except OSError:
                    return output

            if not sent and time.time() >= send_at:
                os.write(fd, b"SLSHLAG")
                sent = True
                capture_until = time.time() + 0.25
            if capture_until is not None and time.time() >= capture_until:
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def run_seeded_cursor_overlay() -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "ignored-host"]
    env = os.environ.copy()
    env["SLSH_LOOPBACK"] = "1"
    env["SLSH_DELAY_MS"] = "1000"
    env.setdefault("SHELL", "/bin/sh")

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    termios.tcflush(fd, termios.TCIOFLUSH)
    os.set_blocking(fd, False)
    fcntl_rows_cols(fd, 10, 50)

    output = b""
    answered_cursor_query = False
    sent = False
    sent_at = 0.0
    deadline = time.time() + 6
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.02)
            if readable:
                try:
                    output += os.read(fd, 4096)
                except BlockingIOError:
                    pass
                except OSError:
                    return output

                if not answered_cursor_query and b"\x1b[6n" in output:
                    os.write(fd, b"\x1b[10;1R")
                    answered_cursor_query = True

            if answered_cursor_query and not sent and (b"$" in output or b"#" in output):
                os.write(fd, b"Z")
                sent = True
                sent_at = time.time()
            if sent and time.time() - sent_at >= 0.25:
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


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
