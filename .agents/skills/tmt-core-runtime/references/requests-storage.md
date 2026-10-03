# Storage, requests and configuration

Per-module rules for `tmt-adapters::storage`, `tmt-core::request`, the delivery notices and
configuration. The owner map is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#sqlite-and-durable-exchanges); public shapes are in
[request-response-v1](../../../../contracts/request-response-v1.md).

## SQLite adapter

- Raw connections stay private; core sees narrow ports. Frozen fixtures keep their provenance in
  `typescript/test/fixtures/storage-history`.
- WAL setup retries only classified Busy inside one five-second budget, verifies the returned
  journal mode is `wal` before migrating and preserves the original Busy error on exhaustion.
- OS-denied writes and SQLite read-only/WAL failures classify as a typed not-writable error (a
  bare CANTOPEN needs independent permission evidence). An existing data directory without owner
  write permission is reported, not repaired. Only the typed cause changes a core command's code to
  `STORAGE_NOT_WRITABLE` through `tmt-command-output::Failure::storage_access`; hooks, context
  snapshots and internal workers keep their own absence policies.
- A rebuild migration copies the stored definition with only the intended change, replays stored
  indexes and triggers, refuses a table that differs from what earlier migrations make in a fresh
  database, runs with foreign keys off inside one immediate transaction that rolls back whole, and
  restores foreign keys on every exit. Migrations keep recorded names and historical retention.
- **Change cursor:** every core-owned table has three AFTER triggers
  `<table>_advances_change_cursor_on_{insert,update,delete}` that advance `change_cursor` inside
  the writing transaction. An update counts only when a compared column differs
  (`bindings.last_verified_at` is not compared). A migration that adds a table or column, or
  rebuilds a table, must recreate its triggers: `change_cursor_tests` fails until every table is
  covered or deliberately excluded and every column compared.
- Host columns on `bindings`, `request_attempts`, `request_responses` and `host_servers` accept any
  host name (1 to 32 of `[a-z0-9-]`, starting with a letter); NULL is tmux; inbox routes carry no
  host. `host_servers` holds TMT's UUIDv4 per server incarnation of a host without its own
  server-level store.
- Metadata is one value per key per identity UUID, at most 64 entries, revalidated against the
  active UUID inside the write transaction; retirement hides it and a same-name successor inherits
  nothing. Status is one atomic record per active UUID.
- Hook subscriptions queue retirement notifications in the retiring transaction; a delivered
  subscription cannot be resurrected; no transaction spans remote work.

## Request service

- Preparation, delivery transitions, exact final submission, waiter release, attention revisions
  and retention housekeeping belong to `RequestService`. Prompt/final content, attempt metadata,
  retention and acknowledgment have independent lifecycles.
- Cadence is reserved with a durable attempt before sending and refunded only after definite
  failure. Final bodies are immutable: identical retries are idempotent, conflicting finals fail.
  Retention is frozen per attempt; lazy housekeeping respects active waiters, keeps the acceptance
  deadline and never resurrects an expired submission.
- `ackall` acknowledges one snapshot, so a later final is unread again; acknowledgment means
  handled, not successful.
- `first_final_refusal` is the one rule for whether a request still accepts a first final;
  `open_requests` applies it, so "waiting on you" is an open-request question, not an attention
  one. `answer_target` selects one open request by recipient and originator, never guessing, and
  derives the route proof in process.
- A UUID-prefix result lookup (at least eight hex characters, optional `req_`) resolves and reads
  under one transaction; `retained_request_ids` returns at most five ordered ambiguity candidates.
- `enqueue` prepares the attempt, stores the prompt and publishes recipient attention in one
  transaction; the recipient revision is allocated atomically with `queued`, so a merely prepared
  attempt cannot wake a listener. An inactive recipient commits a failed, non-waiting attempt with no
  attention. Full delivery settles recipient attention, not the originator's response attention.
  A preamble reservation refunds only for proven non-delivery.
- Listeners use an indexed watermark and one bounded snapshot; `exchange_command` owns the
  monotonic deadline and trailing debounce; polls do no tmux inventory, cleanup or held
  transaction, and there is no daemon or event bus.
- `reply_receipt` is the one receipt codec: bounded and validated before storage effects, with
  malformed receipt, stale revision, unknown identity and uncertain transport as distinct failures.
  Talk renders `<tmt-reply from="…">` with the resolved originator's display name or `unknown`:
  XML-escaped presentation, not authentication.

## Reply notices

- A request opts in only through its notification policy; historical, anonymous and queue-only
  requests have none. First-final acceptance reserves a callback only without a live blocking
  waiter, with process evidence observed outside the transaction and matched against stored waiter
  ownership inside it.
- `storage::requests::reply_batch` persists fixed windows and members independent of final bodies
  and X attention. Queued members store no body: only immediate and send-time rendering read it
  through `notice_context`, which never acknowledges or changes retention. `delivery::notices`
  sanitizes display fields, quotes the body as data under the channel and paste limits and
  re-derives queued legacy members from request keys.
- `request::notification::batch` owns the quiet/deadline policy; `reply_notice` composes
  enrollment evidence, enqueue, binding-fenced delivery and one-shot settlement;
  `reply_notice_command` schedules finite detached workers that claim by process-incarnation CAS
  before waiting, seal batch membership and record per-frame attempt evidence before transport.
- A unique SQLite sending claim serializes worker transport per binding. Only exact process-death
  evidence releases a stranded claim: attempted frames retire uncertain, untouched members stay
  queued. Workers hold no transaction while sleeping or probing; a failed send never replays.
  Clean workers remove their own logs; failed ones keep them.
- The competing-waiter grace derives from the registry maximum of `Driver::maximum_send_duration`,
  is an observer allowance only (expiry keeps notices queued) and uses `std::time::Duration` as pure
  data; clock reads stay forbidden in core by the architecture guard.
- An approval-blocked registered frame settles definitely unsent without stopping later frames;
  joined host notices keep the host's single approval result with no input fallback.
  `HostDriver::input_activity` reports elapsed real key evidence or Unknown; no screen or prompt
  buffer is read as typing.
- `process::detached` owns startup acknowledgment, failure cleanup and the worker's removal of its
  own stderr log (only while the path still names the same device and inode); there is no sweeper.
  The request observer composes durable reads, the timeout claim and delivery outside locks and
  never re-sends.

## Configuration and theme

- `config::document` keeps unknown JSON fields and validates known settings through
  `tmt-core::settings`; `json_document` keeps number compatibility and raw object order on targeted
  edits. `init` creates the local file as `{}\n` exclusively and refuses existing paths. The three
  `defaults.*` settings are global-file-only.
- The global `theme` object is presentation: `ConfigFiles::theme` checks only its shape (a wrong
  one is a `ThemeProblem`, never a configuration error), `config show` reports `themeError` and
  still succeeds because Squad reads it, and `tmt` configures the process theme once only when
  stdout or stderr is a terminal and `theme.base` is set. `appearance::parse` rejects `auto`; the
  shared parser accepts it for `squad.toml`. `tmt-cli-style::theme::background` is pure parsing over
  injected read and clock functions; the executable owns terminal I/O. Only `tmt-cli-style` names
  colors, which the architecture test (`colors`) enforces for every production crate.
