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
  host name (1 to 32 of `[a-z0-9-]`, starting with a letter); NULL is tmux; inbox and remote routes
  carry no host. `host_servers` holds TMT's UUIDv4 per server incarnation of a host without its own
  server-level store.
- `request_attempts.route_kind` is `pane`, `inbox` or `remote`, and CHECKs tie each kind to its own
  columns. A remote route stores the Remote reference, an optional target UUID, the target name and
  the sender name; it has no local recipient or pane, so recipient-keyed readers never list it and
  `find_active_request` never matches it. `request_responses` has no remote route.
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
- `tmt_adapters::passthrough` owns the sender rule, the recipient check (`can`, from
  `Storage::passthrough_facts`; only a bound, not-ended agent that is not waiting on approval)
  and the refusal codes and texts that `talk --passthrough` and `dispatch.create` share;
  `delivery::deliver_request` types the text and settles it as a request delivery.
- A request's delivery mode (`request` or `passthrough`, in `request_delivery_policies`) decides
  what the recipient is handed. A passthrough is a person's typed text: Core refuses it unless the
  originator carries the `user` mark, queues it without publishing recipient attention, keeps it
  out of `open_requests` and `answer_refusal`, and reports its final as `not_required`. The
  mode joins the row's change-cursor comparison. `settle_request_delivery(Sent)` of a passthrough
  runs `answer_oldest_open` in the same transaction: the retained original text becomes the first
  final (`store_final`, shared with `submit_response`, no originator hint) of the oldest open
  request from that agent to the sender; any other settlement stores nothing. The same send creates a
  `request_passthrough` row (one per passthrough, cascading with the request): `pending`, or `bound` at
  once with `folded = 1` (schema 69) when `RequestRecords::running_turn` shows a verified `Working` turn
  of a driver whose descriptor declares `folds_queued_text` (Claude only, until another is measured)
  whose `since_ms` is not after the send's start (`RunningTurn::holds_text_typed_at`), since no prompt will
  announce text a running turn takes in. The hook's
  runtime-observation transaction binds a pending row at an admitted turn start (`TurnBoundary::Started`: the
  prompt matches the typed text, first in first out) and settles it at the admitted turn end
  (`Ended`: the captured final, else `no_answer`; an answer raises the sender's attention revision through
  `request::raise_attention`, the helper `store_final` shares, #4915); `PassthroughCapture::state_at` reads a row past its
  bind window (unbound) or `BOUND_WINDOW_MS` (bound) as `no_answer`, so no sweeper exists. The same
  transaction writes what a read would show: `release_unseen_ends` settles a bound row whose session was
  replaced, whose window passed, or that a later `Started` of its session overtook (before binding, so a
  start never overtakes the capture it binds).
- Incoming request Digest admission excludes a person on either side using the reserved active `user=true`
  mark, not pane absence. A request visible in a person's inbox reads `delivered` /
  `waiting_for_person` until a final makes it `answered`; stale holds and activity grant
  nothing. Legacy person references remain in history but checklist reads, claims and
  reoffers expose no items for delivery.
- A message's state (`request::delivery::delivery_view`, pure) is derived at read time inside the
  history, exchange, incoming and inbox read transactions from rows only: `storage/requests/delivery.rs::facts` reads the attempt's
  route and status, the wake state (an identified recipient's request stays `queued`; its paste is the
  wake), the `focus_items`/`focus_checklists` hold, `focus_policies.max_hold_ms` plus the oldest held
  arrival (the latest send time), the driver activity of the exact bound session and process (a
  stored event, no host call), and what came back (`ReturnedAnswer`: a submitted final, or the
  passthrough capture as `state_at` reads it, so a capture past its window already reads
  `no_answer`). A returned answer outranks every other fact: `answered` is terminal, and an ended capture
  without an answer is `unconfirmed`, never `working` (#4910). `working` of an inbox request ends with
  the turn that began after its hand-over: `request_turns` (schema 70, `requests::turns`) records that turn's
  start when the hook transaction admits a main-session `Started` (for each handed-over request with no
  final, passthrough or row yet, prepared within `BOUND_WINDOW_MS`) and its end at `Ended`;
  a closed or idle-without-end turn reads `unconfirmed`/`turn_ended_without_reply`, and a request with no
  row keeps the activity-only reading bounded to six hours (#4911). The Digest tick writes `maxHoldMs` with every `digest.policy.set` and
  rewrites a policy whose recorded value differs; older policies read null until then.
- A delivered checklist keeps `confirmed_at_ms` null until later evidence arrives (pre-068 rows are
  confirmed by the migration). One typed at an idle opportunity is confirmed by the hook's
  runtime-observation transaction when an admitted turn start's prompt names one of its request
  ids (`requests::confirm_digest_by_prompt`). One handed to a Stop hook marks a continuation open
  (`digest_continuation_open`), cleared by the continuation's own Stop, any later prompt or the end of its provider run
  (`end_digest_continuation`, called from `bindings::session::set_state` and binding removal); `digest::idle_entry` refuses an idle opportunity meanwhile. `digest_claimable_items` and
  `claim_digest_checklist` add the unconfirmed items only at a turn-boundary opportunity, skipping
  an incoming request that no longer admits a first final (`reofferable`), and the claim returns
  them to the held pool before sealing, so an idle claim never types them twice.
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

## Exchange flow

`request::flow` defines content-free UTC bucket values and bounds; `RequestService`
records accepted preparation/final, explicit successful retrieval and effective
acknowledgment events in their existing transactions. `storage::requests::flow`
owns aggregate SQL and bounded pruning inside request housekeeping. Observers and
notices use ordinary response reads, never the explicit retrieval operation.
`tmt x flow` reads all local pairs in an observation transaction, without caller
selection, body loading or housekeeping. The [request contract](../../../../contracts/request-response-v1.md#content-free-exchange-flow)
owns event cardinality, direction, window rounding, `--bucket` summing
(`flow::rebucket`, applied after the read) and independent 30-day retention.

## Room and dispatch composition

- `tmt-core::room::RoomRepository` is the shared CLI/HTTP roster boundary.
  `ActiveRoomReader` supports active selection, including inside a caller-owned
  storage read transaction. Canonical UUIDs are authoritative; exact labels must
  resolve uniquely. Historical UUID lookup retains retired rooms but never
  authorizes new work. Membership mutations and revision-checked retirement use
  the same `storage::room` IMMEDIATE writer, never unlocked read-modify-write.
  `tmt-adapters::room` owns the CLI/HTTP wire projection; `ls --room` filters
  existing presence rather than acquiring another presence owner.
- `RequestRecords` rechecks effective recipient membership inside request
  preparation, for pane and inbox routes, before cadence or attention writes.
  CLI preflight alone is no atomic fence. Direct `talk --room` names one recipient,
  not a fan-out. Presence, self-reported status and roster membership remain
  distinct; room commands require no extension installation or tmux probe when an
  identity is explicitly selected.
- Core `dispatch` composes values and normalized audience; its adapter owns strict
  JSON and intent digests. `storage::dispatch` borrows `TransactionRequests` so
  `RequestService::enqueue` retains all request SQL/state and one transaction
  commits the operation with every accepted/failed recipient attempt. There is
  no nested transaction or second request store. Full-roster dispatch checks
  revision and exact effective UUID audience after immutable replay lookup;
  direct mode checks only the targeted member. The tagged mode and room UUID are
  immutable intent, never permission to expand a retry's audience.
- Known CLI sender provenance uses the existing identity resolver and participates
  in intent hashing and request attention; HTTP refuses caller-selected senders.
  Unknown-sender digests and historical ledger names stay unchanged. `RequestKind`
  and the request service own announcements' no-response and settlement policy.
  Core `operation` supplies operation UUIDs for dispatch retries and board mutations.
- Each canonical attempt retains its original optional room UUID through details,
  incoming results and attention. Room-scoped listeners filter the watermark and
  both incoming queries using participant/room indexes before pagination. Later
  roster changes cannot hide delivered work; room retirement never cascades into
  request retention, receipts or spatial/resource content. Transports do not infer
  historical membership. Public composition shapes belong to the
  [extension API](../../../../contracts/extension-api.md#dispatch-readiness-and-input-safety).

## Request history

- `request::history` owns owner-visible keyset history over canonical attempts,
  not chat storage; its service composes existing retention and attention final
  states. Storage reuses attempt/response decoders. Lists acquire bounded UTF-8
  preview bytes, preserving embedded NUL without loading complete message bodies.
  HTTP inspection retains the bearer/Origin admission of its existing transport;
  operation lookup and dispatch replay share one immutable ledger decoder, and a
  lookup cannot resubmit. Browser transports own scope checks, not request caches
  or another completion policy.
- `RequestAttempt.origin` (person, automation, or unknown for a row from before schema 57)
  is set once by the producer: CLI talk/send, `room send` and MCP record a person, only
  `dispatch.create` carries an extension's declared automation. It joins the dispatch intent
  digest only when automation, no operation updates it afterwards, and the history projection
  never derives it from names or text.
- Schema 47 indexes the originator results view by submission-time keyset. Results use an
  observation snapshot without housekeeping; storage reuses the canonical attempt/response
  row decoders rather than introducing another request store. An optional recipient narrows
  the same index walk with a per-entry test (no recipient column in the index), so a page
  may visit every retained final of the originator; add a recipient-leading index before
  that cost matters.
- Schema 64 indexes answered passthrough turns by settle time. The originator answers view cuts
  finals (schema-47 index) and turns (that index, with a per-entry originator test) to the page
  and merges the two short lists under one `(answeredAtMs, requestId)` keyset; a request is a
  final or a turn, never both, so the keyset names one entry.
- `tmt-adapters::request_text` owns display-control classification shared by label validation
  and preview/notice normalization. Stored request and response bodies remain exact.
- `tmt-adapters::request_history` admits and encodes the local-owner API without reply proofs
  or pane paths. The [extension API contract](../../../../contracts/extension-api.md) owns
  fields and caps; transport admission remains with the existing HTTP or process owner.

## Reply notices

- A request opts in only through its notification policy; historical, anonymous and API-dispatched
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

Test builds activate adapters' default-off `test-isolation` feature through
consumer dev-dependencies. Ambient discovery requires `TMT_TEST_ROOT` and private
HOME/XDG/provider/temp paths plus cwd beneath that admitted directory, including
existing symlink targets; local configuration discovery stops at its boundary.
Missing admission fails plainly, and an indirect unadmitted discovery panics in
adapter unit tests rather than becoming a swallowed error. Use injected paths in
unit fixtures. A plain package-scoped release build leaves the feature off.
`ConfigPaths::resolve` with explicit inputs is not an isolation guard for arbitrary
paths; callers own those inputs. Other ambient filesystem APIs remain outside
this first discovery slice. CLI fixtures suppress automatic browser opens.

- `ConfigPaths::discover` owns `TMT_HOME` (an exact explicit directory), the default
  `$XDG_CONFIG_HOME/tmt` or `~/.config/tmt`, and local `tmt.json`. The one-release
  default-directory cutover checks the new directory and, when present, its database.
  An absent database permits one no-follow check of the former sibling database; an
  existing former entry refuses with both paths: move the current directory aside
  first, then move the former directory as a whole.
  An existing current database never probes the former store; explicit homes bypass
  these checks. Only the active managed
  release, verified by the existing native-install receipt owner, may exclusively
  rename the former default into it; other executables refuse with both paths
  and `TMT_HOME` before creating anything. An exclusive rename finding an occupied destination and a remaining
  former store refuses rather than silently splitting state. A completed concurrent
  move succeeds only with the source absent and the current directory present.
  An owned successful move normally reports both paths once on stderr; the managed
  skill-refresh protocol supplies a silent report callback to preserve its frozen
  JSON/empty-stderr contract. Resolution, admission and cutover have one owner;
  ordinary discovery and concurrent completion stay quiet. The DB, WAL and SHM
  remain together, including open file descriptors. Explicit homes and former
  local files are never renamed or read as aliases.
- **Database basename (#2292).** `ConfigPaths::database` is the one owner of the name: `tmt.db`
  when it exists, else `tmux-team.db` when only that exists, else `tmt.db` for a new store, so
  no reader creates an empty database beside an existing one. It is resolved after the directory
  move, and the #2757 guard counts either name. `storage::rename_database` renames only inside
  `tmt upgrade` (hidden `__native-rename-database`, run by the activated executable on every
  completed unpinned upgrade). SQLite proves nothing holds the database: an exclusive lock, with
  no busy wait, fails while any other connection is open, idle WAL ones included. Under that lock
  the database is checkpointed and switched to one file (the stale `-wal` and `-shm` a finished
  process leaves behind are removed), gets the new name through a hard link, and the old name is replaced atomically by a relative link to
  `tmt.db`, so no name is ever missing and nothing is copied. A held database, a foreign
  `-journal`, a second database or a foreign link keeps the old name with one plain reason. An interruption
  leaves both names on one file; the next run finishes it only under the same proof. Released
  binaries and the rollback release open the old name through the link (default open flags, no
  `NOFOLLOW`) and SQLite places the journals under `tmt.db`. Removing the link belongs to the
  later alias decision.

- `config::document` uses shared `Document` for set, theme-base and clear: current-byte
  read, known-setting validation through `tmt-core::settings` and mutation share one
  directory lock. `ConfigPaths.global_dir` owns lock state, never a project sibling
  lock. `json_document` preserves unknown fields, number compatibility and raw object
  order; publication preserves permissions. Shared failures keep their consequence
  and retry instruction in the existing envelope. `init` creates the local file as
  `{}\n` exclusively and refuses existing paths. The three `defaults.*` settings are
  global-file-only.
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

## Digest checklist delivery

`request::digest` is the UUID delivery policy and ordered reference/claim contract;
`RequestService` serializes policy CAS, held publication, notice admission and
checklist membership. Schema 49 adds policy, delivery metadata, held-reference and
sealed-checklist tables, each with complete change-cursor coverage. Prompt/final
owners and retention stay unchanged. Persisted `focus_*` table/index names remain
internal storage spellings. `storage::requests::digest` owns bounded SQL,
including empty settled-checklist pruning in existing request housekeeping.
Schema 51 adds idle due-now watermarks, immutable arrival usage observations and
successful-delivery counters that survive checklist cleanup. The adapter snapshots
stored usage through its remembered driver's reader; no provider refresh or external
I/O occurs in the holding transaction. `api::digest` and the Digest adapter project
the trusted local consumer seam; the owning local API contract defines stats,
legacy-counter provenance and the difference between idle and turn-boundary eligibility.

Schema 55 adds `recent_responses`, local provider-final pane output with a ten-entry
identity ring and frozen retention. `responses::FinalText` owns byte truncation;
`storage::responses` appends within the existing binding/preferences observation CAS
only when the activity owner admitted a new Stop. Claude shares the bounded usage tail;
Codex supplies its hook payload. Retirement removes the ring independently of exchange
retention. `api::responses` reads one snapshot without housekeeping; the
[local API contract](../../../../contracts/extension-api.md#recent-agent-responses)
owns consent, expiry and wire shapes. This output never completes an exchange.

Schema 54 adds `consumption_tier5m`, the 30-day five-minute consumption rollup
(see [identity bindings](identity-bindings.md)), with the usual change-cursor triggers.
Schema 56 adds its nullable `credits` and `service_tier` columns (the newest credits
observed in the bucket and the tier of its newest token-moving read); no row is rewritten.

Schema 53 records claim opportunity/sealed count and first settlement time/order in
the existing checklist owner. Settlement order uses the durable change cursor in
the same transaction, never timestamps. The content-free history read shares one
observation snapshot, bounds inline inputs and preserves canonical expiry/pruning;
legacy facts remain unknown. The local API contract owns pagination and coverage.

The ordinary request wake and existing reply-frame/joined-fallback writer claims
admit Digest before granting new external input. Owner UUID and urgent bypass only
this gate. A request already stored as queue-only remains pull-only. A provider owns
turn/launch admission; talk/check and the UUID-pinned `digest.checklist.flush` API
use fresh matching live idle evidence and the ordinary channel-first delivery owner.
There is no Core Digest timer, detached worker,
cadence or scheduler. Definite unsent settlement releases only the sealed members;
claimed/uncertain effects never become replay leases. Canonical contracts:
[Digest delivery](../../../../contracts/request-response-v1.md#digest-delivery-windows),
[local API](../../../../contracts/extension-api.md#digest-policy-and-checklist).
