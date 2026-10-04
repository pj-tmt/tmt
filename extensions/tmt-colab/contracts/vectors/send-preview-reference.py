#!/usr/bin/env python3
"""Independent send-preview framing/signature oracle; public RFC 8032 fixture key.

Regeneration uses the existing Python cryptography tooling, never product code.
Tests consume the frozen JSON without Python. Use --write only after review.
"""
import argparse
import base64
import hashlib
import json
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


def uid(n):
    return f"00000000-0000-4000-8000-{n:012d}"


def lp(value):
    if isinstance(value, str):
        value = value.encode("utf-8")
    return len(value).to_bytes(4, "big") + value


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def vector():
    # Exact controls/Unicode are intentional fixture data, not repository prose.
    message = ("Page: Shared page\nLink: https://example.test/x/colab/#space=" + "a" * 32 + "&path=%2Fpages%2F" + uid(1) + "\n\nQuote:\n"
               "<script>untrusted()</script>\r\n😀\0\u202e\n\nComment:\nKeep ! and café exact")
    final = message.encode("utf-8")
    final_digest = hashlib.sha256(final).digest()
    issued, validity = 1791004000000, 3600000
    fields = ["tmt-colab-send-v1", "1", "a" * 32, uid(1), uid(2),
              (1).to_bytes(4, "big") + lp(uid(3)), uid(5), uid(6), uid(9),
              final_digest, uid(4), "1", "none", str(issued), str(issued + validity)]
    canonical = b"".join(lp(value) for value in fields)
    seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
    key = Ed25519PrivateKey.from_private_bytes(seed)
    return {"provenance": "Independent Python LP/SHA-256/cryptography Ed25519; RFC 8032 test 1 seed",
            "seed": seed.hex(), "publicKey": key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex(),
            "issuedAt": issued, "validityMs": validity, "finalBytes": b64(final),
            "finalDigest": b64(final_digest), "input": b64(canonical), "signature": b64(key.sign(canonical))}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    target = Path(__file__).with_name("send-preview-v1.json")
    expected = json.dumps(vector(), indent=2, ensure_ascii=True) + "\n"
    if args.write:
        target.write_text(expected)
    else:
        assert target.read_text() == expected, "Frozen send-preview vector differs"
        print("send-preview vector matches independent oracle")
