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
        write_fake_ssh(fake_ssh)

        env = os.environ.copy()
        env["PATH"] = f"{tmp}{os.pathsep}{env['PATH']}"

        output = run_slsh(env)

    checks = {
        "echo command output": b"hello" in output,
        "red sgr output": b"\x1b[31mred" in output,
        "tmux prompt/output rendered": b"bash" in output or b"#" in output or b"$" in output,
    }

    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        sys.stderr.write("local PTY smoke failed:\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
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

remote_command = sys.argv[-1]
pid, fd = pty.fork()
if pid == 0:
    os.execlp("bash", "bash", "-lc", remote_command)

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
        os.write(fd, data)
"""
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(script)
    os.chmod(path, 0o755)


def run_slsh(env: dict[str, str]) -> bytes:
    argv = [
        os.path.join(ROOT, "target", "debug", "slsh"),
        "fakehost",
        "bash",
        "--noprofile",
        "--norc",
        "-i",
    ]

    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    output = b""
    sent_echo = False
    sent_color = False
    deadline = time.time() + 10

    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.05)
            if not readable:
                continue

            try:
                chunk = os.read(fd, 4096)
            except OSError:
                break
            if not chunk:
                break

            output += chunk

            if not sent_echo and prompt_seen(output):
                os.write(fd, b"echo hello\r")
                sent_echo = True
            elif sent_echo and not sent_color and b"hello" in output:
                os.write(fd, b"printf '\\033[31mred\\033[0m\\n'\r")
                sent_color = True
            elif sent_color and b"\x1b[31mred" in output:
                os.write(fd, b"exit\r")
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def prompt_seen(output: bytes) -> bool:
    return b"bash" in output or b"#" in output or b"$" in output


if __name__ == "__main__":
    raise SystemExit(main())
