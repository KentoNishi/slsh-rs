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
import termios
import time


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SLSH = os.path.join(ROOT, "target", "debug", "slsh")


def main() -> int:
    require("cargo")
    require("python3")
    subprocess.run(["cargo", "build"], cwd=ROOT, check=True)

    tests = [
        ("bash tab/edit", run_bash_tab_and_edit),
        ("synthetic repaint editor", run_synthetic_repaint_editor),
        ("resize while editing", run_resize_while_editing),
        ("less search/quit", run_less_search),
    ]
    optional = [
        ("vim modal/insert", "vim", run_vim_modal_insert),
        ("nano edit/exit", "nano", run_nano_edit_exit),
        ("tmux prefix/detach", "tmux", run_tmux_prefix_detach),
        ("htop arrows/quit", "htop", run_htop_arrows_quit),
    ]
    for name, command, test in optional:
        if shutil.which(command):
            tests.append((name, test))
        else:
            print(f"skip {name}: missing {command}")

    failed = []
    for name, test in tests:
        try:
            test()
            print(f"ok {name}")
        except TestFailure as failure:
            failed.append((name, failure))
            sys.stderr.write(f"\nFAIL {name}: {failure}\n")
            if failure.output:
                sys.stderr.write("captured bytes:\n")
                sys.stderr.buffer.write(failure.output[-12000:])
                sys.stderr.write("\n")

    if failed:
        sys.stderr.write("\nstress interactions failed:\n")
        for name, failure in failed:
            sys.stderr.write(f"  {name}: {failure}\n")
        return 1

    print("stress interactions passed")
    return 0


class TestFailure(AssertionError):
    def __init__(self, message: str, output: bytes = b""):
        super().__init__(message)
        self.output = output


def require(command: str) -> None:
    if shutil.which(command) is None:
        raise SystemExit(f"missing required command: {command}")


def loopback_env(delay_ms: int = 120) -> dict[str, str]:
    env = os.environ.copy()
    env["SLSH_LOOPBACK"] = "1"
    env["SLSH_DELAY_MS"] = str(delay_ms)
    env["TERM"] = "xterm-256color"
    env["SHELL"] = shutil.which("bash") or "/bin/sh"
    env["BASH_SILENCE_DEPRECATION_WARNING"] = "1"
    env.pop("ENV", None)
    env.pop("BASH_ENV", None)
    return env


def spawn_slsh(args: list[str], env: dict[str, str], rows: int = 24, cols: int = 80):
    argv = [SLSH, "ignored-host"] + args
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(argv[0], argv, env)
    os.set_blocking(fd, False)
    fcntl_rows_cols(fd, rows, cols)
    termios.tcflush(fd, termios.TCIOFLUSH)
    return pid, fd


def fcntl_rows_cols(fd: int, rows: int, cols: int) -> None:
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


def read_available(fd: int) -> bytes:
    output = b""
    while True:
        readable, _, _ = select.select([fd], [], [], 0)
        if not readable:
            return output
        try:
            chunk = os.read(fd, 8192)
        except (BlockingIOError, OSError):
            return output
        if not chunk:
            return output
        output += chunk


def child_exited(pid: int) -> bool:
    try:
        waited, _ = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        return True
    return waited == pid


def terminate(pid: int) -> None:
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        pass


def run_until(
    pid: int,
    fd: int,
    drive,
    done,
    timeout: float,
    kill_on_timeout: bool = True,
) -> bytes:
    output = b""
    start = time.time()
    try:
        while time.time() - start < timeout:
            if child_exited(pid):
                return output + read_available(fd)

            readable, _, _ = select.select([fd], [], [], 0.02)
            if readable:
                output += read_available(fd)

            drive(fd, output, time.time() - start)
            if done(output, time.time() - start):
                return output
    finally:
        if kill_on_timeout and not child_exited(pid):
            terminate(pid)

    raise TestFailure("timeout", output)


def wait_exit(pid: int, fd: int, timeout: float) -> bytes:
    output = b""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if child_exited(pid):
            return output + read_available(fd)
        readable, _, _ = select.select([fd], [], [], 0.02)
        if readable:
            output += read_available(fd)
    terminate(pid)
    raise TestFailure("process did not exit", output)


def run_bash_tab_and_edit() -> None:
    env = loopback_env(20)
    env["PS1"] = "SLSH-STRESS$ "
    pid, fd = spawn_slsh(["bash", "--noprofile", "--norc", "-i"], env)
    tmp = f"/tmp/slsh-stress-{os.getpid()}"
    setup = f"mkdir -p {tmp}; printf STRESSTAB > {tmp}/slsh_unique_completion_target; echo SETUPDONE\r"
    complete = f"cat {tmp}/slsh_unique_completion_t\t\r"
    edit = "echo alpa\x1b[Dh\x1b[C\r"
    cleanup = f"rm -rf {tmp}; exit\r"
    state = {"stage": "setup", "sent_at": 0.0}

    def drive(fd: int, output: bytes, elapsed: float) -> None:
        if state["stage"] == "setup" and prompt_seen(output):
            os.write(fd, setup.encode())
            state["stage"] = "complete"
            state["sent_at"] = elapsed
        elif state["stage"] == "complete" and b"SETUPDONE" in output:
            os.write(fd, complete.encode())
            state["stage"] = "edit"
            state["sent_at"] = elapsed
        elif state["stage"] == "edit" and b"STRESSTAB" in output:
            os.write(fd, edit.encode())
            state["stage"] = "cleanup"
            state["sent_at"] = elapsed
        elif state["stage"] == "cleanup" and b"alpha" in output:
            os.write(fd, cleanup.encode())
            state["stage"] = "exit"

    output = run_until(
        pid,
        fd,
        drive,
        lambda output, _elapsed: child_exited(pid)
        or (state["stage"] == "exit" and b"alpha" in output),
        15,
        kill_on_timeout=False,
    )
    output += wait_exit(pid, fd, 4)
    if b"STRESSTAB" not in output:
        raise TestFailure("tab completion command did not run", output)
    if b"alpha" not in output:
        raise TestFailure("cursor edit command did not produce alpha", output)


def run_synthetic_repaint_editor() -> None:
    app = synthetic_editor_app("aXb-complet", resize=False)
    env = loopback_env(180)
    pid, fd = spawn_slsh(["python3", "-c", app], env, rows=14, cols=70)
    sent = {"done": False}

    def drive(fd: int, output: bytes, _elapsed: float) -> None:
        if not sent["done"] and b"SYNTH_READY" in output:
            os.write(fd, b"ab\x1b[D" b"X" b"\x1b[C" b"\t" b"\x7f" b"\r")
            sent["done"] = True

    output = run_until(
        pid,
        fd,
        drive,
        lambda output, _elapsed: b"SYNTH_EDIT_OK" in output or b"SYNTH_EDIT_BAD" in output,
        10,
    )
    if b"SYNTH_EDIT_OK" not in output:
        raise TestFailure("synthetic repaint editor rejected input", output)
    if b"\x1b[2\x1b[" in output:
        raise TestFailure("local patch interleaved inside split remote escape", output)


def run_resize_while_editing() -> None:
    app = synthetic_editor_app("resize-ok", resize=True)
    env = loopback_env(160)
    pid, fd = spawn_slsh(["python3", "-c", app], env, rows=12, cols=60)
    state = {"sent": False, "resized": False, "finished": False}

    def drive(fd: int, output: bytes, elapsed: float) -> None:
        if not state["sent"] and b"SYNTH_READY" in output:
            os.write(fd, b"resize")
            state["sent"] = True
        elif state["sent"] and not state["resized"] and elapsed > 0.8:
            fcntl_rows_cols(fd, 18, 90)
            state["resized"] = True
        elif state["resized"] and not state["finished"] and elapsed > 1.4:
            os.write(fd, b"-ok\r")
            state["finished"] = True

    output = run_until(
        pid,
        fd,
        drive,
        lambda output, _elapsed: b"SYNTH_EDIT_OK" in output or b"SYNTH_EDIT_BAD" in output,
        12,
    )
    if b"SYNTH_EDIT_OK" not in output:
        raise TestFailure("resize editing fixture rejected input", output)


def synthetic_editor_app(expected: str, resize: bool) -> str:
    return f"""
import os,sys,termios,tty,time
tty.setraw(0)
text=''
cursor=0
expected={expected!r}
resize={resize!r}
counter=0

def size():
    try:
        s=os.get_terminal_size(1)
        return max(5,s.lines), max(20,s.columns)
    except OSError:
        return 14,70

def draw(partial=False):
    global counter
    rows, cols = size()
    counter += 1
    sys.stdout.write('\\033[?1049h\\033[?1h')
    if partial:
        sys.stdout.write('\\033[2;1H\\033[2K')
        sys.stdout.flush()
        time.sleep(0.025)
    sys.stdout.write('\\033[H\\033[2J')
    sys.stdout.write('SYNTH_READY row=%d col=%d\\r\\n' % (rows, cols))
    if resize and counter % 2 == 0:
        sys.stdout.write('\\033[3;1Habove input changes %d\\033[L\\033[M' % counter)
    sys.stdout.write('\\033[2;1H> ' + text)
    sys.stdout.write('\\033[%d;1H\\033[42mSTATUS %d\\033[0m' % (rows, counter))
    sys.stdout.write('\\033[2;%dH' % (cursor + 3))
    sys.stdout.flush()

def read_key():
    b=os.read(0,1)
    if b == b'\\x1b':
        b += os.read(0,1)
        if b.endswith(b'['):
            b += os.read(0,1)
        elif b.endswith(b'O'):
            b += os.read(0,1)
    return b

draw()
while True:
    key=read_key()
    if key in (b'\\r', b'\\n'):
        break
    if key in (b'\\x1b[D', b'\\x1bOD'):
        cursor=max(0,cursor-1)
    elif key in (b'\\x1b[C', b'\\x1bOC'):
        cursor=min(len(text),cursor+1)
    elif key in (b'\\x7f', b'\\x08'):
        if cursor:
            text=text[:cursor-1]+text[cursor:]
            cursor-=1
    elif key == b'\\t':
        insert='-complete'
        text=text[:cursor]+insert+text[cursor:]
        cursor+=len(insert)
    elif key == b'\\x03':
        text='INTERRUPTED'
        break
    else:
        s=key.decode('utf-8','ignore')
        text=text[:cursor]+s+text[cursor:]
        cursor+=len(s)
    draw(partial=True)
    time.sleep(0.015)

sys.stdout.write('\\033[?1l\\033[?1049l')
sys.stdout.write(('SYNTH_EDIT_OK' if text == expected else 'SYNTH_EDIT_BAD %r expected %r' % (text, expected)) + '\\r\\n')
sys.stdout.flush()
"""


def run_less_search() -> None:
    if not shutil.which("less"):
        print("skip less search/quit: missing less")
        return
    script = "printf 'line %03d\\n' $(seq 1 220) | less"
    env = loopback_env(100)
    pid, fd = spawn_slsh(["bash", "-lc", script], env)
    sent = {"done": False}

    def drive(fd: int, output: bytes, _elapsed: float) -> None:
        if not sent["done"] and (b"line 001" in output or b":" in output):
            os.write(fd, b"/line 150\rnq")
            sent["done"] = True

    output = run_until(pid, fd, drive, lambda _output, _elapsed: child_exited(pid), 10)
    if not sent["done"]:
        raise TestFailure("less never became interactive", output)


def run_vim_modal_insert() -> None:
    path = f"/tmp/slsh-stress-vim-{os.getpid()}.txt"
    env = loopback_env(220)
    pid, fd = spawn_slsh(["vim", "-Nu", "NONE", "-n", "-i", "NONE", path], env)
    state = {"stage": "wait"}

    def drive(fd: int, output: bytes, elapsed: float) -> None:
        if state["stage"] == "wait" and (path.encode() in output or elapsed > 1.2):
            os.write(fd, b"islsh vim stress")
            state["stage"] = "insert"
        elif state["stage"] == "insert" and b"-- INSERT --" in output:
            os.write(fd, b"\x1b:q!\r")
            state["stage"] = "quit"

    output = run_until(pid, fd, drive, lambda _output, _elapsed: child_exited(pid), 12)
    if b"\x1b[>c" in output or b"\x1b]10;?\x07" in output or b"\x1b]11;?\x07" in output:
        raise TestFailure("terminal query leaked through vim", output)


def run_nano_edit_exit() -> None:
    path = f"/tmp/slsh-stress-nano-{os.getpid()}.txt"
    env = loopback_env(180)
    pid, fd = spawn_slsh(["nano", path], env)
    state = {"stage": "wait"}

    def drive(fd: int, output: bytes, _elapsed: float) -> None:
        if state["stage"] == "wait" and (b"GNU nano" in output or b"^X Exit" in output):
            os.write(fd, b"nano stress")
            state["stage"] = "typed"
        elif state["stage"] == "typed" and b"nano stress" in output:
            os.write(fd, b"\x18n")
            state["stage"] = "exit"

    output = run_until(pid, fd, drive, lambda _output, _elapsed: child_exited(pid), 12)
    if b"GNU nano" not in output and b"^X Exit" not in output:
        raise TestFailure("nano screen was not observed", output)


def run_tmux_prefix_detach() -> None:
    session = f"slshstress{os.getpid()}"
    env = loopback_env(140)
    env["SHELL"] = shutil.which("bash") or "/bin/sh"
    pid, fd = spawn_slsh(
        ["tmux", "-L", session, "-f", "/dev/null", "new-session", "-s", session],
        env,
        rows=18,
        cols=80,
    )
    state = {"stage": "wait", "detach_at": 0.0, "prefix_start": 0}

    def drive(fd: int, output: bytes, elapsed: float) -> None:
        if state["stage"] == "wait" and (session.encode() in output or elapsed > 1.2):
            os.write(fd, b"echo TMUXSTRESS\r")
            state["stage"] = "echo"
        elif state["stage"] == "echo" and b"TMUXSTRESS" in output:
            state["prefix_start"] = len(output)
            os.write(fd, b"\x02d")
            state["stage"] = "detach"
            state["detach_at"] = elapsed

    output = run_until(pid, fd, drive, lambda _output, _elapsed: child_exited(pid), 12)
    prefix_output = output[state["prefix_start"] :]
    if b"TMUXSTRESS" not in output:
        raise TestFailure("tmux shell command did not render", output)
    if b"\x1b[2md" in prefix_output or b"d\xcc\xb6" in prefix_output:
        raise TestFailure("tmux prefix follower was predicted locally", prefix_output)
    subprocess.run(["tmux", "-L", session, "kill-server"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def run_htop_arrows_quit() -> None:
    env = loopback_env(120)
    pid, fd = spawn_slsh(["htop"], env)
    state = {"stage": "wait"}

    def drive(fd: int, output: bytes, elapsed: float) -> None:
        if state["stage"] == "wait" and (b"F10" in output or b"Load average" in output or elapsed > 1.5):
            os.write(fd, b"\x1b[B\x1b[B\x1b[Aq")
            state["stage"] = "quit"

    output = run_until(pid, fd, drive, lambda _output, _elapsed: child_exited(pid), 10)
    if state["stage"] == "wait":
        raise TestFailure("htop never became interactive", output)


def prompt_seen(output: bytes) -> bool:
    return b"SLSH-STRESS$ " in output or b"bash-" in output or b"# " in output or b"$ " in output


if __name__ == "__main__":
    raise SystemExit(main())
