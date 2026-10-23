"""Public-Core fixture: remembered observation and a foreground refusal, no provider launch."""
import json
import os
import pathlib
import subprocess
import sys
import termios

spec = json.loads(pathlib.Path(sys.argv[1]).read_text())
args = sys.argv[2:]
root = pathlib.Path(spec["root"])
proof = root / "handoff.json"


def modes(fd):
    value = termios.tcgetattr(fd)
    value[-1] = [v.hex() if isinstance(v, bytes) else v for v in value[-1]]
    return value


if args and args[0] == "resume":
    with (root / "resume-calls.jsonl").open("a") as output:
        output.write(json.dumps(args) + "\n")
    if "--show" in args:
        assert args == ["resume", "--show", "--", spec["recipient"]]
        print(json.dumps({"identity": "worker", "launch": None}))
        sys.exit(0)
    assert not spec.get("unbound"), "an unbound Session action must remain copy-only"
    expected = ["resume"] + (["--move"] if spec["move"] else []) + ["--", spec["recipient"]]
    assert args == expected
    active = []
    for entry in (root / "active").iterdir():
        try:
            os.kill(int(entry.name), 0)
            active.append(int(entry.name))
        except ProcessLookupError:
            pass
    packet = {"argv": args, "pid": os.getpid(), "tty": [os.isatty(fd) for fd in (0, 1, 2)],
              "modes": modes(0), "foreground": os.tcgetpgrp(0) == os.getpgrp(),
              "activeReads": active}
    proof.write_text(json.dumps(packet))
    print("FOREGROUND_SESSION_REFUSAL", flush=True)
    # A real Core refusal must retain its own exit; no provider is ever started.
    sys.exit(23)

assert not proof.exists(), "a Core read started after the foreground handoff"
active = root / "active" / str(os.getpid())
active.write_text("read")
try:
    result = subprocess.run(spec["cli"] + args, input=sys.stdin.buffer.read(),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
    if args and args[0] == "ls" and "--json" in args and result.returncode == 0:
        value = json.loads(result.stdout)
        for row in value["identities"]:
            if row["id"] == spec["recipient"]:
                row.update(driverSupport="full", location={"host": spec.get("host", "tmux"), "session": "work", "window": "agents"},
                           resume={"driver": "codex", "session": "fixture-only", "staleAtMs": None})
                if "unbound" in spec:
                    row["session"] = {"unbound": spec["unbound"]}
        sys.stdout.write(json.dumps(value))
    else:
        sys.stdout.buffer.write(result.stdout)
    sys.stderr.buffer.write(result.stderr)
    sys.exit(result.returncode)
finally:
    active.unlink()
