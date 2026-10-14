#!/usr/bin/env python3
"""Independent oracle for the Colab Firestore declaration envelope.

Builds the exact `tmt colab deploy-declaration --json` reply from the two embedded source
files with Python stdlib only (hashlib, json); it imports no Rust code. Without `--write` it
verifies that the checked-in vector, and the digest recorded inside declaration.json, equal
what the sources produce. `--write` pins admission.digest in declaration.json to the
Rules bytes and regenerates the vector; run it only after reviewing a source change, then
regenerate the composed golden trio with Remote's `compose_firestore` example (README).

The hosting vector is the same reply for a binary that embeds a hosted inventory: the declaration
gains a last top-level `hosting` key holding the canonical manifest verbatim (path-sorted
`{path, sha256, length, contentType, csp}`, compact, fields in that order). The manifest is built
here from a fixed fixture, so no digest is typed by hand.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCES = HERE.parent.parent / "firestore"
VECTOR = HERE / "deploy-declaration-v1.json"
HOSTING_MANIFEST = HERE / "deploy-declaration-v1.hosting.manifest.json"
HOSTING_VECTOR = HERE / "deploy-declaration-v1.hosting.json"
# The native CSP every non-renderer hosted file carries (browser_policy.rs POLICY).
CSP = (
    "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; "
    "connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; "
    "object-src 'none'; frame-ancestors 'none'"
)
# (path, contentType, bytes): a small hosted inventory, including Remote's three SDK routes.
HOSTED = [
    ("/THIRD-PARTY-NOTICES.txt", "text/plain", b"Fixture dependency notices\n"),
    ("/index.html", "text/html", b"<!doctype html><title>Colab</title><script type=module src=/sdk/remote-v1.js></script>\n"),
    ("/sdk/THIRD-PARTY-NOTICES-remote.txt", "text/plain", b"Fixture Firebase notices\n"),
    ("/sdk/remote-firebase-v1.js", "text/javascript", b"export const firebase = 'fixture';\n"),
    ("/sdk/remote-v1.js", "text/javascript", b"export const remote = 'fixture';\n"),
]
DIGEST_LINE = re.compile(r'("digest": ")[0-9a-f]{64}(")')


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def pinned_declaration(declaration: str, artifact: str) -> str:
    updated, count = DIGEST_LINE.subn(lambda m: m.group(1) + sha256(artifact) + m.group(2), declaration)
    if count != 1:
        sys.exit("declaration.json must carry exactly one admission digest line")
    return updated


def hosting_manifest() -> str:
    """The compact canonical manifest, path-sorted, fields in Remote's struct order."""
    files = [
        {
            "path": path,
            "sha256": hashlib.sha256(body).hexdigest(),
            "length": len(body),
            "contentType": content_type,
            "csp": CSP,
        }
        for path, content_type, body in sorted(HOSTED)
    ]
    return json.dumps({"version": 1, "files": files}, separators=(",", ":"), ensure_ascii=True)


def with_hosting(declaration: str, manifest: str) -> str:
    """The manifest becomes the last top-level key, inserted verbatim before the closing brace."""
    if not declaration.endswith("\n}\n"):
        sys.exit("declaration.json must end with a closing brace on its own line")
    return declaration[: -len("\n}\n")] + ',\n  "hosting": ' + manifest + "\n}\n"


def envelope(declaration: str, artifact: str) -> str:
    reply = {
        "version": 1,
        "extension": "colab",
        "backend": "firestore",
        "declaration": declaration,
        "declarationDigest": sha256(declaration),
        "artifact": artifact,
    }
    # Compact, insertion-ordered, plus the newline the command prints.
    return json.dumps(reply, separators=(",", ":"), ensure_ascii=False) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    write = parser.parse_args().write
    artifact = (SOURCES / "admission.rules").read_text(encoding="utf-8")
    declaration_path = SOURCES / "declaration.json"
    declaration = pinned_declaration(declaration_path.read_text(encoding="utf-8"), artifact)
    for name, text in (("declaration", declaration), ("artifact", artifact)):
        if not text.isascii() or len(text.encode("utf-8")) > 64 * 1024:
            sys.exit(f"{name} must be ASCII and at most 64 KiB")
    expected = envelope(declaration, artifact)
    manifest = hosting_manifest()
    hosted = with_hosting(declaration, manifest)
    if not hosted.isascii() or len(hosted.encode("utf-8")) > 64 * 1024:
        sys.exit("hosted declaration must be ASCII and at most 64 KiB")
    json.loads(hosted)  # the composed text is still one JSON object
    generated = {
        VECTOR: expected,
        HOSTING_MANIFEST: manifest + "\n",
        HOSTING_VECTOR: envelope(hosted, artifact),
    }
    if write:
        declaration_path.write_text(declaration, encoding="utf-8")
        for path, text in generated.items():
            path.write_text(text, encoding="utf-8")
        return 0
    if declaration_path.read_text(encoding="utf-8") != declaration:
        sys.exit("declaration.json admission.digest is stale; rerun with --write")
    for path, text in generated.items():
        if path.read_text(encoding="utf-8") != text:
            sys.exit(f"{path.name} is stale; rerun with --write")
    return 0


if __name__ == "__main__":
    sys.exit(main())
