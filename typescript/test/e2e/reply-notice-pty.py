"""Scenario-owned real terminal client; JSON commands inject actual key bytes."""
import fcntl
import json
import os
import signal
import subprocess
import sys
import termios
import threading

master, slave = os.openpty()
client = None

def terminate(_signal, _frame):
    raise SystemExit(1)

signal.signal(signal.SIGTERM, terminate)

def terminal():
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

def drain():
    try:
        while os.read(master, 65536):
            pass
    except OSError:
        pass

try:
    client = subprocess.Popen(
        ["tmux", "-S", sys.argv[1], "attach-session", "-t", sys.argv[2]],
        stdin=slave, stdout=slave, stderr=slave,
        env={**os.environ, "TERM": "xterm"}, preexec_fn=terminal,
    )
    threading.Thread(target=drain, daemon=True).start()
    print(json.dumps({"pid": client.pid}), flush=True)
    for line in sys.stdin:
        command = json.loads(line)
        if command == "close":
            break
        if command != "key":
            raise ValueError("Unknown PTY command")
        # Ctrl-U is a real key that clears canonical input without submitting
        # a request to the mock agent or leaving text ahead of a later notice.
        os.write(master, b"\x15")
        print(json.dumps({"key": True}), flush=True)
finally:
    if client is not None:
        if client.poll() is None:
            client.terminate()
        try:
            client.wait(timeout=2)
        except subprocess.TimeoutExpired:
            client.kill()
            client.wait(timeout=2)
    os.close(master)
    os.close(slave)
