"""Independent slice-2 projections over the slice-1 reference envelope and declarations.
No Rust code under test is called. Run here with python3 reference.py.
"""
import json
from pathlib import Path

here = Path(__file__).parent
run = here.parent / "deploy_run"
view = json.loads((run / "view-fresh.json").read_text())
digest = (run / "view-fresh.sha256").read_text()
extensions = json.loads((here.parent / "declarations" / "plan-firestore-no-ttl.json").read_text())
record = {"deploymentId": view["deploymentId"], "binding": None, "run": None}
projection = {"authorized": False, "plan": view, "planDigest": digest, "extensions": extensions, "record": record}
(here / "plan.json").write_text(json.dumps(projection, separators=(",", ":")))
lines = [
    "Firestore sharing plan",
    "Account: " + view["account"],
    "Project: " + view["project"],
    "Deployment: " + view["deploymentId"],
    "Database: (default) (standard), " + view["database"]["location"],
    "Sign-in: " + ", ".join(view["signIn"]),
    "Rules: " + view["rules"]["digest"] + " (replaces none)",
    "Index configs: " + str(view["indexConfigs"]),
    "Roles: none created",
    "Physical TTL: not provisioned",
    "Plan digest: " + digest,
    "Extensions and resources:",
    json.dumps(extensions, indent=2),
    "Not authorized; nothing changed in your Firebase project.",
    "Authorize this exact plan with --authorize <plan-digest-prefix>.",
]
(here / "plan-human.txt").write_text("\n".join(lines) + "\n")
