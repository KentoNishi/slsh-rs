#!/usr/bin/env python3
import fcntl
import getpass
import os
import pty
import select
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SSH_USER = getpass.getuser()


def main() -> int:
    for command in ["cargo", "ssh", "sshd", "ssh-keygen"]:
        require(command)

    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    with tempfile.TemporaryDirectory() as tmp:
        port = free_port()
        client_key = os.path.join(tmp, "client")
        host_key = os.path.join(tmp, "host")
        known_hosts = os.path.join(tmp, "known_hosts")
        authorized_keys = os.path.join(tmp, "authorized_keys")
        sshd_config = os.path.join(tmp, "sshd_config")
        sshd_log = os.path.join(tmp, "sshd.log")

        subprocess.run(
            ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", client_key],
            check=True,
        )
        subprocess.run(
            ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", host_key],
            check=True,
        )
        shutil.copyfile(f"{client_key}.pub", authorized_keys)
        write_sshd_config(sshd_config, port, host_key, authorized_keys, tmp)

        if os.geteuid() == 0:
            os.makedirs("/run/sshd", exist_ok=True)

        sshd = subprocess.Popen(
            [shutil.which("sshd") or "sshd", "-D", "-f", sshd_config, "-E", sshd_log],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            wait_for_sshd(port, client_key, known_hosts, sshd, sshd_log)
            output = run_slsh(port, client_key, known_hosts)
        finally:
            sshd.terminate()
            try:
                sshd.wait(timeout=2)
            except subprocess.TimeoutExpired:
                sshd.kill()
                sshd.wait()

        checks = {
            "echo command output": b"hello" in output,
            "red sgr output": b"\x1b[31mred" in output,
            "256-color sgr output": b"\x1b[38;5;196mhot" in output,
            "dec special graphics output": dec_graphics_seen(output),
            "alternate screen exit reset style": alternate_exit_reset_seen(output),
            "alternate screen exit preserved scrollback": not alternate_exit_repaint_seen(output),
        }

    failed = [name for name, ok in checks.items() if not ok]
    if failed:
        sys.stderr.write("local sshd smoke failed:\n")
        for name in failed:
            sys.stderr.write(f"  missing {name}\n")
        sys.stderr.write("\nCaptured bytes:\n")
        sys.stderr.buffer.write(output)
        sys.stderr.write("\n")
        return 1

    print("local sshd smoke passed")
    return 0


def require(command: str) -> None:
    if shutil.which(command) is None:
        raise SystemExit(f"missing required command: {command}")


def free_port() -> int:
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    try:
        return sock.getsockname()[1]
    finally:
        sock.close()


def write_sshd_config(
    path: str, port: int, host_key: str, authorized_keys: str, tmp: str
) -> None:
    config = f"""Port {port}
ListenAddress 127.0.0.1
HostKey {host_key}
PidFile {os.path.join(tmp, "sshd.pid")}
AuthorizedKeysFile {authorized_keys}
PasswordAuthentication no
KbdInteractiveAuthentication no
ChallengeResponseAuthentication no
UsePAM no
PermitRootLogin yes
StrictModes no
LogLevel ERROR
"""
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(config)


def ssh_probe_args(port: int, client_key: str, known_hosts: str) -> list[str]:
    return [
        "ssh",
        "-p",
        str(port),
        "-i",
        client_key,
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        f"UserKnownHostsFile={known_hosts}",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=1",
        f"{SSH_USER}@127.0.0.1",
    ]


def wait_for_sshd(
    port: int,
    client_key: str,
    known_hosts: str,
    sshd: subprocess.Popen,
    sshd_log: str,
) -> None:
    for _ in range(50):
        if sshd.poll() is not None:
            raise SystemExit(read_sshd_log(sshd_log))
        probe = subprocess.run(
            ssh_probe_args(port, client_key, known_hosts) + ["true"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if probe.returncode == 0:
            return
        time.sleep(0.1)
    raise SystemExit(f"sshd did not become ready\n{read_sshd_log(sshd_log)}")


def read_sshd_log(path: str) -> str:
    if not os.path.exists(path):
        return "sshd exited without a log"
    with open(path, encoding="utf-8", errors="replace") as handle:
        return handle.read()


def run_slsh(port: int, client_key: str, known_hosts: str) -> bytes:
    argv = [
        os.path.join(ROOT, "target", "debug", "slsh"),
        "-p",
        str(port),
        "-i",
        client_key,
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        f"UserKnownHostsFile={known_hosts}",
        "-o",
        "IdentitiesOnly=yes",
        f"{SSH_USER}@127.0.0.1",
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
    stage = "wait_prompt"
    deadline = time.time() + 20

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
                and dec_graphics_seen(output)
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


def prompt_seen(output: bytes) -> bool:
    return b"bash" in output or b"#" in output or b"$" in output


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
