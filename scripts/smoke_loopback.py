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
    delayed_submit_output = run_delayed_submit_overlay()
    seeded_cursor_output = run_seeded_cursor_overlay()
    scrolled_overlay_output = run_scrolled_overlay()
    app_prefix_output = run_app_prefix_guard()
    app_cursor_output = run_app_cursor_overlay()
    mouse_sgr_output = run_mouse_forwarding(True)
    mouse_x10_output = run_mouse_forwarding(False)

    failed = []
    if marker not in output:
        failed.append("interactive shell marker")
    if command_marker not in command_output:
        failed.append("remote command marker")
    if b"SLSHLAG" not in delayed_echo_output:
        failed.append("delayed local echo")
    if b"echo SLSHSUBMIT" not in delayed_submit_output:
        failed.append("delayed submit keeps overlay")
    if b"\x1b[10;" not in seeded_cursor_output or b"\x1b[1;" in seeded_cursor_output:
        failed.append("startup cursor seeded overlay")
    if (
        b"LINE30" not in scrolled_overlay_output
        or b"\x1b[10;" not in scrolled_overlay_output
        or b"\x1b[1;" in scrolled_overlay_output.split(b"LINE30", 1)[-1]
    ):
        failed.append("scrolled overlay row")
    if b"Q" in app_prefix_output:
        failed.append("nonlinear prefix suppresses next printable overlay")
    if b"Z" not in app_cursor_output or b"\x1b[2;1H" not in app_cursor_output:
        failed.append("app cursor overlay row")
    if b"SLSHMOUSEOK" not in mouse_sgr_output:
        failed.append("SGR mouse forwarding")
    if b"SLSHMOUSEOK" not in mouse_x10_output:
        failed.append("X10 mouse forwarding")

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
        sys.stderr.write("\nDelayed submit bytes:\n")
        sys.stderr.buffer.write(delayed_submit_output)
        sys.stderr.write("\nSeeded cursor bytes:\n")
        sys.stderr.buffer.write(seeded_cursor_output)
        sys.stderr.write("\nScrolled overlay bytes:\n")
        sys.stderr.buffer.write(scrolled_overlay_output)
        sys.stderr.write("\nApp prefix bytes:\n")
        sys.stderr.buffer.write(app_prefix_output)
        sys.stderr.write("\nApp cursor bytes:\n")
        sys.stderr.buffer.write(app_cursor_output)
        sys.stderr.write("\nSGR mouse bytes:\n")
        sys.stderr.buffer.write(mouse_sgr_output)
        sys.stderr.write("\nX10 mouse bytes:\n")
        sys.stderr.buffer.write(mouse_x10_output)
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


def run_delayed_submit_overlay() -> bytes:
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
    sent_at = 0.0
    send_at = time.time() + 1.25
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
                os.write(fd, b"echo SLSHSUBMIT\r")
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
    answer_cursor_at = None
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

                if not answered_cursor_query and answer_cursor_at is None and b"\x1b[6n" in output:
                    answer_cursor_at = time.time() + 0.15

            if answer_cursor_at is not None and time.time() >= answer_cursor_at:
                os.write(fd, b"\x1b[10;1R")
                answered_cursor_query = True
                answer_cursor_at = None

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


def run_scrolled_overlay() -> bytes:
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
    sent_scroll = False
    sent_probe = False
    probe_at = 0.0
    deadline = time.time() + 12
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
                    os.write(fd, b"\x1b[1;1R")
                    answered_cursor_query = True

            if answered_cursor_query and not sent_scroll and (b"$" in output or b"#" in output):
                command = b"for i in $(seq 1 30); do echo LINE$i; done\r"
                os.write(fd, b"\x1b[200~" + command + b"\x1b[201~")
                sent_scroll = True
            if sent_scroll and not sent_probe and b"LINE30" in output:
                os.write(fd, b"Z")
                sent_probe = True
                probe_at = time.time()
            if sent_probe and time.time() - probe_at >= 0.25:
                return output
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output


def run_app_prefix_guard() -> bytes:
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

    app = (
        "python3 -c 'import os,sys,tty,time;"
        "tty.setraw(0);"
        "sys.stdout.write(\"\\033[?1049h\\033[?1h\\033[HAPP\");"
        "sys.stdout.flush();"
        "os.read(0,2);"
        "time.sleep(.2);"
        "sys.stdout.write(\"\\033[?1l\\033[?1049l\");"
        "sys.stdout.flush()'\r"
    ).encode()

    output = b""
    sent_app = False
    sent_prefix = False
    prefix_at = 0.0
    prefix_start = 0
    deadline = time.time() + 12
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.02)
            if readable:
                try:
                    output += os.read(fd, 4096)
                except BlockingIOError:
                    pass
                except OSError:
                    return output[prefix_start:]

            if not sent_app and (b"$" in output or b"#" in output):
                os.write(fd, b"\x1b[200~" + app + b"\x1b[201~")
                sent_app = True
            if sent_app and not sent_prefix and b"\x1b[HAPP" in output:
                prefix_start = len(output)
                os.write(fd, b"\x02Q")
                sent_prefix = True
                prefix_at = time.time()
            if sent_prefix and time.time() - prefix_at >= 0.25:
                return output[prefix_start:]
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output[prefix_start:]


def run_app_cursor_overlay() -> bytes:
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

    app = (
        "python3 -c 'import os,sys,tty,time;"
        "tty.setraw(0);"
        "sys.stdout.write(\"\\033[?1049h\\033[?1h\\033[H\\033[2J"
        "HEADER\\r\\033[23d^G Help\\r\\033[24d^X Exit\\r\\033[22d\\033[2d\");"
        "sys.stdout.flush();"
        "os.read(0,1);"
        "time.sleep(.2);"
        "sys.stdout.write(\"\\033[?1l\\033[?1049l\");"
        "sys.stdout.flush()'\r"
    ).encode()

    output = b""
    sent_app = False
    sent_probe = False
    probe_at = 0.0
    probe_start = 0
    deadline = time.time() + 12
    try:
        while time.time() < deadline:
            readable, _, _ = select.select([fd], [], [], 0.02)
            if readable:
                try:
                    output += os.read(fd, 4096)
                except BlockingIOError:
                    pass
                except OSError:
                    return output[probe_start:]

            if not sent_app and (b"$" in output or b"#" in output):
                os.write(fd, b"\x1b[200~" + app + b"\x1b[201~")
                sent_app = True
            if sent_app and not sent_probe and b"\x1b[24d^X Exit" in output:
                probe_start = len(output)
                os.write(fd, b"Z")
                sent_probe = True
                probe_at = time.time()
            if sent_probe and time.time() - probe_at >= 0.25:
                return output[probe_start:]
    finally:
        try:
            os.kill(pid, signal.SIGTERM)
        except OSError:
            pass

    return output[probe_start:]


def run_mouse_forwarding(sgr: bool) -> bytes:
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

    enable_mouse = "\\033[?1000h\\033[?1006h" if sgr else "\\033[?1000h"
    disable_mouse = "\\033[?1006l\\033[?1000l" if sgr else "\\033[?1000l"
    expected = "\\033[<0;10;5M" if sgr else "\\033[M *%"
    injected = b"\x1b[<0;10;5M" if sgr else b"\x1b[M *%"

    app = (
        "python3 -c 'import os,sys,tty;"
        "tty.setraw(0);"
        f"sys.stdout.write(\"\\033[?1049h{enable_mouse}MOUSE_READY\\r\\n\");"
        "sys.stdout.flush();"
        "data=os.read(0,32);"
        f"ok=data.startswith(b\"{expected}\");"
        f"sys.stdout.write(\"{disable_mouse}\\033[?1049l\" + (\"SLSHMOUSEOK\\n\" if ok else (\"SLSHMOUSEBAD %r\\n\" % (data,))));"
        "sys.stdout.flush()'\r"
    ).encode()

    output = b""
    sent_app = False
    sent_mouse = False
    deadline = time.time() + 12
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

            if not sent_app and (b"$" in output or b"#" in output):
                os.write(fd, b"\x1b[200~" + app + b"\x1b[201~")
                sent_app = True
            if sent_app and not sent_mouse and b"MOUSE_READY" in output:
                os.write(fd, injected)
                sent_mouse = True
            if b"SLSHMOUSEOK" in output or b"SLSHMOUSEBAD" in output:
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
