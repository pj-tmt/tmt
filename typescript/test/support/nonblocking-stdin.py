"""Deliver stdin only after a real EAGAIN observation; own and reap the test child."""
import json
import os
import select
import subprocess
import sys
import time

configuration = json.load(sys.stdin)
reader, writer = os.pipe()
os.set_blocking(reader, False)
os.set_blocking(writer, False)
child = None
prefix = b""
ready = False
try:
    child = subprocess.Popen(configuration["argv"], stdin=reader,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    os.close(reader)
    reader = None
    readable, _, _ = select.select([child.stderr], [], [], 5)
    if readable:
        prefix = child.stderr.readline()
        ready = prefix == b"stdin-eagain\n"
    if ready and configuration.get("input") is not None:
        payload = memoryview(configuration["input"].encode("utf-8"))
        offset = 0
        deadline = time.monotonic() + 15
        while offset < len(payload):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Non-blocking producer write deadline")
            _, writable, _ = select.select([], [writer], [], remaining)
            if not writable:
                raise TimeoutError("Non-blocking producer never became writable")
            try:
                offset += os.write(writer, payload[offset:])
            except BlockingIOError:
                continue
        os.close(writer)
        writer = None
    stdout, stderr = child.communicate(timeout=5)
    if not ready:
        stderr = prefix + stderr
    print(json.dumps({"ready": ready, "status": child.returncode,
                      "stdout": stdout.decode("utf-8"),
                      "stderr": stderr.decode("utf-8", errors="replace")}))
finally:
    if reader is not None:
        os.close(reader)
    if writer is not None:
        os.close(writer)
    if child is not None:
        if child.poll() is None:
            child.kill()
        child.communicate(timeout=2)
