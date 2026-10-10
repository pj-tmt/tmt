#!/usr/bin/env python3
"""Independent oracle for the Colab Firestore declaration envelope.

Builds the exact `tmt colab deploy-declaration --json` reply from the two embedded source
files with Python stdlib only (hashlib, json); it imports no Rust code. Without `--write` it
verifies that the checked-in vector, and the digest recorded inside declaration.json, equal
what the sources produce. `--write` pins admission.digest in declaration.json to the
Rules bytes and regenerates the vector; run it only after reviewing a source change, then
regenerate the composed golden trio with Remote's `compose_firestore` example (README).
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
DIGEST_LINE = re.compile(r'("digest": ")[0-9a-f]{64}(")')


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def pinned_declaration(declaration: str, artifact: str) -> str:
    updated, count = DIGEST_LINE.subn(lambda m: m.group(1) + sha256(artifact) + m.group(2), declaration)
    if count != 1:
        sys.exit("declaration.json must carry exactly one admission digest line")
    return updated


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
    if write:
        declaration_path.write_text(declaration, encoding="utf-8")
        VECTOR.write_text(expected, encoding="utf-8")
        return 0
    if declaration_path.read_text(encoding="utf-8") != declaration:
        sys.exit("declaration.json admission.digest is stale; rerun with --write")
    if VECTOR.read_text(encoding="utf-8") != expected:
        sys.exit("deploy-declaration-v1.json is stale; rerun with --write")
    return 0


if __name__ == "__main__":
    sys.exit(main())
