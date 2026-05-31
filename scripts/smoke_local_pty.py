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
    require("cargo")

    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    with tempfile.TemporaryDirectory() as tmp:
        fake_ssh = os.path.join(tmp, "ssh")
        fake_ssh_log = os.path.join(tmp, "ssh.stdin")
        write_fake_ssh(fake_ssh)

        env = os.environ.copy()
        env["PATH"] = f"{tmp}{os.pathsep}{env['PATH']}"
        env["FAKE_SSH_LOG"] = fake_ssh_log
        env["FAKE_SSH_SLOW_LOGIN"] = "1"

        startup_output = run_startup_slsh(env)
        output = run_slsh(env)
        if os.path.exists(fake_ssh_log):
            with open(fake_ssh_log, "rb") as handle:
                ssh_input = handle.read()
        else:
            ssh_input = b""

    startup_screen = reduce_terminal(startup_output)
    output_screen = reduce_terminal(output)
    checks = {
        "startup stderr warning visible": "Warning fake ssh stderr" in startup_screen,
        "startup login preamble visible": "Welcome fake ssh login" in startup_screen,
        "startup prompt not doubled": prompt_line_count(startup_screen) == 1,
        "echo command output": b"hello" in output,
        "red sgr output": b"\x1b[31mred" in output,
        "256-color sgr output": b"\x1b[38;5;196mhot" in output,
        "dec special graphics output": dec_graphics_seen(output) or "┌─┐" in output_screen,
        "alternate screen exit reset style": alternate_exit_reset_seen(output),
        "alternate screen exit preserved scrollback": not alternate_exit_repaint_seen(output),
        "enter key forwarded": b"\r" in ssh_input,
        "backspace key forwarded": b"\x7f" in ssh_input,
        "ctrl-c forwarded": b"\x03" in ssh_input,
        "ctrl-x forwarded": b"\x18" in ssh_input,
        "left key forwarded": b"\x1b[D" in ssh_input,
        "ctrl-left key forwarded": b"\x1b[1;5D" in ssh_input,
        "ctrl-right key forwarded": b"\x1b[1;5C" in ssh_input,
        "ctrl-delete key forwarded": b"\x1b[3;5~" in ssh_input,
        "login preamble captured": b"Welcome fake ssh login" in output,
        "prompt/output rendered": b"bash" in output or b"#" in output or b"$" in output,
    }

    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        sys.stderr.write("local PTY smoke failed:\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nBytes sent to ssh:\n")
        sys.stderr.buffer.write(ssh_input)
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
import time
import tty

log_path = os.environ.get("FAKE_SSH_LOG")
tty.setraw(sys.stdin.fileno())
pid, fd = pty.fork()
if pid == 0:
    os.write(2, b"Warning fake ssh stderr\r\n")
    if os.environ.get("FAKE_SSH_SLOW_LOGIN"):
        time.sleep(1.0)
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
                os.write(fd, b"xy\x7f\x1b[D\x1b[1;5D\x1b[1;5C\x1b[3;5~\x18\x03")
                stage = "sent_shortcuts"
                stage_at = time.time()
            elif stage == "sent_shortcuts" and time.time() - stage_at > 0.25:
                os.write(fd, b"echo hellx\x7fo\r")
                stage = "wait_hello"
            elif stage == "wait_hello" and b"hello" in output:
                os.write(fd, b"printf '\\033[31mred\\033[0m\\n'\r")
                os.write(fd, b"printf '\\033[38;5;196mhot\\033[0m\\n'\r")
                os.write(fd, b"printf '\\033)0\\016lqk\\017\\n'\r")
                stage = "wait_features"
            elif (
                stage == "wait_features"
                and b"\x1b[31mred" in output
                and b"\x1b[38;5;196mhot" in output
                and "┌─┐" in reduce_terminal(output)
            ):
                os.write(
                    fd,
                    b"printf '\\033[?1049h\\033[42m\\033[2JALT\\033[?1049l\\n'\r",
                )
                stage = "wait_alternate"
            elif stage == "wait_alternate" and alternate_exit_reset_seen(output):
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
    g0_dec = False
    g1_dec = False
    using_g1 = False
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\x1b":
            if i + 2 < len(text) and text[i + 1] in "()" and text[i + 2] in "0B":
                if text[i + 1] == "(":
                    g0_dec = text[i + 2] == "0"
                else:
                    g1_dec = text[i + 2] == "0"
                i += 3
                continue
            end = parse_escape(text, i, screen, rows, cols)
            if end is not None:
                row, col, i = end(row, col)
                continue
            i += 1
            continue
        if ch == "\x0e":
            using_g1 = True
        elif ch == "\x0f":
            using_g1 = False
        elif ch == "\r":
            col = 0
        elif ch == "\n":
            row += 1
            if row >= rows:
                screen.pop(0)
                screen.append([" "] * cols)
                row = rows - 1
        elif ch >= " ":
            if g1_dec if using_g1 else g0_dec:
                ch = map_dec_special_graphics(ch)
            if 0 <= row < rows and 0 <= col < cols:
                screen[row][col] = ch
            col += 1
            if col >= cols:
                col = cols - 1
        i += 1
    return "\n".join("".join(line).rstrip() for line in screen)


def map_dec_special_graphics(ch: str) -> str:
    return {
        "j": "┘",
        "k": "┐",
        "l": "┌",
        "m": "└",
        "n": "┼",
        "q": "─",
        "t": "├",
        "u": "┤",
        "v": "┴",
        "w": "┬",
        "x": "│",
    }.get(ch, ch)


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


def dec_graphics_seen(output: bytes) -> bool:
    return (
        "┌─┐".encode() in output
        or b"\x1b(0lqk\x1b(B" in output
        or b"\x1b)0\x0elqk\x0f" in output
    )


def alternate_exit_repaint_seen(output: bytes) -> bool:
    exit_index = output.rfind(b"\x1b[?1049l")
    if exit_index < 0:
        return False
    return b"\x1b[2J" in output[exit_index:]


def alternate_exit_reset_seen(output: bytes) -> bool:
    exit_index = output.rfind(b"\x1b[?1049l")
    if exit_index < 0:
        return False
    return b"\x1b[0m" in output[exit_index:]


if __name__ == "__main__":
    raise SystemExit(main())
