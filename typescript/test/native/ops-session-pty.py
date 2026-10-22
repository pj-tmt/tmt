"""Real Ops terminal: copy/cancel, then same-PID foreground resume or move with restored modes."""
import errno
import fcntl
import json
import os
import pathlib
import select
import shlex
import signal
import struct
import subprocess
import sys
import termios
import time

sys.dont_write_bytecode = True
from ops_pty_screen import Screen

spec = json.loads(sys.argv[1])
root = pathlib.Path(spec["root"])
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 160, 0, 0))
original = termios.tcgetattr(slave)
expected_modes = original[:-1] + [[v.hex() if isinstance(v, bytes) else v for v in original[-1]]]
(root / "wire.json").write_text(json.dumps(spec))
(root / "active").mkdir()
status_read, status_write = os.pipe()
release_read, release_write = os.pipe()
screen = Screen()
output = bytearray()
query_tail = b""
status_tail = b""
board_pid = None
board_status = None
child = None


def controlling_terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def read_output(timeout):
    global query_tail, status_tail, board_pid, board_status
    ready = select.select([master, status_read], [], [], timeout)[0]
    if status_read in ready:
        status_tail += os.read(status_read, 4096)
        while b"\n" in status_tail:
            line, status_tail = status_tail.split(b"\n", 1)
            state = json.loads(line)
            board_pid = state.get("pid", board_pid)
            board_status = state.get("returncode", board_status)
    if master in ready:
        try:
            data = os.read(master, 65536)
        except OSError as error:
            if error.errno != errno.EIO:
                raise
            return
        output.extend(data)
        screen.feed(data)
        query_tail += data
        while b"\x1b[?u\x1b[c" in query_tail:
            _, query_tail = query_tail.split(b"\x1b[?u\x1b[c", 1)
            os.write(master, b"\x1b[?0u\x1b[?1;2c")
        query_tail = query_tail[-16:]


def wait_for(predicate, label):
    deadline = time.monotonic() + 25
    while not predicate():
        if board_status is not None or time.monotonic() >= deadline:
            raise AssertionError(label + f" (exit={board_status})\n" + bytes(output[-12000:]).decode("utf8", "replace"))
        read_output(min(0.1, max(0, deadline - time.monotonic())))


def open_review():
    os.write(master, b"R")
    wait_for(lambda: screen.contains("Launch preset:"), "session public read completed")
    os.write(master, b"\x1b[H" + (b"\x1b[B" if spec["move"] else b"") + b"\r")
    wait_for(lambda: screen.contains("The board closes here") and screen.contains("c copy command"), "review before foreground handoff")


try:
    assert not os.environ.get("TMUX") and not os.environ.get("TMUX_PANE")
    command = [spec["ops"], "ui", "--squad", "product", "--view", "member-session-pty", "--reset-view"]
    owner = """
import json, os, subprocess, sys
status, release = int(sys.argv[1]), int(sys.argv[2])
p = subprocess.Popen(json.loads(sys.argv[3]))
os.write(status, (json.dumps({'pid':p.pid})+'\\n').encode())
os.write(status, (json.dumps({'returncode':p.wait()})+'\\n').encode())
os.read(release,1)
"""
    child = subprocess.Popen([sys.executable, "-c", owner, str(status_write), str(release_read), json.dumps(command)],
        stdin=slave, stdout=slave, stderr=slave, pass_fds=(status_write, release_read),
        preexec_fn=controlling_terminal,
        env={**os.environ, "TMT_EXECUTABLE": spec["proxy"], "TERM": "xterm-256color", "COLORTERM": "truecolor"})
    wait_for(lambda: screen.contains("room product") and screen.contains("No current recipient observed."), "initial squad")
    os.write(master, b"\x1b[C")
    wait_for(lambda: screen.contains("▾"), "expanded squad")
    os.write(master, b"\x1b[F")
    wait_for(lambda: screen.contains("⏎ to message worker"), "selected UUID")
    open_review()
    os.write(master, b"c")
    copied = root / "copied"
    wait_for(copied.exists, "copy without leaving the board")
    expected_argv = ["resume"] + (["--move"] if spec["move"] else []) + ["--", spec["recipient"]]
    assert shlex.split(copied.read_text()) == [spec["proxy"]] + expected_argv
    assert board_status is None and not (root / "handoff.json").exists()
    os.write(master, b"\x1b")
    wait_for(lambda: not screen.contains("The board closes here"), "cancel confirmation")
    os.write(master, b"\x1b")
    wait_for(lambda: not screen.contains("Identity UUID:"), "close session without launching")
    assert not (root / "handoff.json").exists()
    # Reopening queues another read after Copy on the same ordered lane, so the
    # new ready state also proves the copy effect completed before confirmation.
    open_review()
    os.write(master, b"\x1b[F\r")
    deadline = time.monotonic() + 25
    while board_status is None and time.monotonic() < deadline:
        read_output(0.1)
    assert board_status == 23, f"Core refusal exit was not preserved: {board_status}"
    read_output(0)
    proof = json.loads((root / "handoff.json").read_text())
    assert proof["argv"] == expected_argv
    assert proof["pid"] == board_pid, "resume must replace the board, not spawn a detached child"
    assert proof["tty"] == [True, True, True] and proof["foreground"]
    assert proof["modes"] == expected_modes and termios.tcgetattr(slave) == original
    assert proof["activeReads"] == [], "a public read remained active at exec"
    calls = [json.loads(line) for line in (root / "resume-calls.jsonl").read_text().splitlines()]
    assert calls == [["resume", "--show", "--", spec["recipient"]]] * 2 + [expected_argv]
    handoff_at = bytes(output).index(b"FOREGROUND_SESSION_REFUSAL")
    before = bytes(output[:handoff_at])
    assert b"\x1b[?1049l" in before and b"\x1b[<1u" in before
    assert before.count(b"\x1b[>1u") == before.count(b"\x1b[<1u") > 0
    os.write(release_write, b"x")
    child.wait(timeout=5)
    print(json.dumps({"mode":"move" if spec["move"] else "resume", "samePid":True, "terminalRestoredBeforeExec":True,
        "foreground":True,"copyAndCancelWithoutExec":True,"activeReads":0,"coreExit":board_status,"argv":proof["argv"]}))
finally:
    if child is not None:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=5)
    for fd in (master, slave, status_read, status_write, release_read, release_write):
        os.close(fd)
