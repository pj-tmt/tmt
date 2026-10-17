"""Scenario-owned terminal peer for the real Ops composer and lifecycle."""

import errno
import codecs
import fcntl
import json
import os
import select
import re
import signal
import struct
import subprocess
import sys
import termios
import time


spec = json.loads(sys.argv[1])
master, slave = os.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 160, 0, 0))
original_modes = termios.tcgetattr(slave)
child = None
output = bytearray()
queries = 0
query_tail = b""
status_read, status_write = os.pipe()
release_read, release_write = os.pipe()
status_tail = b""
board_pid = None
board_status = None
board_stopped = False


class Screen:
    """Decode the cursor/erase subset written by the real Crossterm backend."""

    def __init__(self):
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.x = self.y = 0
        self.cells = [[" "] * 160 for _ in range(36)]

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b"):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if not match:
                    return
                self.pending = self.pending[match.end():]
                values, _, final = match.groups()
                if values.startswith("?"):
                    if values == "?1049" and final == "h":
                        self.cells = [[" "] * 160 for _ in range(36)]
                    continue
                if values.startswith((">", "<", "=")):
                    continue
                args = [int(v or 0) for v in values.split(";")] if values else [0]
                count = args[0] or 1
                if final in ("H", "f"):
                    self.y = max(0, args[0] - 1)
                    self.x = max(0, (args[1] if len(args) > 1 else 1) - 1)
                elif final == "G":
                    self.x = count - 1
                elif final == "A":
                    self.y = max(0, self.y - count)
                elif final == "B":
                    self.y += count
                elif final == "C":
                    self.x += count
                elif final == "D":
                    self.x = max(0, self.x - count)
                elif final == "J" and args[0] in (2, 3):
                    self.cells = [[" "] * 160 for _ in range(36)]
                elif final == "K" and self.y < 36:
                    start = 0 if args[0] in (1, 2) else self.x
                    end = self.x + 1 if args[0] == 1 else 160
                    self.cells[self.y][start:end] = [" "] * (end - start)
                continue
            char, self.pending = self.pending[0], self.pending[1:]
            if char == "\r":
                self.x = 0
            elif char == "\n":
                self.y += 1
            elif char >= " ":
                if self.y < 36 and self.x < 160:
                    self.cells[self.y][self.x] = char
                self.x += 1

    def contains(self, text):
        return any(text in "".join(row) for row in self.cells)


screen = Screen()


def controlling_terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def read_output(timeout):
    global queries, query_tail, status_tail, board_pid, board_status, board_stopped
    ready = select.select([master, status_read], [], [], timeout)[0]
    if status_read in ready:
        status_tail += os.read(status_read, 4096)
        while b"\n" in status_tail:
            line, status_tail = status_tail.split(b"\n", 1)
            state = json.loads(line)
            board_pid = state.get("pid", board_pid)
            board_status = state.get("returncode", board_status)
            board_stopped = state.get("stopped", board_stopped)
    if master not in ready:
        return
    try:
        data = os.read(master, 65536)
    except OSError as error:
        if error.errno == errno.EIO:
            return
        raise
    output.extend(data)
    screen.feed(data)
    query_tail += data
    while b"\x1b[?u\x1b[c" in query_tail:
        _, query_tail = query_tail.split(b"\x1b[?u\x1b[c", 1)
        queries += 1
        # Answer both queries so crossterm can distinguish support from timeout.
        flags = b"\x1b[?0u" if spec["enhanced"] else b""
        os.write(master, flags + b"\x1b[?1;2c")
    query_tail = query_tail[-16:]


def wait_for(predicate, description, timeout=20):
    deadline = time.monotonic() + timeout
    while not predicate():
        if time.monotonic() >= deadline:
            raise AssertionError(
                description
                + f" (stopped={board_stopped}, raw={termios.tcgetattr(slave) != original_modes}, message={screen.contains('Message')})\n"
                + bytes(output[-12000:]).decode("utf-8", "replace")
            )
        if board_status is not None:
            raise AssertionError(f"{description}: board exited {board_status}\n" + bytes(output[-6000:]).decode("utf-8", "replace"))
        read_output(min(0.1, max(0, deadline - time.monotonic())))


def fresh_text(text, after=0):
    return screen.contains(text)


def api(operation, values):
    result = subprocess.run(
        spec["cli"] + ["api"],
        input=json.dumps({"version": 1, "operation": operation, "input": values}),
        text=True,
        capture_output=True,
        timeout=5,
        check=True,
    )
    return json.loads(result.stdout)


def received():
    items = api("requests.list", {"recipientId": spec["recipient"], "limit": 50})["items"]
    return [
        api("requests.show", {"requestId": item["requestId"]})
        for item in items
        if item["sender"].get("identityId") == spec["sender"]
    ]


def stop_and_resume():
    mark = len(output)
    os.kill(board_pid, signal.SIGTSTP)
    wait_for(lambda: board_stopped, "board suspended", 10)
    if termios.tcgetattr(slave) != original_modes:
        raise AssertionError("suspension did not restore the terminal")
    read_output(0)
    if spec["enhanced"] and b"\x1b[<1u" not in output[mark:]:
        raise AssertionError("suspension did not pop the owned keyboard mode")
    before_queries = queries
    mark = len(output)
    os.kill(board_pid, signal.SIGCONT)
    wait_for(
        # Fresh output and raw modes prove re-entry independently of whether
        # waitpid reports WCONTINUED on this host.
        lambda: b"\x1b[>1u" in output[mark:]
        and termios.tcgetattr(slave) != original_modes
        and fresh_text("Message"),
        "resumed composer and keyboard mode",
    )
    if queries != before_queries:
        raise AssertionError("resume queried while the input reader was active")


try:
    command = spec["cli"] + [
        "ops", "ui", "--squad", "product", "--view", "keyboard", "--reset-view"
    ]
    # Keep the controlling session alive while checking the exited board's modes.
    # Darwin revokes a slave tty when its session leader exits.
    owner = """
import json, os, subprocess, sys
status, release = int(sys.argv[1]), int(sys.argv[2])
process = subprocess.Popen(json.loads(sys.argv[3]))
def report(value):
    os.write(status, (json.dumps(value) + "\\n").encode())
report({"pid": process.pid})
while True:
    _, value = os.waitpid(process.pid, os.WUNTRACED | os.WCONTINUED)
    if os.WIFSTOPPED(value):
        report({"stopped": True})
    elif os.WIFCONTINUED(value):
        report({"stopped": False})
    else:
        process.returncode = os.waitstatus_to_exitcode(value)
        report({"returncode": process.returncode})
        break
os.read(release, 1)
sys.exit(0)
"""
    child = subprocess.Popen(
        [sys.executable, "-c", owner, str(status_write), str(release_read), json.dumps(command)],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        preexec_fn=controlling_terminal,
        pass_fds=(status_write, release_read),
        env={**os.environ, "TERM": "xterm-256color", "COLORTERM": "truecolor"},
    )
    wait_for(lambda: fresh_text("Roster preview"), "initial squad preview")
    mark = len(output)
    os.write(master, b"\x1b[C")
    wait_for(lambda: fresh_text("▾", mark), "expanded squad")
    mark = len(output)
    os.write(master, b"\x1b[F")
    wait_for(lambda: fresh_text("Message", mark), "selected member composer")
    mark = len(output)
    os.write(master, b"\r")
    caption = "Shift+Enter new line" if spec["enhanced"] else "Alt+Enter new line"
    wait_for(lambda: fresh_text(caption, mark), "capability-derived newline hint")
    os.write(master, b"first")
    if spec["lifecycle"] == "suspend":
        wait_for(lambda: fresh_text("first", mark), "retained draft before suspension")
        stop_and_resume()
    if spec["enhanced"]:
        os.write(master, b"\x1b[13;2u")
        expected = "first\nsecond"
    else:
        os.write(master, b"\x1b\r")
        os.write(master, b"middle\x0a")
        expected = "first\nmiddle\nsecond"
    os.write(master, b"second")
    # Newline keys cannot silently dispatch. Inspect Core before plain Enter.
    if received():
        raise AssertionError("a newline key submitted a request")
    os.write(master, b"\r")
    requests = []

    def submitted():
        global requests
        requests = received()
        return bool(requests)

    wait_for(submitted, "plain Enter submitted the multiline message")
    if len(requests) != 1 or requests[0]["prompt"].get("message") != expected:
        raise AssertionError(f"wrong durable message: {requests!r}")
    if spec["lifecycle"] == "reload":
        # Retained drafts correctly block exec: submit first, then leave the
        # empty composer. The reload must pop and renegotiate its own mode.
        mark = len(output)
        before_queries = queries
        os.write(master, b"\x1b[27u")
        subprocess.run(
            spec["cli"] + ["ops", "ui", "--reload-all"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
            check=True,
        )
        wait_for(
            lambda: queries > before_queries
            and fresh_text("Board reloaded")
            and fresh_text("⏎ write")
            and not fresh_text("Sending…"),
            "reload after submitted draft",
            30,
        )
        if b"\x1b[<1u" not in output[mark:] or b"\x1b[>1u" not in output[mark:]:
            raise AssertionError("exec handoff did not restore and re-enable keyboard mode")
        os.write(master, b"\r")
        wait_for(lambda: fresh_text(caption), "composer after exec")
        if len(received()) != 1:
            raise AssertionError("reload dispatched the message twice")
    if spec["lifecycle"] == "term":
        os.kill(board_pid, signal.SIGTERM)
    else:
        os.write(master, b"\x03")
    deadline = time.monotonic() + 15
    while board_status is None and time.monotonic() < deadline:
        read_output(0.1)
    if board_status is None:
        raise AssertionError("board did not exit")
    read_output(0)
    if termios.tcgetattr(slave) != original_modes:
        raise AssertionError("exit did not restore the terminal")
    pushes = bytes(output).count(b"\x1b[>1u")
    pops = bytes(output).count(b"\x1b[<1u")
    if spec["enhanced"]:
        if pushes == 0 or pushes != pops:
            raise AssertionError(f"unbalanced keyboard mode: {pushes}/{pops}")
    elif pushes or pops:
        raise AssertionError("unsupported terminal changed the keyboard stack")
    if board_status != (143 if spec["lifecycle"] == "term" else 0):
        raise AssertionError(f"unexpected board exit {board_status}")
    os.write(release_write, b"x")
    child.wait(timeout=5)
    print(
        json.dumps(
            {
                "message": expected,
                "queries": queries,
                "pushes": pushes,
                "pops": pops,
                "terminalRestored": True,
                "returncode": board_status,
            }
        )
    )
finally:
    if child is not None:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=5)
    os.close(master)
    os.close(slave)
    for descriptor in (status_read, status_write, release_read, release_write):
        os.close(descriptor)
