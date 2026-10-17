"""Opt-in Linux PID-domain capability controls; never runs Cargo or product code."""
import ctypes
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import time

CASES = ("normal", "nonzero", "survivor", "cancel", "unknown")


def admit(case, result):
    if not result["empty"] or not result["reaped"]:
        raise RuntimeError("Domain teardown is unknown; roots retained")
    expected = {"normal": (0, False), "nonzero": (7, False),
                "survivor": (0, True), "unknown": (0, False)}
    if case == "cancel":
        matched = result["exit"] != 0 and result["cancelled"]
    else:
        matched = (result["payload_exit"], bool(result["residents"])) == expected[case]
        matched = matched and result["exit"] == (0 if case == "normal" else 1)
    if not matched or result["unknown"] != (case == "unknown"):
        raise RuntimeError("Unexpected capability result; roots retained")
    return {"control_passed": True, "payload_admitted": case == "normal"}


def members(identity):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if os.readlink(entry / "ns/pid") == identity:
                found.append(int(entry.name))
        except FileNotFoundError:
            pass
    return sorted(found)


def line(stream, transcript=None):
    data = bytearray()
    deadline = time.monotonic() + 5
    with selectors.DefaultSelector() as selector:
        selector.register(stream, selectors.EVENT_READ)
        while len(data) < 4096:
            if not selector.select(max(0, deadline - time.monotonic())):
                break
            byte = os.read(stream.fileno(), 1)
            if transcript is not None:
                transcript.write(byte)
                transcript.flush()
            if byte == b"\n":
                return json.loads(data)
            if not byte:
                break
            data.extend(byte)
    raise RuntimeError("Domain handshake failed or timed out; roots retained")


def inside(case, uid, gid):
    identity = os.readlink("/proc/self/ns/pid")
    print(json.dumps({"namespace": identity, "init": os.getpid()}), flush=True)
    if sys.stdin.readline() != "go\n" or os.getpid() != 1:
        raise RuntimeError("Namespace identity was not admitted")
    # The workload cannot regain root or use a privileged launcher.
    code = "import os; assert os.getuid() == " + str(uid) + "; "
    code += "assert os.getgid() == " + str(gid) + "; assert os.getgroups() == []; "
    code += "assert 'NoNewPrivs:\\t1' in open('/proc/self/status').read(); "
    code += "assert 'CapEff:\\t0000000000000000' in open('/proc/self/status').read(); "
    if case == "survivor":
        code += """
import socket, time
r, w = os.pipe()
if os.fork():
    os.close(w); assert os.read(r, 1) == b'1'; os._exit(0)
os.close(r); os.setsid()
if os.fork(): os._exit(0)
s = socket.socket(); s.bind(('127.0.0.1', 0)); s.listen()
os.write(w, b'1'); os.close(w); time.sleep(30)
"""
    elif case == "cancel":
        code += "import time; print(1, flush=True); time.sleep(30)"
    else:
        code += "sys_exit = 7 if " + repr(case == "nonzero") + " else 0; os._exit(sys_exit)"
    payload = subprocess.Popen([
        "setpriv", "--reuid=" + str(uid), "--regid=" + str(gid), "--clear-groups",
        "--bounding-set=-all", "--no-new-privs", sys.executable, "-c", code,
    ], stdin=subprocess.DEVNULL,
       stdout=subprocess.PIPE if case == "cancel" else subprocess.DEVNULL, stderr=sys.stderr)
    if case == "cancel" and line(payload.stdout) != 1:
        raise RuntimeError("Cancellation payload was not ready")
    print(json.dumps({"running": payload.pid}), flush=True)
    status = payload.wait(timeout=8)
    # Reap exited adopted children; live descendants remain a refusal.
    while True:
        try:
            child, _ = os.waitpid(-1, os.WNOHANG)
            if child == 0:
                break
        except ChildProcessError:
            break
    try:
        if case == "unknown":
            raise OSError("Injected teardown observation failure")
        residents = [pid for pid in members(identity) if pid != 1]
        report = {"payload_exit": status, "residents": residents, "unknown": False}
    except OSError as error:
        report = {"payload_exit": status, "residents": [], "unknown": True, "error": str(error)}
    print(json.dumps(report), flush=True)
    return int(status != 0 or bool(report["residents"]) or report["unknown"])


def qualify(output, uid, gid):
    if sys.platform != "linux" or os.geteuid() != 0 or uid <= 0 or gid <= 0:
        raise RuntimeError("Requires Linux supervisor root and an unprivileged runner identity")
    if ctypes.CDLL(None, use_errno=True).prctl(36, 1, 0, 0, 0) != 0:
        raise OSError(ctypes.get_errno(), "Cannot become the external child subreaper")
    if not output.is_absolute() or output.parent != Path("/home/runner/work/_temp"):
        raise RuntimeError("Output must be an absent direct child of the runner temporary root")
    output.mkdir(mode=0o755, parents=False, exist_ok=False)
    reports = []
    for case in CASES:
        owned = output / case
        owned.mkdir(mode=0o700)
        os.chown(owned, uid, gid)
        command = ["unshare", "--pid", "--fork", "--kill-child", "--mount-proc",
                   sys.executable, str(Path(__file__).resolve()), "inside", case, str(uid), str(gid)]
        with (output / (case + ".stderr")).open("wb") as errors, (output / (case + ".stdout")).open("wb") as transcript:
            process = subprocess.Popen(command, cwd=owned, stdin=subprocess.PIPE,
                                       stdout=subprocess.PIPE, stderr=errors, text=True, bufsize=1)
            descriptor = None
            try:
                ready = line(process.stdout, transcript)
                identity = ready["namespace"]
                if ready["init"] != 1 or identity == os.readlink("/proc/self/ns/pid"):
                    raise RuntimeError("PID namespace isolation missing")
                occupants = members(identity)
                if len(occupants) != 1:
                    raise RuntimeError("Namespace identity is ambiguous")
                descriptor = os.open("/proc/" + str(occupants[0]) + "/ns/pid", os.O_RDONLY)
                if os.readlink("/proc/self/fd/" + str(descriptor)) != identity:
                    raise RuntimeError("Namespace identity changed")
                process.stdin.write("go\n")
                process.stdin.flush()
                running = line(process.stdout, transcript)
                if running["running"] <= 1:
                    raise RuntimeError("Missing unprivileged payload")
                if case == "cancel":
                    process.kill()  # Exact owned child; --fork ignores TERM, --kill-child kills init.
                    result = {"payload_exit": None, "residents": [], "unknown": False}
                else:
                    result = line(process.stdout, transcript)
                status = process.wait(timeout=5)
                deadline = time.monotonic() + 3
                reaped = False
                reaped_children = []
                while time.monotonic() < deadline:
                    try:
                        pid, child_status = os.waitpid(-1, os.WNOHANG)
                        if pid:
                            reaped_children.append([pid, os.waitstatus_to_exitcode(child_status)])
                        if pid == 0:
                            time.sleep(0.02)
                    except ChildProcessError:
                        reaped = True
                        break
                result.update(exit=status, cancelled=case == "cancel", namespace=identity, ready=ready, running=running,
                              empty=not members(identity), reaped=reaped, reaped_children=reaped_children, command=command)
                (output / (case + ".json")).write_text(json.dumps(result, indent=2) + "\n")
                result.update(admit(case, result))
                if result["payload_admitted"]:
                    owned.rmdir()
                reports.append(result)
            except BaseException:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                raise
            finally:
                if descriptor is not None:
                    os.close(descriptor)
    (output / "qualification.json").write_text(json.dumps(reports, indent=2) + "\n")


if __name__ == "__main__":
    if len(sys.argv) == 5 and sys.argv[1] == "inside" and sys.argv[2] in CASES:
        sys.exit(inside(sys.argv[2], int(sys.argv[3]), int(sys.argv[4])))
    if len(sys.argv) != 5 or sys.argv[1] != "qualify":
        raise SystemExit("Usage: workspace-proof-domain.py qualify <absent-owned-output> <uid> <gid>")
    qualify(Path(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]))
