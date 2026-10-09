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
- Originator withdrawal and final submission share the IMMEDIATE transaction. Withdrawal
  metadata stays on the attempt, separate from delivery and the final-submission marker;
  same-reason retries retain the first timestamp. It releases waiters without a recipient
  notification, attention revision, acknowledgment or retention renewal. The open SQL query
  excludes withdrawn rows before its limit, and the shared first-final rule rejects replies.
  Results and history project this terminal state without inventing a final or approval.
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
- Delivery availability distinguishes an active unbound identity from an offline recorded
  endpoint. `talk_command::preparation` captures one waiter decision; unbound destinations
  use the existing foreground observer by default, without a host wake. Notification/waiter
  ownership precedes pull-visible queue publication, so an immediate recipient final cannot
  race notification registration. A timeout releases that waiter and leaves the queue intact;
  recorded offline endpoints retain their separate bounded background-observer policy.
- `reply_receipt` is the one receipt codec: bounded and validated before storage effects, with
  malformed receipt, stale revision, unknown identity and uncertain transport as distinct failures.
  Talk renders `<tmt-reply from="…">` with the resolved originator's display name or `unknown`:
  XML-escaped presentation, not authentication.

## Request history

- Schema 47 indexes the originator results view by submission-time keyset. Results use an
  observation snapshot without housekeeping; storage reuses the canonical attempt/response
  row decoders rather than introducing another request store.
- `tmt-adapters::request_text` owns display-control classification shared by label validation
  and preview/notice normalization. Stored request and response bodies remain exact.
- `tmt-adapters::request_history` admits and encodes the local-owner API without reply proofs
  or pane paths. The [extension API contract](../../../../contracts/extension-api.md) owns
  fields and caps; transport admission remains with the existing HTTP or process owner.

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

- `ConfigPaths::discover` owns `TMT_HOME` (an exact explicit directory), the default
  `$XDG_CONFIG_HOME/tmt` or `~/.config/tmt`, and local `tmt.json`. The one-release
  default-directory cutover checks the new directory once. Only the active managed
  release, verified by the existing native-install receipt owner, may exclusively
  rename the former default into it; other executables refuse with both paths
  and `TMT_HOME` before creating anything. An exclusive rename finding an occupied destination and a remaining
  former store refuses rather than silently splitting state. A completed concurrent
  move succeeds only with the source absent and the current directory present.
  An owned successful move normally reports both paths once on stderr; the managed
  skill-refresh protocol supplies a silent report callback to preserve its frozen
  JSON/empty-stderr contract. Resolution, admission and cutover have one owner;
  ordinary discovery and concurrent completion stay quiet. The DB, WAL and SHM
  remain together, including
  open file descriptors; `tmux-team.db` stays until a separately authorized
  stopped-writer cutover. Explicit homes and former local files are never renamed
  or read as aliases.

- `config::document` keeps unknown JSON fields and validates known settings through
  `tmt-core::settings`; `json_document` keeps number compatibility and raw object order on targeted
  edits. `init` creates the local file as `{}\n` exclusively and refuses existing paths. The three
  `defaults.*` settings are global-file-only.
- `config set --global theme.base` uses the CLI style's base registry (excluding board-only
  `auto`); the config adapter changes only `theme.base`, preserving opaque keys and token overrides.
  A malformed theme container refuses the write. Token writes remain file-only; `config rm` still
  clears local runtime overrides only.
- The global `theme` object is presentation: `ConfigFiles::theme` checks only its shape (a wrong
  one is a `ThemeProblem`, never a configuration error), `config show` reports `themeError` and
  still succeeds because Squad reads it, and `tmt` configures the process theme once only when
  stdout or stderr is a terminal and `theme.base` is set. `appearance::parse` rejects `auto`; the
  shared parser accepts it for `squad.toml`. `tmt-cli-style::theme::background` is pure parsing over
  injected read and clock functions; the executable owns terminal I/O. Only `tmt-cli-style` names
  colors, which the architecture test (`colors`) enforces for every production crate.

## Focus checklist delivery

`request::focus` is the UUID delivery policy and ordered reference/claim contract;
`RequestService` serializes policy CAS, held publication, notice admission and
checklist membership. Schema 49 adds policy, delivery metadata, held-reference and
sealed-checklist tables, each with complete change-cursor coverage. Prompt/final
owners and retention stay unchanged. `storage::requests::focus` owns bounded SQL,
including empty settled-checklist pruning in existing request housekeeping;
`api::focus` and the Focus adapter project the trusted local consumer seam.

The ordinary request wake and existing reply-frame/joined-fallback writer claims
admit Focus before granting new external input. Owner UUID and urgent bypass only
this gate. Existing explicit inbox publication remains pull-only. A provider owns
turn/launch admission; talk/check uses fresh matching live idle evidence and the
ordinary channel-first delivery owner. There is no Focus timer, detached worker,
cadence or scheduler. Definite unsent settlement releases only the sealed members;
claimed/uncertain effects never become replay leases. Canonical contracts:
[Focus delivery](../../../../contracts/request-response-v1.md#focus-delivery-windows),
[local API](../../../../contracts/extension-api.md#focus-policy-and-checklist).
