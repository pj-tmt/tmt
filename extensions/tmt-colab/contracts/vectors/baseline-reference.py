#!/usr/bin/env python3
"""Independent minimal update-v1 oracle; no product code or third-party imports.

Pins fresh root items (plain html text, meta.title and optional publisherAgent strings) for client 1159.
Update-v1: unsigned varints, UTF-8 varstrings, content-string ref 4, content-any
ref 8 with parentSub bit 32, lib0 string tag 119, and an empty delete set.
The clock advances in UTF-16 code units, independently of Rust's text offsets.
Non-ASCII/NUL fixture text exercises exact UTF-8 and astral clock preservation.
This intentionally supports only this baseline schema, not arbitrary Yjs updates.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path


def varuint(n):
    out = bytearray()
    while n > 127:
        out.append((n & 127) | 128)
        n >>= 7
    out.append(n)
    return bytes(out)


def string(value):
    raw = value.encode("utf-8")
    return varuint(len(raw)) + raw


def update(source, title, publisher=None):
    # Empty source inserts no item. Map title is always present, including empty.
    html = bytes([4, 1]) + string("html") + string(source) if source else b""
    meta = bytes([40, 1]) + string("meta") + string("title") + bytes([1, 119]) + string(title)
    if publisher is not None:
        meta += bytes([40, 1]) + string("meta") + string("publisherAgent") + bytes([1, 119]) + string(publisher)
    return bytes([1]) + varuint((2 if source else 1) + (publisher is not None)) + varuint(1159) + bytes([0]) + html + meta + bytes([0])


def binary(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def frame(*fields):
    return b"".join(len(value).to_bytes(4, "big") + value for value in fields)


def vectors():
    result = []
    for source, title, publisher in [("", "", None), ("<p>Hello</p>\r\n", "Baseline", None), ("<p>\U0001f680 e\u0301 \x00</p>", "Title \U0001f680", None), ("<p>Annotated</p>", "Annotations", "publishing-agent"), ("", "Empty published page", "agent-\U0001f680")]:
        raw = source.encode("utf-8")
        encoded = update(source, title, publisher)
        result.append({"source": source, "title": title, "clientId": 1159,
                       "update": binary(encoded), "sourceDigest": binary(hashlib.sha256(raw).digest()),
                       "commitment": binary(hashlib.sha256(frame(b"tmt-colab-baseline-v1", b"1", raw, encoded)).digest())})
        if publisher is not None:
            result[-1]["publisherAgent"] = publisher
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    path = Path(__file__).with_name("baseline-v1.json")
    expected = json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n"
    if args.write:
        path.write_text(expected)
    elif path.read_text() != expected:
        raise SystemExit("baseline vector drift")
    print("baseline-v1: 5 independent vectors match")
