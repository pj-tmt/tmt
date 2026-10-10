"""Independent projections over the envelope/declaration reference vectors.
No product code is called. Run with --check to compare, or no arguments to regenerate.
"""
import json
import sys
from pathlib import Path

here = Path(__file__).parent
run = here.parent / "deploy_run"
extensions = json.loads((here.parent / "declarations" / "plan-firestore-no-ttl.json").read_text())


def readable_bytes(value):
    for divisor, unit in [(1024 * 1024, "MiB"), (1024, "KiB")]:
        if value >= divisor:
            number = str(value // divisor) if value % divisor == 0 else f"{value / divisor:.2f}"
            return f"{number} {unit}"
    return f"{value} B"


def project(view, digest):
    record = {"deploymentId": view["deploymentId"], "binding": None, "run": None}
    projection = {"authorized": False, "plan": view, "planDigest": digest, "extensions": extensions, "record": record}
    lines = [
        "Firestore sharing plan", "Account: " + view["account"], "Project: " + view["project"],
        "Deployment: " + view["deploymentId"],
        f'Database: (default) (standard), {view["database"]["location"]}',
        "Sign-in: " + ", ".join(view["signIn"]),
        f'Rules: {view["rules"]["digest"][:12]} ({"no existing Rules" if view["rules"]["replaces"] == "none" else view["rules"]["replaces"]})',
        "Index configs: " + str(view["indexConfigs"]), "Roles: none created", "TTL: not set up",
        "Plan digest: " + digest[:12],
    ]
    replaced = view["rules"]["replacedDigest"]
    if replaced:
        lines.extend([
            f'DESTRUCTIVE: Replace the live Rules for project {view["project"]}. This affects every tenant using its Rules.',
            f"Existing Rules fingerprint: {replaced}",
            f"Authorizing plan {digest[:12]} allows this replacement.",
        ])
    lines.extend("Destructive change: " + item for item in view["destructive"])
    lines.append("Extensions and resources:")
    for extension in extensions["extensions"]:
        lines.append(f'{extension["name"]} (declaration {extension["declarationDigest"][:12]})')
        admission = extension["admission"]
        lines.append(f'  Admission: {admission["entryPoint"]} (artifact {admission["artifactDigest"][:12]})')
        for resource in extension["resources"]:
            limits = resource["limits"]
            ttl = "none" if resource["ttl"] == "none" else "not set up"
            lines.append(f'  {resource["name"]}: {resource["kind"]} at {resource["path"]}; object {readable_bytes(limits["maxObjectBytes"])}, namespace {readable_bytes(limits["maxNamespaceBytes"])}, entries {limits["maxEntries"]}, TTL: {ttl}')
            lines.extend(f'    Index: {index["field"]} {index["direction"]}' for index in resource["indexes"])
    lines.extend(f'{extension["name"]}: unavailable ({extension["reason"]})' for extension in extensions["unavailable"])
    lines.extend(["Not authorized; nothing changed in your Firebase project.", f"To deploy this plan, run the same command with --authorize {digest[:12]}"])
    return json.dumps(projection, separators=(",", ":")), "\n".join(lines) + "\n"


assert sys.argv[1:] in [[], ["--check"]]
for kind, prefix in [("fresh", "plan"), ("foreign", "foreign-plan")]:
    view = json.loads((run / f"view-{kind}.json").read_text())
    digest = (run / f"view-{kind}.sha256").read_text().strip()
    projection, human = project(view, digest)
    outputs = {f"{prefix}-human.txt": human}
    if kind == "fresh":
        outputs["plan.json"] = projection
    for name, content in outputs.items():
        path = here / name
        if sys.argv[1:]:
            assert path.read_bytes() == content.encode(), f"stale independent projection: {path}"
        else:
            path.write_text(content)
