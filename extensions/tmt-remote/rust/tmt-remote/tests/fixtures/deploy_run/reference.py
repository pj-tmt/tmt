"""Independent reference for the deploy envelope (#2164). It builds the owner-visible view,
its canonical bytes, the plan digest and the deployed Rules bytes from first principles,
without the Rust code under test. Run from this directory: python3 reference.py"""
import hashlib, json, pathlib

here = pathlib.Path(__file__).parent
decl = here.parent / "declarations"

ACCOUNT = "owner@example.test"
PROJECT = "demo-remote-1"
DEPLOYMENT = "3f2b8c1e-5d4a-4e7b-9c1d-2a6f8e0b4c11"
LOCATION = "asia-east1"
PROVIDERS = ["google.com", "anonymous"]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


body = (here / "rules-body.rules").read_bytes()
foreign = (here / "foreign.rules").read_bytes()
marker = "// tmt-remote deployment %s rules %s\n" % (DEPLOYMENT, sha(body))
deployed = marker.encode() + body

extension_plan = json.loads((decl / "plan-firestore-no-ttl.json").read_text())
extension_digest = (decl / "plan-firestore-no-ttl.sha256").read_text()
assert sha((decl / "plan-firestore-no-ttl.json").read_bytes().rstrip(b"\n")) == extension_digest

providers = sorted(PROVIDERS)
steps = ["database"] + ["sign-in:" + p for p in providers]
for extension in extension_plan["extensions"]:
    for resource in extension["resources"]:
        for index in resource["indexes"]:
            steps.append("index:%s#%s:%s" % (resource["path"], index["field"], index["direction"]))
steps += ["rules", "verify"]
index_configs = sum(1 for s in steps if s.startswith("index:"))


def view(replaces, replaced_digest):
    return {
        "version": 1,
        "backend": "firestore",
        "profile": "sharing",
        "account": ACCOUNT,
        "project": PROJECT,
        "deploymentId": DEPLOYMENT,
        "database": {"name": "(default)", "edition": "standard", "location": LOCATION},
        "signIn": providers,
        "rules": {"digest": sha(body), "replaces": replaces, "replacedDigest": replaced_digest},
        "indexConfigs": index_configs,
        "steps": steps,
        "extensionPlanDigest": extension_digest,
        "destructive": [] if replaced_digest is None else ["replaces-rules:" + replaced_digest],
    }


for name, v in [("fresh", view("none", None)), ("foreign", view("foreign", sha(foreign)))]:
    out = compact(v)
    (here / ("view-%s.json" % name)).write_bytes(out)
    (here / ("view-%s.sha256" % name)).write_text(sha(out))
(here / "deployed.rules").write_bytes(deployed)
print("ok", len(steps), "steps")
