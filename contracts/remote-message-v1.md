# Remote message v1

`tmt talk <target> <message> --remote <peer>` sends one request to an agent on a paired Remote
peer (#4536). Core prepares and records the request and owns its receipts and retention; Remote
owns the wire to the peer. The boundary between them is one command, `tmt-remote message --json`:
one JSON document on stdin, one on stdout.

## Command

- The sender is confirmed (a verified caller or an existing `--identity`), else
  `REMOTE_SENDER_REQUIRED`. The request is never anonymous and never an announcement.
- The target is a name, or a canonical lowercase UUID with `--expect-name <name>`, which pins it:
  the peer refuses unless that UUID still has that name. An unpinned name must resolve to one live
  identity on the peer.
- `--remote` implies detach and takes no `--passthrough`, `--urgent`, `--no-preamble`, `--timeout`,
  `--delay`, `--room` or `--kind`.
- The peer name is 1–64 bytes and a target name 1–128 bytes, without controls; Remote owns the
  peer grammar and the message size cap (`REMOTE_CLOUD_MESSAGE_TOO_LARGE`).
- Core finds `tmt-remote` on `PATH` like any extension and requires the installed release to
  declare `messageProtocols` containing `1` ([extension API](extension-api.md)). A missing or
  older Remote gives `REMOTE_UNAVAILABLE` before anything is stored. Core never calls `message` to
  probe.

## Request

```json
{"version":1,"remote":"bob","requestId":"<uuid>","createdAtMs":1700000000000,
 "from":{"id":"<uuid>","name":"Ben"},"target":{"name":"reviewer"},"message":"…",
 "reply":{"requestId":"req_<uuid>","receipt":"v2_…"}}
```

- `requestId` is the request's UUID without its `req_` prefix. `createdAtMs` is its preparation
  time, identical on any repeat. `from` is the confirmed sender as stored at preparation.
- `target` is `{"name"}`, or `{"id","name"}` for a UUID target (`name` is the expected name).
- `reply` is the request's ordinary receipt, kept locally by Remote and never sent to the peer or
  logged. When the peer's final arrives Remote submits it once with
  `tmt reply <requestId> --receipt <receipt> --stdin`; it then appears in `tmt result`,
  `tmt x` and the answers view like any final.

## Answer

`{"version":1,"state":"…","remote":"bob","target":{…},"reason":"CODE"}` with `reason` an optional
fixed uppercase code. Core accepts only `version` 1 and the requested `remote`.

| `state` | Request status | Meaning |
| --- | --- | --- |
| `sent` | sent | Stored in the peer's inbox; not yet delivered to its agent. |
| `held` | queued | Held for the peer owner's approval. |
| `refused` | failed | Refused before any effect. |
| `unavailable` | failed | No route; nothing delivered. |
| `unconfirmed` | uncertain | The peer may have processed it. |

A nonzero exit with `{"error":{"code","message"}}` means Remote refused before any effect: the
request fails with that code. A spawn failure fails as `REMOTE_UNAVAILABLE`. A timeout (60
seconds), a crash, a nonzero exit without that envelope, or any other output leaves the request
uncertain (`REMOTE_UNCONFIRMED`, `deliveryState: "uncertain"`).

## Never retried

Core sends each request once. Uncertain delivery is never resent; a late final still arrives by
the request's receipt. A failed request takes no final.
