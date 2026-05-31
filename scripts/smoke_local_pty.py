#!/usr/bin/env python3
import fcntl
import os
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def main() -> int:
    require("tmux")
    require("cargo")

    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    with tempfile.TemporaryDirectory() as tmp:
        fake_ssh = os.path.join(tmp, "ssh")
        fake_ssh_log = os.path.join(tmp, "ssh.stdin")
        write_fake_ssh(fake_ssh)

        env = os.environ.copy()
        env["PATH"] = f"{tmp}{os.pathsep}{env['PATH']}"
        env["FAKE_SSH_LOG"] = fake_ssh_log

        startup_output = run_startup_slsh(env)
        output = run_slsh(env)
        if os.path.exists(fake_ssh_log):
            with open(fake_ssh_log, "rb") as handle:
                control_input = handle.read()
        else:
            control_input = b""

    startup_screen = reduce_terminal(startup_output)
    checks = {
        "startup stderr warning visible": "Warning fake ssh stderr" in startup_screen,
        "startup login preamble visible": "Welcome fake ssh login" in startup_screen,
        "startup bootstrap command absent": "exec tmux -CC" not in startup_screen,
        "startup prompt not doubled": prompt_line_count(startup_screen) == 1,
        "echo command output": b"hello" in output,
        "red sgr output": b"\x1b[31mred" in output,
        "256-color sgr output": b"\x1b[38;5;196mhot" in output,
        "dec special graphics output": "┌─┐".encode() in output,
        "enter key forwarded": b"send-keys" in control_input and b"-H 0d" in control_input,
        "backspace key forwarded": b"send-keys" in control_input and b"BSpace" in control_input,
        "ctrl-c forwarded": b"send-keys" in control_input and b"C-c" in control_input,
        "left key forwarded": b"send-keys" in control_input and b"Left" in control_input,
        "login preamble rendered": b"Welcome fake ssh login" in output,
        "bootstrap command hidden": b"exec tmux -CC" not in output,
        "tmux prompt/output rendered": b"bash" in output or b"#" in output or b"$" in output,
    }

    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        sys.stderr.write("local PTY smoke failed:\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nCommands sent to tmux:\n")
        sys.stderr.buffer.write(control_input)
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
        sys.stderr.write("\nStartup screen:\n")
        sys.stderr.write(startup_screen)
        sys.stderr.write("\n")
        return 1

    print("local PTY smoke passed")
    return 0


def require(command: str) -> None:
    if shutil.which(command) is None:
        raise SystemExit(f"missing required command: {command}")


def write_fake_ssh(path: str) -> None:
    script = r"""#!/usr/bin/env python3
import os
import pty
import select
import sys

remote_command = sys.argv[-1] if "tmux -CC" in sys.argv[-1] else None
log_path = os.environ.get("FAKE_SSH_LOG")
pid, fd = pty.fork()
if pid == 0:
    if remote_command:
        os.execlp("bash", "bash", "-lc", remote_command)
    os.write(2, b"Warning fake ssh stderr\r\n")
    os.write(1, b"Welcome fake ssh login\r\n")
    os.execlp("bash", "bash", "--noprofile", "--norc", "-i")

while True:
    readable, _, _ = select.select([sys.stdin.buffer, fd], [], [])
    if fd in readable:
        try:
            data = os.read(fd, 4096)
        except OSError:
            break
        if not data:
            break
        os.write(sys.stdout.fileno(), data)
    if sys.stdin.buffer in readable:
        data = os.read(sys.stdin.fileno(), 4096)
        if not data:
            break
        if log_path:
            with open(log_path, "ab") as handle:
                handle.write(data)
        os.write(fd, data)
"""
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(script)
    os.chmod(path, 0o755)


def run_slsh(env: dict[str, str]) -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "fakehost"]

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    output = b""
    stage = "wait_prompt"
    stage_at = time.time()
    deadline = time.time() + 15

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

            if stage == "wait_prompt" and prompt_seen(output):
                os.write(fd, b"xy\x7f\x1b[D\x03")
                stage = "sent_shortcuts"
                stage_at = time.time()
            elif stage == "sent_shortcuts" and time.time() - stage_at > 0.25:
                os.write(fd, b"echo hellx\x7fo\r")
                stage = "wait_hello"
            elif stage == "wait_hello" and b"hello" in output:
                os.write(fd, b"printf '\\033[31mred\\033[0m\\n'\r")
                os.write(fd, b"printf '\\033[38;5;196mhot\\033[0m\\n'\r")
                os.write(fd, b"printf '\\033(0lqk\\033(B\\n'\r")
                stage = "wait_features"
            elif (
                stage == "wait_features"
                and b"\x1b[31mred" in output
                and b"\x1b[38;5;196mhot" in output
                and "┌─┐".encode() in output
            ):
                os.write(fd, b"exit\r")
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def run_startup_slsh(env: dict[str, str]) -> bytes:
    argv = [os.path.join(ROOT, "target", "debug", "slsh"), "fakehost"]

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    output = b""
    deadline = time.time() + 10
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
                if "Welcome fake ssh login" in reduce_terminal(output) and prompt_seen(output):
                    os.write(fd, b"exit\r")
                    return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def reduce_terminal(output: bytes, rows: int = 24, cols: int = 80) -> str:
    text = output.decode("utf-8", "ignore")
    screen = [[" "] * cols for _ in range(rows)]
    row = 0
    col = 0
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\x1b":
            end = parse_escape(text, i, screen, rows, cols)
            if end is not None:
                row, col, i = end(row, col)
                continue
            i += 1
            continue
        if ch == "\r":
            col = 0
        elif ch == "\n":
            row += 1
            if row >= rows:
                screen.pop(0)
                screen.append([" "] * cols)
                row = rows - 1
        elif ch >= " ":
            if 0 <= row < rows and 0 <= col < cols:
                screen[row][col] = ch
            col += 1
            if col >= cols:
                col = cols - 1
        i += 1
    return "\n".join("".join(line).rstrip() for line in screen)


def parse_escape(text: str, index: int, screen: list[list[str]], rows: int, cols: int):
    if index + 1 >= len(text) or text[index + 1] != "[":
        return None
    end = index + 2
    while end < len(text) and not text[end].isalpha():
        end += 1
    if end >= len(text):
        return None
    body = text[index + 2 : end]
    action = text[end]

    def apply(row: int, col: int):
        next_row = row
        next_col = col
        if action == "J" and body.endswith("2"):
            for r in range(rows):
                screen[r] = [" "] * cols
        elif action == "K":
            for c in range(next_col, cols):
                screen[next_row][c] = " "
        elif action == "H":
            parts = [part for part in body.split(";") if part and not part.startswith("?")]
            if len(parts) >= 2 and parts[0].isdigit() and parts[1].isdigit():
                next_row = max(0, min(rows - 1, int(parts[0]) - 1))
                next_col = max(0, min(cols - 1, int(parts[1]) - 1))
        return next_row, next_col, end + 1

    return apply


def prompt_seen(output: bytes) -> bool:
    return b"bash" in output or b"#" in output or b"$" in output


def prompt_line_count(screen: str) -> int:
    return sum(1 for line in screen.splitlines() if line.rstrip().endswith(("#", "$", ">")))


if __name__ == "__main__":
    raise SystemExit(main())
