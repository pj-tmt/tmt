#!/usr/bin/env python3
"""Independent page-export oracle: conversations projection, JSON, Markdown and manifest.

Implements the colab-v1 "Plaintext page export" and "Conversation export" rules from
the written contract with Python stdlib plus `cryptography` Ed25519 (public RFC 8032
fixture seeds). It imports no Rust or browser code. Tests consume the frozen JSON
without Python. Regenerate only after review: `export-reference.py --write`.
"""
import argparse
import base64
import hashlib
import json
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

SPACE = "a" * 32
EPOCH = "9007199254740995"
MAX_AT = 8_640_000_000_000_000
SEEDS = {
    1: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",  # RFC 8032 test 1
    2: "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",  # RFC 8032 test 2
}


def uid(n):
    return f"00000000-0000-4000-8000-{n:012d}"


def lp(value):
    if isinstance(value, str):
        value = value.encode("utf-8")
    return len(value).to_bytes(4, "big") + value


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def unb64(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def key_of(slot):
    return Ed25519PrivateKey.from_private_bytes(bytes.fromhex(SEEDS[slot]))


def public_hex(slot):
    return key_of(slot).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex()


def sign_ask(slot, sender, operation, thread, message_ids, message, issued, validity=3_600_000):
    final = message.encode("utf-8")
    ids = len(message_ids).to_bytes(4, "big") + b"".join(lp(i) for i in message_ids)
    fields = ["tmt-colab-send-v1", "1", SPACE, uid(1), thread, ids, uid(5), uid(6), operation,
              hashlib.sha256(final).digest(), sender, "1", "none", str(issued), str(issued + validity)]
    canonical = b"".join(lp(f) for f in fields)
    return {"operationId": operation, "senderDevice": sender, "input": b64(canonical),
            "signature": b64(key_of(slot).sign(canonical)), "finalBytes": b64(final)}


# ---- fixture ---------------------------------------------------------------------------

W1, W2, W3 = uid(4), uid(14), uid(15)


def scope(kind, writer, revision, at, name, deleted=False, page=None):
    return {"version": 1, "kind": kind, "spaceId": SPACE, "pageId": page or uid(1), "epoch": EPOCH,
            "senderDevice": writer, "revision": str(revision), "deleted": deleted, "deviceName": name, "at": str(at)}


def thread(writer, tid, revision, at, name, anchor=None, resolved=False, deleted=False, page=None):
    return {**scope("thread", writer, revision, at, name, deleted, page), "threadId": tid, "anchor": anchor,
            "resolved": resolved}


def comment(writer, mid, revision, at, name, thread_ref, body, deleted=False):
    return {**scope("comment", writer, revision, at, name, deleted), "messageId": mid,
            "thread": {"writer": thread_ref[0], "id": thread_ref[1]}, "body": body}


def put(root, record):
    ident = record.get("threadId") or record.get("messageId")
    root[f"{ident}:{record['revision']}"] = record


def ask_state(op, revision, state, request=None, reason=None):
    return {"version": 1, "kind": "ask-state", "operationId": op, "revision": str(revision), "state": state,
            "requestId": request, "reason": reason}


def fixture_own():
    T1, T2, T3 = uid(2), uid(17), uid(18)
    first = {"writer": W1, "id": T1}
    own = {w: {"threads": {}, "messages": {}, "intents": {}, "replies": {}} for w in (W1, W2, W3)}
    anchor = {"exact": "Keep ``` fences & <b>exact</b>\n😀", "prefix": "before ", "suffix": " after"}
    put(own[W1]["threads"], thread(W1, T1, 1, 1791003800000, "Ben's `laptop`", anchor))
    put(own[W1]["threads"], thread(W1, T1, 2, 1791003900000, "Ben's `laptop`", anchor, resolved=True))
    put(own[W1]["messages"], comment(W1, uid(3), 1, 1791003800000, "Ben's `laptop`", (W1, T1), "first draft"))
    put(own[W1]["messages"], comment(
        W1, uid(3), 2, 1791003850000, "Ben's `laptop`", (W1, T1),
        "Keep ! and café exact\r\n```not a fence```\n````‮\u0000 end"))
    put(own[W2]["messages"], comment(W2, uid(16), 1, 1791003860000, "Dev\nphone", (W1, T1), "Agreed — ✅"))
    put(own[W2]["messages"], comment(W2, uid(20), 1, MAX_AT, "Dev\nphone", (W1, T1), "From the far future"))
    put(own[W2]["threads"], thread(W2, T2, 1, 1791003870000, "Dev\nphone"))
    put(own[W2]["messages"], comment(W2, uid(21), 1, 1791003870000, "Dev\nphone", (W2, T2), "to be removed"))
    put(own[W2]["messages"], comment(W2, uid(21), 2, 1791003880000, "Dev\nphone", (W2, T2), "", deleted=True))
    put(own[W2]["threads"], thread(W2, T3, 1, 1791003890000, "Dev\nphone"))
    put(own[W2]["threads"], thread(W2, T3, 2, 1791003895000, "Dev\nphone", deleted=True))
    # Never exported: a thread for another page, a writer without a historical key.
    put(own[W2]["threads"], thread(W2, uid(30), 1, 1791003900000, "Dev\nphone", page=uid(99)))
    put(own[W3]["threads"], thread(W3, uid(31), 1, 1791003900000, "Stranger"))
    # Ask 1: Ben's accepted ask with a stored reply (the independent send-preview vector).
    message = ("Page: Shared page\nLink: https://example.test/x/colab/#space=" + SPACE + "&path=%2Fpages%2F" + uid(1) +
               "\n\nQuote:\n<script>untrusted()</script>\r\n😀\0‮\n\nComment:\nKeep ! and café exact")
    ask1 = sign_ask(1, W1, uid(9), T1, [uid(3)], message, 1791004000000)
    request = "req_" + uid(10)
    own[W1]["intents"][uid(9)] = {"version": 1, "kind": "ask", "signed": ask1, "agentName": "Reviewer `bot`",
                                  "deviceName": "Ben's `laptop`"}
    for state in (ask_state(uid(9), 1, "dispatching"), ask_state(uid(9), 2, "accepted", request)):
        own[W1]["messages"][f"{uid(9)}:{state['revision']}"] = state
    own[W1]["replies"][uid(9)] = {"version": 1, "kind": "ask-reply", "operationId": uid(9), "requestId": request,
                                  "agentId": uid(6), "body": "Done ✅\n````\nnested\n````"}
    # Ask 2: a second writer's ask that never reached a state (uncertain, no reply).
    ask2 = sign_ask(2, W2, uid(19), T1, [uid(3), uid(16)], "Second question", 1791004100000)
    own[W2]["intents"][uid(19)] = {"version": 1, "kind": "ask", "signed": ask2, "agentName": "", "deviceName": ""}
    # Never exported: signed with another writer's key, and an unsigned-by-state reply.
    forged = sign_ask(1, W2, uid(22), T1, [uid(3)], "forged", 1791004200000)
    own[W2]["intents"][uid(22)] = {"version": 1, "kind": "ask", "signed": forged, "agentName": "x", "deviceName": "y"}
    return own


# ---- projection (colab-v1 "Conversation export") --------------------------------------------

DEVICE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
DECIMAL = re.compile(r"^[1-9][0-9]*$")
STATES = ["dispatching", "held", "accepted", "uncertain", "failed", "refused", "cancelled", "expired", "abandoned"]


def ok(condition):
    if not condition:
        raise ValueError("invalid")


def utf8_len(value):
    return len(value.encode("utf-8"))


def exact(value, keys):
    ok(isinstance(value, dict) and sorted(value) == sorted(keys))


def validate_selector(value):
    exact(value, ["exact", "prefix", "suffix"])
    ok(value["exact"] != "" and utf8_len(value["exact"]) <= 16384)
    for context in (value["prefix"], value["suffix"]):
        ok(len(context) <= 32 and utf8_len(context) <= 128)


def validate_discussion(root, key, r):
    scope_keys = ["version", "kind", "spaceId", "pageId", "epoch", "senderDevice", "revision", "deleted",
                  "deviceName", "at"]
    if r.get("kind") == "thread":
        exact(r, scope_keys + ["threadId", "anchor", "resolved"])
        ok(root == "threads" and isinstance(r["resolved"], bool) and DEVICE.match(r["threadId"]))
        if r["anchor"] is not None:
            validate_selector(r["anchor"])
        ok(not r["deleted"] or r["anchor"] is None)
        ident = r["threadId"]
    else:
        exact(r, scope_keys + ["messageId", "thread", "body"])
        ok(root == "messages" and r["kind"] == "comment" and DEVICE.match(r["messageId"]))
        exact(r["thread"], ["writer", "id"])
        ok(DEVICE.match(r["thread"]["writer"]) and DEVICE.match(r["thread"]["id"]))
        ok(isinstance(r["body"], str) and utf8_len(r["body"]) <= 16384)
        ok(r["body"] == "" if r["deleted"] else r["body"] != "")
        ident = r["messageId"]
    ok(r["version"] == 1 and isinstance(r["deleted"], bool) and re.match(r"^[a-z2-7]{32}$", r["spaceId"]))
    ok(DEVICE.match(r["pageId"]) and DEVICE.match(r["senderDevice"]) and DECIMAL.match(r["epoch"]))
    ok(DECIMAL.match(r["revision"]) and utf8_len(r["deviceName"]) <= 128)
    ok(re.match(r"^(0|[1-9][0-9]*)$", r["at"]) and int(r["at"]) <= MAX_AT)
    ok(key == f"{ident}:{r['revision']}")


def latest(records):
    records = sorted(records, key=lambda r: int(r["revision"]))
    first = records[0]
    if first["revision"] != "1" or first["deleted"]:
        return None
    previous = first
    for record in records[1:]:
        if int(record["revision"]) != int(previous["revision"]) + 1 or previous["deleted"]:
            return None
        if record["kind"] == "comment" and previous["kind"] == "comment" and record["thread"] != previous["thread"]:
            return None
        previous = record
    return previous


def project_threads(own, keys):
    threads, comments = {}, []
    for writer, roots in own.items():
        if writer not in keys:
            continue
        groups = {}
        for root in ("threads", "messages"):
            for key, value in roots[root].items():
                if not isinstance(value, dict) or value.get("kind") not in ("thread", "comment"):
                    continue
                try:
                    validate_discussion(root, key, value)
                    ok(value["senderDevice"] == writer and value["spaceId"] == SPACE and value["pageId"] == uid(1)
                       and value["epoch"] == EPOCH)
                except ValueError:
                    continue
                ident = value["threadId"] if value["kind"] == "thread" else value["messageId"]
                groups.setdefault(f"{value['kind']}:{ident}", []).append(value)
        for records in groups.values():
            record = latest(records)
            if record is None:
                continue
            if record["kind"] == "thread":
                threads[f"{writer}:{record['threadId']}"] = {
                    "writer": writer, "id": record["threadId"], "revision": record["revision"],
                    "anchor": record["anchor"], "resolved": record["resolved"], "deleted": record["deleted"],
                    "deviceName": record["deviceName"], "at": record["at"], "comments": []}
            else:
                comments.append(({"writer": writer, "id": record["messageId"], "revision": record["revision"],
                                  "deleted": record["deleted"], "body": record["body"],
                                  "deviceName": record["deviceName"], "at": record["at"]},
                                 f"{record['thread']['writer']}:{record['thread']['id']}"))
    for item, target in comments:
        if target in threads:
            threads[target]["comments"].append(item)
    out = sorted(threads.values(), key=lambda t: f"{t['writer']}:{t['id']}")
    for t in out:
        t["comments"].sort(key=lambda c: f"{c['writer']}:{c['id']}")
    return out


def decode_ask(signed):
    exact(signed, ["operationId", "senderDevice", "input", "signature", "finalBytes"])
    ok(DEVICE.match(signed["operationId"]) and DEVICE.match(signed["senderDevice"]))
    raw = unb64(signed["input"])
    fields, i = [], 0
    while i < len(raw):
        n = int.from_bytes(raw[i:i + 4], "big")
        fields.append(raw[i + 4:i + 4 + n])
        i += 4 + n
    ok(len(fields) == 15 and fields[0] == b"tmt-colab-send-v1" and fields[1] == b"1")
    s = [f.decode("utf-8") if k not in (5, 9) else "" for k, f in enumerate(fields)]
    count = int.from_bytes(fields[5][:4], "big")
    ids, j = [], 4
    for _ in range(count):
        n = int.from_bytes(fields[5][j:j + 4], "big")
        ids.append(fields[5][j + 4:j + 4 + n].decode("utf-8"))
        j += 4 + n
    ok(j == len(fields[5]) and ids == sorted(set(ids)))
    issued, expires = int(s[13]), int(s[14])
    ok(expires > issued and expires - issued <= 86_400_000)
    ok(s[8] == signed["operationId"] and s[10] == signed["senderDevice"])
    final = unb64(signed["finalBytes"])
    ok(fields[9] == hashlib.sha256(final).digest())
    return {"space": s[2], "page": s[3], "thread": s[4], "messageIds": ids, "machine": s[6], "agent": s[7],
            "operationId": s[8], "senderDevice": s[10], "issuedAt": issued, "expiresAt": expires,
            "message": final.decode("utf-8")}, raw


def transition_ok(source, target):
    if source == target:
        return True
    allowed = {
        "dispatching": ["held", "accepted", "uncertain", "failed", "refused", "cancelled", "expired"],
        "held": ["accepted", "uncertain", "refused", "cancelled"],
        "uncertain": ["held", "accepted", "refused", "cancelled", "abandoned"],
    }
    return target in allowed.get(source, [])


def request_ok(value):
    ok(isinstance(value, str) and value.startswith("req_") and len(value) > 4)


def project_asks(own, keys):
    out = []
    for writer, roots in own.items():
        if writer not in keys:
            continue
        public = Ed25519PublicKey.from_public_bytes(keys[writer])
        for op, value in roots["intents"].items():
            if not isinstance(value, dict) or value.get("kind") != "ask":
                continue
            try:
                exact(value, ["version", "kind", "signed", "agentName", "deviceName"])
                ok(value["version"] == 1 and utf8_len(value["agentName"]) <= 128 and utf8_len(value["deviceName"]) <= 128)
                intent, raw = decode_ask(value["signed"])
                try:
                    public.verify(unb64(value["signed"]["signature"]), raw)
                except InvalidSignature:
                    raise ValueError("invalid")
                ok(intent["operationId"] == op and intent["senderDevice"] == writer and intent["space"] == SPACE
                   and intent["page"] == uid(1))
                view = {"writer": writer, "operationId": op, "deviceName": value["deviceName"],
                        "agentName": value["agentName"], "agent": intent["agent"], "machine": intent["machine"],
                        "thread": intent["thread"], "messageIds": intent["messageIds"],
                        "issuedAt": intent["issuedAt"], "expiresAt": intent["expiresAt"],
                        "message": intent["message"], "state": "uncertain", "reason": None, "requestId": None,
                        "reply": None}
                revision, states = "0", []
                for key, item in roots["messages"].items():
                    if not isinstance(item, dict) or item.get("kind") != "ask-state" or item.get("operationId") != op:
                        continue
                    exact(item, ["version", "kind", "operationId", "revision", "state", "requestId", "reason"])
                    ok(DECIMAL.match(item["revision"]) and item["state"] in STATES and item["version"] == 1)
                    if item["requestId"] is not None:
                        request_ok(item["requestId"])
                    ok(item["state"] != "accepted" or item["requestId"] is not None)
                    ok(item["reason"] is None or re.match(r"^[A-Z][A-Z0-9_]{0,63}$", item["reason"]))
                    ok(key == f"{op}:{item['revision']}")
                    states.append(item)
                states.sort(key=lambda s: int(s["revision"]))
                for state in states:
                    ok(revision == "0" or transition_ok(view["state"], state["state"]))
                    ok(view["requestId"] is None or state["requestId"] == view["requestId"])
                    view.update(state=state["state"], reason=state["reason"], requestId=state["requestId"])
                    revision = state["revision"]
                reply = roots["replies"].get(op)
                if reply is not None:
                    exact(reply, ["version", "kind", "operationId", "requestId", "agentId", "body"])
                    request_ok(reply["requestId"])
                    ok(reply["kind"] == "ask-reply" and reply["version"] == 1 and utf8_len(reply["body"]) <= 16384)
                    ok(view["state"] == "accepted" and reply["operationId"] == op
                       and reply["agentId"] == intent["agent"] and reply["requestId"] == view["requestId"])
                    view["reply"] = {"requestId": reply["requestId"], "agentId": reply["agentId"],
                                     "body": reply["body"]}
                out.append(view)
            except ValueError:
                continue
    out.sort(key=lambda a: f"{a['writer']}:{a['operationId']}")
    return out


# ---- rendering ------------------------------------------------------------------------------

def escapable(point, keep_layout):
    if keep_layout and point in (0x0A, 0x09):
        return False
    return (point < 0x20 or 0x7F <= point <= 0x9F or point in (0x2028, 0x2029, 0x200E, 0x200F, 0xFEFF)
            or 0x202A <= point <= 0x202E or 0x2066 <= point <= 0x2069)


def display(value, keep_layout):
    return "".join(f"\\u{{{ord(c):x}}}" if escapable(ord(c), keep_layout) else c for c in value)


def longest_run(value):
    runs = re.findall(r"`+", value)
    return max((len(r) for r in runs), default=0)


def code_span(value):
    shown = display(value, False)
    if shown == "":
        return "(none)"
    ticks = "`" * (longest_run(shown) + 1)
    pad = " " if re.search(r"^[` ]|[` ]$", shown) else ""
    return f"{ticks}{pad}{shown}{pad}{ticks}"


def fence(value):
    shown = display(value, True)
    ticks = "`" * max(3, longest_run(shown) + 1)
    return f"{ticks}\n{shown}\n{ticks}"


def when(value):
    ms = int(value)
    if ms >= 253_402_300_800_000:
        return f"unix ms {ms}"
    moment = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=ms)
    return moment.strftime("%Y-%m-%d %H:%M:%S UTC")


def render_markdown(c):
    lines = ["# Conversations", "", f"- Page: {code_span(c['title'])}", f"- Page ID: {c['pageId']}",
             f"- Space: {c['spaceId']}", f"- Epoch: {c['epoch']}",
             f"- Membership head: revision {c['membershipHead']['revision']}, {c['membershipHead']['statementHash']}",
             "", "Names and times are labels asserted by each writer. They are not verified identities or clocks.",
             "", f"## Threads ({len(c['threads'])})", ""]
    if not c["threads"]:
        lines += ["No threads.", ""]
    for index, t in enumerate(sorted(c["threads"], key=lambda t: (int(t["at"]), f"{t['writer']}:{t['id']}")), 1):
        status = "deleted" if t["deleted"] else "resolved" if t["resolved"] else "open"
        lines += [f"### Thread {index}", "", f"- Thread ID: {t['writer']}:{t['id']}", f"- Status: {status}",
                  f"- Started by: {code_span(t['deviceName'])} at {when(t['at'])}", ""]
        lines += ["Quoted text:", "", fence(t["anchor"]["exact"]), ""] if t["anchor"] else ["Quoted text: none", ""]
        for position, m in enumerate(
                sorted(t["comments"], key=lambda m: (int(m["at"]), f"{m['writer']}:{m['id']}")), 1):
            lines += [f"#### Comment {position}", "", f"- Comment ID: {m['writer']}:{m['id']}",
                      f"- By: {code_span(m['deviceName'])} at {when(m['at'])} (writer {m['writer']})",
                      f"- Revision: {m['revision']}", "", "Deleted." if m["deleted"] else fence(m["body"]), ""]
    lines += [f"## Asks ({len(c['asks'])})", ""]
    if not c["asks"]:
        lines += ["No asks.", ""]
    for index, a in enumerate(sorted(c["asks"], key=lambda a: (a["issuedAt"], f"{a['writer']}:{a['operationId']}")), 1):
        reason = f" ({a['reason']})" if a["reason"] else ""
        lines += [f"### Ask {index}", "", f"- Operation ID: {a['operationId']}",
                  f"- Asked by: {code_span(a['deviceName'])} at {when(a['issuedAt'])} (writer {a['writer']})",
                  f"- Agent: {code_span(a['agentName'])} ({a['agent']}) on machine {a['machine']}",
                  f"- Thread: {a['thread']}", f"- State: {a['state']}{reason}",
                  f"- Request ID: {a['requestId'] or 'none'}", "", "Message sent:", "", fence(a["message"]), ""]
        lines += ["Reply:", "", fence(a["reply"]["body"]), ""] if a["reply"] else ["No reply recorded.", ""]
    return "\n".join(lines)


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def vector():
    source = ("﻿<!doctype html>\r\n<p>λ 😀\u0000 & exact</p>\r\n"
              "<script>parent.postMessage({type:\"export\"},\"*\")</script>")
    title = "Exact \"title\" \\ / λ\u0000\n\t"
    head = {"revision": "9007199254740993",
            "statementHash": "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"}
    own = fixture_own()
    keys = {W1: bytes.fromhex(public_hex(1)), W2: bytes.fromhex(public_hex(2))}
    conversations = {"format": "tmt-colab-conversations", "version": 1, "spaceId": SPACE, "pageId": uid(1),
                     "title": title, "epoch": EPOCH, "membershipHead": head,
                     "threads": project_threads(own, keys), "asks": project_asks(own, keys)}
    # Key order of each row is the contract's declared order.
    json_text = compact(conversations)
    markdown = render_markdown(conversations)
    html = source.encode("utf-8")

    def info(name, data):
        return {"name": name, "sizeBytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}

    files = [info("page.html", html), info("conversations.json", json_text.encode("utf-8")),
             info("conversations.md", markdown.encode("utf-8"))]
    manifest = compact({
        "format": "tmt-colab-page-export", "version": 1, "spaceId": SPACE, "pageId": uid(1), "title": title,
        "exportedAtMs": 1700000000123, "membershipHead": head, "epoch": EPOCH, "plaintext": True,
        "discussions": {"included": True, "scope": "current-epoch", "format": "tmt-colab-conversations",
                        "version": 1},
        "files": files})
    return {
        "provenance": "Independent Python stdlib (UTF-8, hashlib SHA-256, compact JSON) with cryptography Ed25519 "
                      "over public RFC 8032 fixture seeds; export-reference.py. Unicode, control and bidi characters "
                      "are intentional exact-byte data. Shared by native and browser tests; exportedAtMs is injected.",
        "input": {"spaceId": SPACE, "pageId": uid(1), "source": source, "title": title, "exportedAtMs": 1700000000123,
                  "membershipHead": head, "epoch": EPOCH, "own": own,
                  "signingKeys": {w: k.hex() for w, k in keys.items()}},
        "conversationsJson": json_text,
        "conversationsMarkdown": markdown,
        "manifestUtf8": manifest,
        "manifestSha256": hashlib.sha256(manifest.encode("utf-8")).hexdigest(),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    target = Path(__file__).with_name("export-v1.json")
    expected = json.dumps(vector(), indent=2, ensure_ascii=True) + "\n"
    if args.write:
        target.write_text(expected)
    else:
        assert target.read_text() == expected, "Frozen export vector differs"
        print("export vector matches independent oracle")
