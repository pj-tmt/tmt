# Codex native channel contract

Status: implementation in progress for #719, child of #329. This document
specifies the provider adapter boundary; it does not claim completed integration
or live foreground continuity verification.

## Delivery receipt

One delivery creates one immutable `thread/queue/add` request for the enrolled
thread. A correlated response with the exact caller ID and input echo records
queue acceptance, not processing or completion. Only a durable `tmt reply`
completes the TMT request. Caller IDs are not provider deduplication keys.

A correlated error alone cannot prove absence of queue side effects. The adapter
recognizes only these exact, thread-specific pre-enqueue rejection signatures:

- Code -32600: `session THREAD is archived. Run \`codex unarchive THREAD\` to unarchive it first.`
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

## Foreground directory

The launch directory and any `-C`/`--cd` are resolved once. The adapter emits an
absolute `-C` for foreground attachment, independent of whether the launcher
spawns from the original or resolved directory. The owned app-server spawn and
thread creation must consume the same resolved directory. No initial prompt,
fork or implicit shared socket is allowed in this bounded attachment contract.

## Ownership

Enrollment belongs to an exact launch incarnation. A different proven current
incarnation may use baseline delivery without removing the older record.
An absent owner alone does not establish a replacement incarnation. Unknown,
ambiguous or unverifiable ownership remains terminal. Withdrawal compares the
expected launch incarnation and generation under serialization; it never removes
a different replacement record merely because that record's owner is gone.
