#!/usr/bin/env python3
"""Independent reference for the deploy plan goldens (does not use the Rust code).

Usage: plan-reference.py <physicalTtl: true|false>
Reads colab-firestore.json and notes-firestore.json with their artifacts from this
directory and prints the plan bytes; the digest is `shasum -a 256` of those bytes.
Regenerate: for each mode, write stdout to plan-firestore-{ttl,no-ttl}.json and the
digest to the matching .sha256 file.
"""
import hashlib, json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
physical_ttl = sys.argv[1] == "true"


def sha(data):
    return hashlib.sha256(data).hexdigest()


extensions = []
for name in sorted(["colab", "notes"]):
    raw = open(os.path.join(here, f"{name}-firestore.json"), "rb").read()
    decl = json.loads(raw)
    artifact = open(os.path.join(here, decl["admission"]["artifact"]), "rb").read()
    assert sha(artifact) == decl["admission"]["digest"]
    resources = []
    for r in sorted(decl["resources"], key=lambda r: r["name"]):
        if r["ttlField"] is None:
            ttl = "none"
        else:
            ttl = "provisioned" if physical_ttl else "not-provisioned"
        resources.append({
            "name": r["name"],
            "kind": r["kind"],
            "path": f"x/{name}/{r['path']}",
            "limits": {
                "maxObjectBytes": r["limits"]["maxObjectBytes"],
                "maxNamespaceBytes": r["limits"]["maxNamespaceBytes"],
                "maxEntries": r["limits"]["maxEntries"],
            },
            "ttlField": r["ttlField"],
            "ttl": ttl,
            "indexes": sorted(r["indexes"], key=lambda i: (i["field"], i["direction"])),
        })
    extensions.append({
        "name": name,
        "declarationDigest": sha(raw),
        "admission": {
            "artifactDigest": sha(artifact),
            "entryPoint": decl["admission"]["entryPoint"],
        },
        "resources": resources,
    })

plan = {
    "version": 1,
    "profile": "sharing",
    "backend": "firestore",
    "capabilities": {"physicalTtl": physical_ttl},
    "extensions": extensions,
    "unavailable": [],
}
sys.stdout.write(json.dumps(plan, separators=(",", ":")))
