# Squad data and state

Maintained reference for the modules that read or write member data. The user-facing
row shape, state patterns and reminder keys are documented in the embedded lead
skill (`extensions/tmt-ops/skills/tmt-ops/SKILL.md`).

## Membership and leadership

- A squad is the core room `squad-<name>`. Member fields are identity metadata
  `squad.<name>.<field>`, so one identity can belong to several squads and removal
  clears exactly one namespace.
- The per-member `note` field is retired. `membership::parse_change` refuses a
  nonempty `note=` with `SQUAD_NOTE_RETIRED` and a notebook/task/pending hint during
  all-pairs validation, before core calls or writes; an empty `note=` still clears
  stored metadata. `Squad::roster_with` drops legacy note values from member fields
  without mutating storage, row JSON omits `note`, and `rows::OWN_FIELDS` keeps the
  name reserved so providers and bound columns cannot reuse it. Member context lives
  in each member's own saved-identity notebook (`tmt notes path --identity <member>`).
- Leadership is the reserved metadata key `squad.<name>.lead.marker` (`true` or
  `false`), outside the user field/column grammar. The roster parses it into
  `Member::lead_marker` and omits it from public `fields`. `Member::is_lead` reads
  the marker and falls back to `role == "lead"` only for an unconverted member.
- Conversion is per member and happens on writes, never reads. Before applying `set`
  pairs, the membership owner records a marker only when the role write would change
  legacy-derived leadership. `add` writes `false` before joining only when existing
  metadata would otherwise make the new member lead.
- `squad lead` preflights the metadata capacity for every required marker before any
  write, records `true` before joining the new lead, then sets previous leads to
  `false`, keeping role text and membership. `lead --none` preflights and clears the
  markers without selecting or joining anyone; former leads stay members until
  `squad rm`. A marker that would exceed the identity metadata limit returns the
  existing core error before role pairs or the join.
- Membership commands are sequences of idempotent core commands, not a transaction:
  each reports what it applied and a re-run converges. A metadata write that lands
  after preflight can still split a sequence. `add` reports `added: false` for an
  existing member (including a repeated name in one call) and preserves its state and
  task; only a missing state gets the configured initial value. `lead --none` reports
  a null lead and the former leads in `replaced`.

## The user (`me`)

- Core owns the user: the global `user.name` setting (default: the OS login name) and
  the `user.identity` API operation, which resolves, creates and marks the identity
  (contracts/extension-api.md). Ops keeps no copy. `me::current` asks Core; `None` means
  Core could not resolve a name (`USER_NAME_*`), and callers say how to set one.
  `tmt ops squad me [<name>]` shows the user or runs `tmt config set user.name <name>
--global`; `init --me` sets it first, so a bad name changes nothing. Nothing else
  asks for the user.
- `ops.toml` once held `me` and `me_id`. `me::migrate` runs after every config load
  (command entry and board start): while either key exists it marks the identity
  `me_id` names (else `me`) as a person and carries its name into `user.name`, unless
  Core already has a name, and drops the keys. A failed move keeps them for the next run. The readers
  `Config::me`/`me_id` exist only for that migration.
- Core marks the user `user = true`. Ops writes that key only in `me::migrate`, once,
  for the migrated identity when it has no `user` entry (a bound pane would otherwise
  make `user.identity` pass it over for `<name>-user`); an existing value is kept. Ops
  metadata is otherwise `squad.*` only.
- The board's sender is the user, else the saved identity bound to the calling pane
  (`tmt whoami`). A pane that is not bound is no sender; a `whoami` failure (a host
  that cannot name the pane) is kept as `App.caller_failure` and the sender prompt says
  `Could not identify this pane: <core's message>`, never the unbound-pane wording.
- With hooks enabled, `tmt-ops __tmt-hooks 1 observe` applies an `identity.renamed`
  observation at once (`me::renamed`): when the renamed identity carries `user = true`
  and the name in effect no longer resolves, `user.name` takes the new name. Without
  hooks `user.name` keeps the old name until the user sets the new one.
- Sender (`me::resolve_sender`): explicit `--identity`, else the identity core
  attributes the call to (`tmt whoami`), else the user; with none,
  `SQUAD_SENDER_UNKNOWN` names both ways to set one. `whoami`'s `PANE_NOT_FOUND` and an
  unbound pane mean "no caller"; any other core error, such as
  `CALLER_IDENTITY_AMBIGUOUS`, fails the command instead of falling back.
- "You" for `waitingOnYou`, `ls` and the board (`me::you`) is the user, else
  the saved identity bound to the calling pane (the board reads it once per worker);
  with neither, `ls` and the board footer show one hint line. The board also sends as
  "you", because a popup's pane is not its operator.

## Reading a squad (`ls`)

- `ls` (alias `status`) joins one `rooms.roster` snapshot with `ls --room` presence.
  Presence is read first so core reconciliation retires dead temporary identities
  before the roster; a member joining between reads has unknown presence until the
  next load.
- `status::document` writes each column's bound value into the row field of the
  column's name (with its number for sorting) before sections, filters and sorts run,
  so the board and `ls` show one value and a binding adds no core call. A column's
  `from`/`format` (`source::ColumnSource`) reads only the public projection: the
  `ls --room` row (`cwd`, `target`, normalized `resume`) and roster metadata, which
  `rooms.roster` returns unprefixed only when a column reads `meta.<key>`.
- Sections: `[[squad.<name>.section]]` (title, filter, sort) replaces the single list;
  rows matching none follow in one untitled section. `filter` owns a bounded boolean
  language over a row's text fields (1024 bytes, depth 32) and every section is
  validated before output. Without `--squad`, `ls` always returns
  `{squads: [...], you}` in name order so script shape never depends on squad count;
  state-changing commands still require `--squad` when several squads exist.
- Colors: `status::document` resolves cell color from a column's numeric `color`
  thresholds (`rows::Threshold`, strictly increasing, validated theme tokens) over the
  bound number or the field read as a number, else a field provider's token (kept by
  `provider::apply` only if it names a theme token). `config::States` resolves state
  color and rank: an exact entry (including layout presets) wins entirely, else the
  first ordered `state_patterns` glob, else no color and the default rank. Explicit
  sorts precede preset sorts at the same number; unspecified pattern sort ranks after
  ranked states. The pattern matcher is a bitset NFA over Unicode scalars with
  fixed-size transitions and no backtracking or dependency (`*`, `?`, literals; optional
  case-insensitive match by lowercase scalar); limits are 64 patterns and 256 bytes per
  match with indexed config errors. Only `status::document` publishes the resolved
  state token as `colors.state`, ignoring thresholds and provider colors, and the board
  consumes tokens rather than keeping a second state-color map. `ls` text is uncolored
  and shares state sorting (including section sort keys) with the board.
- `attention::Attention` is the one definition of a squad's tab state: members waiting
  on the user (`pending` or `waitingOnYou`) and members `blocked`, each counted once.
  `ls` publishes it as `squad.attention`.
- `requests` derives per-row `annotation` (the sender's newest open tagged request)
  from `requests.list` for the squad room (at most `PAGES` 4 of `PAGE` 50) and
  `waitingOnYou` from `tmt inbox --json` (at most 200); either cut-off sets
  `olderRequestsNotShown`. The same room window yields the replies list (finals to the
  user's requests, newest first). `requests::apply` projects the user's open notes
  annotations to the current lead as optional `squad.noteAnnotations`.

## Aggregate tabs

- `tabs` owns squad keys, built-in keys (`@leads`, `@all`) and configured member-view
  keys (`@tab:<name>`), which cannot collide with squad names. `[tabs] order`, `pin`
  and `hide` name user views `tab:<name>`; unplaced views follow the defaults in
  definition order. Config reading validates every `tabs.<name>` filter, sort, section
  and binding, including hidden views; built-in names stay reserved.
- `tab_view` owns cross-squad acquisition and aggregate documents for both the board
  worker and `ls --tab <name>`. One roster read per squad feeds source documents and
  the public member projection, keeping numeric sort values and state ranks beside the
  JSON. Providers contribute their existing cache; aggregate reads never run them.
  User selection applies before the shared `status::sections` pipeline, so section
  matches may repeat a row. User views use `Rows::leads` with a MEMBER caption and no
  per-tab row overrides. Board and `ls --tab` receive identical projected rows,
  attention and grid metadata, and `status::text` renders that document.
- Unreadable squads are omitted with located `failures`; a failed inbox read keeps
  available roster fields. Both set `partial` (board indicator, text warnings) until
  the next successful read. Member views join one global `ls` read for presence; rows
  carry their squad so talk goes to that room and a jump is the ordinary `tmt focus`.
  The public all document keeps one row per squad; its board-only home composition
  also includes attention members.

## Field providers

- `provider` (`[squad.<name>.fields.<field>]`) runs the user's own program per member
  through `runner` with the run-binding argument rule (`Template::fill_argument`: one
  argument per template, no shell, a value that would start an argument with `-`
  refused), `PARALLEL` 4 at a time, bounded in time and output (`OUTPUT_LIMIT` 4 KiB,
  `VALUE_LIMIT` 200, `MAX_PROVIDERS` 8).
- `provider::Cache` stores each value with the argv that produced it in
  `$XDG_CACHE_HOME/tmt-ops/fields/<squad>.json` (atomic replacement via `cache`: a
  0600 file in a 0700 directory), so a changed input never shows an old value.
  `preset = "github-pr"` is a fixed `gh pr view {pr_link}` argv whose JSON
  `provider::github_pr` renders as `#<n> <state>[ · <review>]`; anything else from `gh`
  is a failed run. `provider::apply` writes current values into member fields before
  the document is built, `?` plus the row's `failed` list after a failed run.
- Readers never run providers: `ls` reads the cache (`--refresh-fields` runs due work
  first), and the board hands each load's members to one fetcher thread (see
  [refresh-and-meter.md](refresh-and-meter.md)).

## Staleness and reminders

- `staleness` owns observed raw task/state age and exact lead-notebook content age,
  separate from providers and column bindings. `observe` is the one read sequence for
  a squad's status, shared by `ls` and the board's squad tabs: it takes the nonblocking
  cache lock before the roster read, reads the lead's notes through public `notes.read`
  only when the observation can publish (or the notes pane is shown, which reuses that
  read), records, and hands the bounded room history to the request overlay. Providers
  never run there.
- `Snapshot::apply` adds the same `staleness` object to every occurrence of a member
  UUID and `squad.notesStaleness`; text labels derive from them. The board draws a
  stale row in the `dim` token with its label at the row's right edge (reserving the room
  only when no column would be hidden) and adds the notes label to the notes pane title
  in `waiting`; the label text carries the meaning without color. The leads tab reads
  rosters without an observer and shows no marks; the home model observes squads for
  blocked-member ages. Content age is unrelated to `App::loading`.
- The observation cache under `$XDG_CACHE_HOME/tmt-ops/staleness` is bounded to
  `FILE_LIMIT` 512 KiB and `MEMBERS_LIMIT` 128 members per room, namespaced by the
  absolute config/data-root path and room UUID with member/lead UUID ownership.
  SHA-256 fingerprints (`sha2`) keep no notebook body. A nonblocking Unix advisory lock
  (`nix::fcntl::Flock`) is held from before the read through atomic publication;
  competing readers report unknown and never regress the cache.
- Time rules: the first observation starts the clock and is never backdated;
  unreadable notes, an unavailable cache and clock rollback mean unknown.
  Fingerprint/ownership/evidence changes publish immediately; otherwise unchanged
  observations replace the cache only when its persisted `observedAtMs` rollback
  watermark is at least `WATERMARK_INTERVAL_MS` (60 s) old. Ages are computed on every
  observation without writing. A rollback crossing the watermark reports unknown and
  restarts grace; a reversal entirely inside an unwritten interval can shorten reported
  ages by at most 60 s. Cache loss or corruption restarts grace. Config edits do not
  reset content age; after disabling and re-enabling, surviving fingerprint matches keep
  their first-observed time. These are observed content times, not core modification
  times or a history feed.
- `activityAfterUpdate` records relevant observed PR link/state changes, member finals or
  authoritative idle transitions after a row update. Only successful unexpired
  `github-pr` cache values, the public room history and runtime-verified
  `session.activity` count as evidence; self-reported activity and offline presence never
  establish idle.
- `Config::reminders` parses `[squad.<name>.reminders]`: enabled for the `team` layout,
  disabled for the others, 30 minutes default, whole `s`/`m`/`h` values from 1 minute
  through 24 hours.
- `reminder` consumes the generic consented `context_v1` callback at SessionStart and
  prompt submission, never Stop. A cache-only gate exits before core calls or room locks
  for cold/off/fresh/claimed/non-lead cases. A warm candidate uses public config and room
  commands to validate its root and room UUID, then `observe::Mode::Reminder` reads only
  the roster, notes and bounded room history; the roster must independently establish the
  callback identity as the sole lead. It runs no providers, presence probes or inbox
  overlays. `staleness` publishes per-generation claims under the same lock before
  returning a summary, and `reminder` states all claims as names/counts in one sanitized
  line.
- Context calls share one monotonic deadline (`BUDGET`, 300 ms). Core's hook runner
  isolates the extension's process group and context-only nested calls inherit it; an
  invocation-scoped timer bounds input, files, publication and output, signals only its
  live process-owned group, and is canceled and joined on completion. The path requires
  the extension to own its process group. A host timeout can cut it off earlier and owns
  reaping. Ordinary core calls keep their independent groups and allowances; no resident
  worker or core Squad concept exists. The lead skill owns the user-visible observed-age,
  claim-loss and cache-loss limits.

## Ops state database

`state` (library module `tmt_ops::state`) owns `<dataRoot>/ops/ops.db`, a private SQLite file
(directory 0700, file 0600) with the table `documents(kind, key, body)`: the `cron` document
(key empty) and one `checklist` document per canonical room UUID, plus the checklist mutation
log (`checklist_events`, `checklist_event_floor`, below). Admission, the five-second
write queue, crash-atomic commits and the refusal of a database written by a newer Ops belong
to the shared `tmt-extension-state` leaf (Store); `state` adds the slots, the import and the
archive, and hands callers a `Refusal` (`Busy`, `Newer`, `Failed`) to present under their own
codes (`SQUAD_CRON_STORE_BUSY`/`_NEWER`/`_IO`, `CHECKLIST_STORAGE_ERROR`). Callers keep their
codecs; the database stores bytes. Constructing or reading an absent store creates nothing.

- **Import.** The first access after an upgrade finds a slot's row absent and the previous
  release's file present (`cron/jobs.json`, `checklist/<room-uuid>/items.json`). The file's
  bytes go through the caller's decoder like any stored bytes and are inserted inside the
  write transaction, so a corrupt or foreign file refuses and changes nothing.
- **Archive.** After that commit the file is renamed to `<name>.migrated-<YYYY-MM-DD>`
  (`-<n>` when taken) by hard-linking to the first free name and then dropping the old name:
  nothing is ever overwritten or deleted. The step runs under the database write lock, because
  two unserialized archivers can both unlink the name (macOS reports both as successful) and
  leave two archives of one generation. A crash between commit and rename is safe: the next
  access sees the row and the file, trusts the row and finishes the rename. An older Ops
  that keeps writing the old file is archived the same way and never read again. The lock
  files and staging names of the old stores are no longer created.
- **Newer database.** `Store` refuses history this binary does not know; Ops names it with an
  upgrade hint (`tmt extension upgrade ops` or `tmt upgrade`) and changes nothing.
- **Other owners.** `ops.toml` edits go through the leaf's `Document` (one lock around the
  compare-and-replace; the same `SQUAD_CONFIG_CHANGED` refusal when the file moved on). The
  cron clock stays a lease in `cron/clock.json` (expiry and takeover are its contract); the
  leaf has no lease primitive.

## Checklist storage and service

The native binary's `checklist` module provides typed list/show/create/edit/assign/
unassign/complete/reopen/archive/restore/delete/reorder operations. `checklist_command`
exposes them through `tmt ops squad checklist` (`tmt ops sq` dispatch alias); primary `ls` has
hidden accepted alias `list`, both returning semantic JSON action `list`. The adapter
parses only explicit UUIDs and revisions, invokes the unchanged service once and
projects its typed results/errors. The board consumes the same typed service directly; `Service::preview` reuses read admission to expose its captured actor, current inventory and eligible roster. Board filters and authored drafts remain presentation state.

- `model` owns the frozen item lifecycle: positive item revisions, a separate inventory
  revision, full authored order including archived items, exact expectations and no-op
  validation. Create and Delete change inventory; other item writes do not. Delete drops
  content and retains only room/checklist/item UUIDs and deletion revision, without purge
  or UUID reuse. Completion and archive are independent.
- `store` keeps each room's version 2 JSON document as the `checklist` document keyed by the
  canonical room UUID in the [Ops state database](#ops-state-database). The document has
  `version`, `roomId`, `checklistId`, `inventoryRevision`, ordered live `items` and minimal
  `deleted` tombstones. Each item also stores `addedBy`/`addedAtMs` and
  `lastChangedBy`/`lastChangedAtMs` (an identity UUID and Unix ms; each pair is both null or
  both set). `Service::apply` builds the stamp from the admitted actor and `status::now_ms`
  and hands it to `model::apply`; a request has no such field. Create sets both pairs; a
  change that moves the item's revision sets `lastChanged*`; reorder and delete stamp no
  item, and a no-op stamps nothing. Version 1 (before provenance) still decodes, with every
  stamp null (unknown, never guessed). Reads create and rewrite nothing, so the first
  successful write stores version 2; the previous release's `items.json` import arrives as
  version 1 and migrates the same way. An Ops binary that only knows version 1 refuses a
  version 2 document (unsupported, no repair). It persists no copied membership, actor authority or operation ledger.
  Read/list/show create nothing and distinguish absent, empty and filtered-empty results
  from errors. Unsupported/corrupt data and unsafe files fail closed without repair. A
  candidate is written inside the database transaction and `before_publish` (the service's
  repeated room/actor/assignment admission) runs before it commits: a refusal discards it.
- `checklist::events` owns the mutation log. `model::apply` returns an `AppliedChange` (action, item,
  resulting revision, and the stamp's actor and time) exactly when the change moved something;
  `Store::update` appends it inside the same transaction that replaces the document
  (`Database::update`'s `before_commit` receives the transaction), after the admission recheck, so
  a refused or discarded change leaves neither. `checklist_events.id` is AUTOINCREMENT: a cursor
  never repeats, even after pruning. The same writer prunes the room to the shorter of 30 days or
  the newest 1000 events and records the newest id lost in `checklist_event_floor.pruned_through`;
  the floor's `legacy` flag marks a list that already had a document when the log began (the
  migration inserts it for every existing document, and the first append inserts it for a
  document that arrived later, such as a legacy-file import). `Service::events` reads under the
  list's read admission, creates nothing, and reports `gap` as `pruned` when `pruned_through`
  is past the cursor, else `legacy` when asked from 0. Titles and bodies are never copied. An
  Ops that does not know the migration refuses the database (`Newer`), like any appended one.
- A conversation list (`--member`) is a second key of the same `checklist` kind,
  `<room-uuid>/<member-uuid>` (both canonical UUIDs, so it cannot equal a room key), with an
  optional `memberId` in its document that must equal the key's second half like `roomId` does.
  Room documents carry no `memberId`. Its
  one checklist UUID is derived from the pair (`Id::conversation_checklist`), so a caller never
  supplies it. `resolve_member` runs once in the adapter (UUID through `references.resolve`,
  which also finds a retired identity; a name through `identity show`, active only); the service
  and store see only the UUID. Every admission re-resolves the member: unknown is
  `CHECKLIST_MEMBER_UNAVAILABLE`, retired is readable and refuses writes. Admission to the
  room and the actor's authority are those of the room list. No Core schema or contract changes.
- The board's conversation Checklist (`board/conversation_checklist.rs`) projects a list, one per
  `(room, member)`: a lane job runs `Service::for_conversation(..).preview()` for the member on
  screen (never per frame; the first read, then one per five seconds, a failed one waits
  thirty) and keeps the list for the chip and the overlay. A `reference` marks a link
  independently of completion: open items also count as todo and completed items as done;
  archived items are neither counted nor listed. A Common-pool member has no room and no
  list. Only the running board reads
  (`State::reading`). The overlay (`N`, a click on the chip) lists links, then todos; a row
  shows `who · age` from the item's `added` stamp (`you`, the room roster's name, or the
  start of an identity the roster no longer lists; no stamp, no author and no age) and a done
  todo `done <age>` from `last_changed`. Every write (`Add`, `Edit`, `Toggle`, `Delete`,
  `Reorder`) is one lane job through `Service::apply` carrying the item and inventory
  revisions the row was read at; one write is in flight per list, and a read begun before a
  write never replaces what the write answered. A refusal shows Core's words in one line
  (`CHECKLIST_MEMBER_UNAVAILABLE` for a retired member); a conflict also re-reads the list. An
  unsent form belongs to the conversation, not the overlay: Esc closes the overlay and keeps
  a changed form, a sent one is dropped on success and re-aimed at the item's new revision
  on a conflict.
- A write queues behind another writer for at most five seconds; a spent budget is
  `CHECKLIST_STORAGE_ERROR` saying nothing was saved with the one retry, and a commit that
  timed out is `CHECKLIST_OUTCOME_UNKNOWN`. The implementation limits are 64 MiB per document
  and 100000 retained item identities; every individual admitted title/body/reference bound
  fits within these limits. The previous release's `checklist/<room-uuid>/items.json` is
  imported on first use (see the database section) and never deleted.
- The service captures the public caller through `me::caller`, using the Core user
  (`me::current`) only when no caller is bound. Ambiguous callers refuse. A user Core
  cannot resolve is no actor. The user and current room leads
  are managers; other current members are collaborators. Non-null Create assignment also
  requires manager permission. Assignment admits an exact active same-room UUID; departure
  preserves its last admitted label and projects it as unavailable. Unassign needs no live
  former assignee. Exact room UUIDs survive rename; successor rooms never inherit data.
  Only the active user can explicitly inspect unavailable-room data read-only.
- Locked writes reload and validate storage, check exact expectations, and recheck current
  actor/room/operation/assignment admission immediately before rename. These public Core
  reads and file replacement are not a cross-database transaction. Temp write/file sync/
  rename failures preserve committed bytes; after rename, failed directory sync or
  acknowledgement returns `CHECKLIST_OUTCOME_UNKNOWN`. Readback never proves which earlier
  uncertain operation committed; a new operation needs a freshly reviewed revision.
  Checklist errors and authorized conflict/deletion projections are typed locally;
  `checklist_command` emits one JSON stdout document with success exit 0 and every error,
  grammar errors included, exit 1. Only typed `Error.current` becomes optional
  `error.current`; refusal never invents a projection. Human failures use main's
  existing stderr error/hint path and emit no success text. UUID/revision/text/order/
  confirmation validity remains operation input validation rather than clap usage.
  Unrelated Squad error JSON is unchanged. No dispatch, hooks, attention, request,
  metadata or configuration mutation, reference fetching or implicit opening occurs.

## Cron

The user-facing command reference is the lead skill's "Manage recurring jobs". Maintainers
own these layers:

- `cron::schedule` owns positive elapsed intervals, fixed local times and five-field cron
  parsing with next-slot math. Named time zones use Jiff's system/zoneinfo database with no
  bundled database. Fixed local times skip DST gaps and take the first occurrence in a fold;
  elapsed intervals keep their stored anchor and duration. Day-of-month and day-of-week use
  the standard alternative rule unless either field starts with `*`.
- `cron::store` keeps the versioned jobs document as the `cron` document of the
  [Ops state database](#ops-state-database): per-squad counters that survive removal, exact
  message bytes, room and owner references, schedules, revisions and pause attribution. The
  caller supplies the absolute Ops root under `storage.root` and admits core UUID references;
  the store resolves no identities, decides no permissions, dispatches nothing and tracks no
  runs. Reads of an absent store create nothing. A mutation queues for the database's write
  lock for at most five seconds, then reads the current document under it, applies its change
  and commits, so concurrent adds, removals, pauses and edits all land; a reader queues behind
  a writer the same way. A spent budget returns `SQUAD_CRON_STORE_BUSY` saying another Ops
  process holds the database, with the one retry (`Run the same command again.`) and with
  nothing saved. A failed change commits nothing. Invalid existing state fails explicitly
  rather than resetting counters or overwriting it. `pause_owner_then` is membership's
  barrier: it commits the departing owner's pause, runs the leave or move with no lock held
  (it runs a core command), then pauses once more so a job its owner added meanwhile is
  paused too; the second pause is best effort because the departure is done.
- `cron_command` composes the management grammar and output; one `cron_service` is shared
  with clock and board callers. `cron_service` owns `list_jobs`/`show_job`/apply,
  explicit user-or-verified `CronActor` admission and room/owner/revision revalidation.
  Existing mutations carry a `JobKey` (squad name, room UUID, c-id) and an expected revision;
  `add` carries its selected room UUID. Admission and stored comparisons run inside the
  stable jobs lock. The user or the squad's current lead may write; an identified
  ordinary member never falls back to the user; reads need no permission. Manual and
  scheduled send callers obtain admitted immutable jobs through the same locked path, and
  anonymous scheduled admission requires an enabled job and active owner membership. No
  dispatch runs under the jobs lock; core roster/identity state can still change after that
  snapshot and core owns final dispatch admission. `Core::api_write` reuses the bounded
  process owner with an explicit identity or anonymous envelope. Scheduled sends, `cron send`
  and the change announcements go through `Core::dispatch_automated`, which declares
  `origin: "automation"` and repeats the same operation once without it when an older Core
  refuses the field with `API_INPUT_INVALID` (nothing is stored on a refusal).
- Owner hooks: `identityHooks` registration (consumer `squad-cron`) precedes job publication;
  a failed publication can leave a harmless unused reference. List, show and apply process
  one pending retirement page of at most 16 hooks, and clock ticks call the same drain.
  A still-matching room/job/owner reference becomes paused with no owner and a new revision
  before hook acknowledgment; obsolete hooks are acknowledged without editing a reassigned or
  removed job. Projections exclude jobs of retired or replaced rooms while keeping their
  records and counters.
- Notices: add/edit/pause/resume/remove notify the owner; reassign notifies old and new
  owners (the message goes to the new one); the actor's own notice is suppressed; retirement
  notifies the current lead anonymously. These post-commit announcements use deterministic
  room/job/revision/action/recipient operation UUIDs, report failures as warnings, and have no
  rollback, outbox or recovery journal, so an interruption can lose a notice. Reassignment
  keeps a pause; resume requires a current owner.
- `cron::clock` owns the bounded, read-only Running/NoClock/Unknown lease projection
  and atomic `clock.json` publication under the Ops `cron` directory. A stable,
  nonblocking `clock.lock` serializes acquisition, renewal and ownership-checked
  release. Contention is `SQUAD_CRON_CLOCK_BUSY`; acquisition and renewal retry on the
  next tick, while release, which runs once at shutdown, waits up to 500 ms for a
  brief holder such as a status read. Pane, PID, start and expiry are evidence, not
  process probes; an expired lease can be taken over, and a clock refused because another
  holds the lease names that holder's PID and pane. `cron::tick` owns slot selection and deterministic
  operation UUIDs from room UUID, non-reused c-id and UTC slot milliseconds. A
  running clock admits every slot since its own previous tick, capped at five
  minutes; startup, takeover, restart and clock rollback baseline at now. A
  standalone tick admits the last 60 seconds. No absent-clock history is replayed.

- `cron_clock` adapts `run/tick/clock/send` to public core commands/API and the shared
  admission service. Clock text shows the relative start age; JSON retains exact
  millisecond evidence. Each scheduled send revalidates revision, room and owner,
  renews its lease, then dispatches the exact message anonymously outside the jobs
  lock. An uncertain create recovers the same operation with `dispatch.show`; it
  never invents a replacement or re-wakes a replay. Manual send retains an explicit
  CronActor and viewed revision, with a new operation ID per action; paused jobs
  with a current owner may be sent without changing their schedule. Acceptance is
  reported separately from delivery and results are not tracked. `board/mod.rs`
  starts and stops an independent cancellable, joined clock worker; refresh-off,
  repaint, tabs and usage generations do not own its lifetime. Foreground run
  handles interrupt/termination/hangup through that same shutdown owner, cancelling
  owned core children before releasing the lease. Its own local process cancellation
  is a clean stop; completed real failures remain errors. A second foreground run or tick
  refuses an unexpired holder; a board waits and can take over after expiry.

- The lease-owning clock starts `tmt digest tick` through its resolved `TMT_EXECUTABLE`
  once on acquisition/current minute and once per later observed minute, without
  catch-up or rollback replay. Each child runs independently of cron passes and
  painting through the cancellable runner, with a 90-second process ceiling and
  64 KiB per-stream capture. Digest owns its single-instance lock, in-minute
  deadlines and delivery; Ops adds no scheduling policy or overlap suppression.
  Missing/nonzero/failed ticks are skipped without a same-minute retry: one plain
  stderr line in foreground run, only an in-memory skip count/last reason in the
  board. Shutdown cancels and joins tick children before lease release. This
  preserves the existing board-owned or explicit foreground clock lifetime;
  closing the board starts no replacement clock or service.
  The same tick runs the [idle alert](board-cron.md#idle-alert-scan) scan.
- The same minute admission independently invokes public `workspace.capture`
  through Core after discovering the operation in `capabilities`. Successful
  discovery is cached for the worker session, including an answer without the
  operation; errors and timeouts retry at the next admitted minute, never within
  the same minute. Discovery and capture share five seconds; an unfinished
  workspace child suppresses another capture, without delaying Digest or cron.
  Discovery and capture failures stay silent, including foreground run: no notice,
  warning, stdout or stderr. Cancellation joins and reaps the child
  before lease release. No new timer, setting, tmux access or paint/input I/O is
  introduced; Core owns [capture and coalescing](../../../../contracts/workspace-v1.md#capture-triggers-and-public-backstop).
