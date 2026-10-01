# Codex native channel contract

Status: receipt/transport groundwork in #736, under #719 and #329. The Codex
runtime does not register or invoke this channel yet. Record, endpoint, lease
and consumer integration remain separate slices; no user-facing native delivery
or live foreground continuity is claimed here.

## Delivery receipt

One delivery creates one immutable `thread/queue/add` request for the enrolled
thread. A correlated response with the exact caller ID and input echo records
queue acceptance, not processing or completion. Only a durable `tmt reply`
completes the TMT request. Caller IDs are not provider deduplication keys.

A correlated error alone cannot prove absence of queue side effects. The adapter
recognizes only these exact, thread-specific pre-enqueue rejection signatures:

- Code -32600: ``session THREAD is archived. Run `codex unarchive THREAD` to unarchive it first.``
- Code -32603: `failed to read thread: invalid thread-store request: no rollout found for thread id THREAD`

All other errors, including generic correlated internal errors, remain uncertain.
Missing, malformed, mismatched or ambiguous receipts also remain uncertain.
Neither refusal nor uncertainty authorizes resend or paste fallback. A proven
pre-write transport refusal remains distinct from a provider rejection received
after writing a request.

The source boundary is OpenAI Codex revision
[33aea33b39a5e35c78de75c332106479b16c6994](https://github.com/openai/codex/blob/33aea33b39a5e35c78de75c332106479b16c6994/codex-rs/app-server/src/request_processors/thread_queue_processor.rs):
`add` calls `require_thread` before `enqueue`; archived and failed thread reads
return from that prerequisite. Conversion to an API submission occurs after
`enqueue` and can itself fail. These are source observations, not a claim that
every internal error is pre-enqueue. Exact archived/deleted responses were also
observed in the isolated 0.159.2 spike. Retained evidence contains selected native
events; full raw logs were removed. The source revision and retained runtime
observations are distinct provenance, not a complete event stream or proof of
all future provider versions. Message drift fails closed to uncertainty.

## Transport and qualification

The Codex adapter alone uses synchronous tungstenite 0.30.0 with default features
disabled and only `handshake` enabled. No TLS, async runtime or handwritten
WebSocket/SHA1 implementation is added. Core and shared driver ports do not
reference this dependency.

The future enrollment owner must establish ownership of the exact endpoint
process and capability before constructing a client. A loopback address or an
arbitrary endpoint's self-report is insufficient authority. The client accepts
only explicit loopback IPv4/nonzero ports and a bounded capability token, sent
in the HTTP Authorization header. It initializes that owned endpoint and reads
the leading provider build version from `userAgent`, not the trailing client
version. The bounded supported set is 0.159.2 and 0.159.3; other builds fail closed.
The initialize format is source-backed at the pinned revision above, in
`request_processors/initialize_processor.rs` and
`login/src/auth/default_client.rs`. Patch compatibility still requires the final
slice's real-provider verification; the allowlist alone is not runtime evidence.

One client connection has one absolute deadline, recalculated before every
underlying read and write, including library-internal handshake/fragment reads.
Input is at most 64 KiB, response/message at most 1 MiB, frame at most 256 KiB,
and at most 256 interleaved events are inspected per call. Only notifications
may be skipped; malformed envelopes, requests and wrong response IDs terminate
the attempt. Delivery consumes the initialized client. There is no reconnect,
retry worker or resend after any outcome.

Tests use owned loopback peers and synthetic frames; they prove transport
classification, bounds and absence of a second write/connection. They do not
prove provider processing, same-live-turn attachment or production ownership.
The final consumer slice must prove those properties and remeasure the fully
linked CLI; groundwork can be removed from an unused binary by the linker.
