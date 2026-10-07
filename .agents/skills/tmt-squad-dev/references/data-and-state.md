# Squad data and state

Maintained reference for the modules that read or write member data. The user-facing
row shape, state patterns and reminder keys are documented in the embedded lead
skill (`extensions/tmt-squad/skills/tmt-squad/SKILL.md`).

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

## The recorded user (`me`)

- `me` and `me_id` are written together. `init` only creates the room (`--me` is
  checked before any effect); `tmt squad me [<name>|--clear]` shows, records or
  removes it. Nothing else asks for `me`.
- The UUID decides. While `me_id` names an active identity, that identity is the user
  and `me::resolve` rewrites `me` to its current name. Only when `me_id` is missing
  or inactive does the name decide and its UUID get recorded. An edited `me` naming a
  different identity is reported with a warning, never followed. A failed write never
  fails the command; the board's refresh (`me::current`) neither writes nor prints.
- With hooks enabled, `tmt-squad __tmt-hooks 1 observe` applies an `identity.renamed`
  observation for `me_id` at once; the hooks are optional and the next command that
  reads `me` makes the same repair.
- Sender (`me::resolve_sender`): explicit `--identity`, else the identity core
  attributes the call to (`tmt whoami`), else the recorded user; with none,
  `SQUAD_SENDER_UNKNOWN` names both ways to set one. `whoami`'s `PANE_NOT_FOUND` and an
  unbound pane mean "no caller"; any other core error, such as
  `CALLER_IDENTITY_AMBIGUOUS`, fails the command instead of falling back.
- "You" for `waitingOnYou`, `ls` and the board (`me::you`) is the recorded user, else
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
  `$XDG_CACHE_HOME/tmt-squad/fields/<squad>.json` (atomic replacement via `cache`: a
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
- The observation cache under `$XDG_CACHE_HOME/tmt-squad/staleness` is bounded to
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

## Checklist storage and service

The native binary's `checklist` module provides typed list/show/create/edit/assign/
unassign/complete/reopen/archive/restore/delete/reorder operations. `checklist_command`
exposes them through `tmt squad checklist` (`tmt sq` dispatch alias); primary `ls` has
hidden accepted alias `list`, both returning semantic JSON action `list`. The adapter
parses only explicit UUIDs and revisions, invokes the unchanged service once and
projects its typed results/errors. No board action is exposed.

- `model` owns the frozen item lifecycle: positive item revisions, a separate inventory
  revision, full authored order including archived items, exact expectations and no-op
  validation. Create and Delete change inventory; other item writes do not. Delete drops
  content and retains only room/checklist/item UUIDs and deletion revision, without purge
  or UUID reuse. Completion and archive are independent.
- `store` owns version 1 JSON at `<dataRoot>/squad/checklist/<canonical-room-UUID>/items.json`.
  `items.lock` is stable; `items.tmp` belongs to the locked publisher. The document has
  `version`, `roomId`, `checklistId`, `inventoryRevision`, ordered live `items` and minimal
  `deleted` tombstones. It persists no copied membership, actor authority or operation ledger.
  Read/list/show create nothing and distinguish absent, empty and filtered-empty results
  from errors. Unsupported/corrupt data and unsafe files fail closed without repair.
- A nonblocking room lock bounds contention as `CHECKLIST_STORAGE_ERROR`. New owned
  directories/files use 0700/0600; existing permissions survive replacement. Files are
  opened no-follow/nonblocking and must be owned regular files with one link; extension
  directories must be owned real directories. The trusted public root may be canonicalized.
  The implementation limits are 64 MiB per document and 100000 retained item identities;
  every individual admitted title/body/reference bound fits within these limits.
- The service captures the public caller through `me::caller`, using read-only recorded
  user context only when no caller is bound. Ambiguous callers refuse. A retired recorded
  UUID cannot adopt a same-name successor. Active recorded users and current room leads
  are managers; other current members are collaborators. Non-null Create assignment also
  requires manager permission. Assignment admits an exact active same-room UUID; departure
  preserves its last admitted label and projects it as unavailable. Unassign needs no live
  former assignee. Exact room UUIDs survive rename; successor rooms never inherit data.
  Only the recorded active user can explicitly inspect unavailable-room data read-only.
- Locked writes reload and validate storage, check exact expectations, and recheck current
  actor/room/operation/assignment admission immediately before rename. These public Core
  reads and file replacement are not a cross-database transaction. Temp write/file sync/
  rename failures preserve committed bytes; after rename, failed directory sync or
  acknowledgement returns `CHECKLIST_OUTCOME_UNKNOWN`. Readback never proves which earlier
  uncertain operation committed; a new operation needs a freshly reviewed revision.
  Checklist errors and authorized conflict/deletion projections are typed locally;
  `checklist_command` emits one JSON stdout document with success exit 0, operation
  error exit 1 and grammar error exit 2. Only typed `Error.current` becomes optional
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
- `cron::store` owns the versioned `<dataRoot>/squad/cron/jobs.json` document: per-squad
  counters that survive removal, exact message bytes, room and owner references, schedules,
  revisions and pause attribution. The caller supplies the absolute `storage.root` and admits
  core UUID references; the store resolves no identities, decides no permissions, dispatches
  nothing and tracks no runs. Reads of an absent store create nothing. A stable nonblocking
  `jobs.lock` serializes reads and mutations; validation and file sync precede atomic rename,
  then a directory sync. A failed publish keeps the previous document; a directory-sync error
  after rename reports an uncertain commit for rereading. New directories/files use
  0700/0600. Invalid existing state fails explicitly rather than resetting counters or
  overwriting it.
- `cron_command` composes the management grammar and output; one `cron_service` is shared
  with clock and board callers. `cron_service` owns `list_jobs`/`show_job`/apply,
  explicit recorded-or-verified `CronActor` admission and room/owner/revision revalidation.
  Existing mutations carry a `JobKey` (squad name, room UUID, c-id) and an expected revision;
  `add` carries its selected room UUID. Admission and stored comparisons run inside the
  stable jobs lock. The recorded user or the squad's current lead may write; an identified
  ordinary member never falls back to the user; reads need no permission. Manual and
  scheduled send callers obtain admitted immutable jobs through the same locked path, and
  anonymous scheduled admission requires an enabled job and active owner membership. No
  dispatch runs under the jobs lock; core roster/identity state can still change after that
  snapshot and core owns final dispatch admission. `Core::api_write` reuses the bounded
  process owner with an explicit identity or anonymous envelope.
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
  and atomic `clock.json` publication under the same extension directory. A stable,
  nonblocking `clock.lock` serializes acquisition, renewal and ownership-checked
  release. Pane, PID, start and expiry are evidence, not process probes; an expired
  lease can be taken over. `cron::tick` owns slot selection and deterministic
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
  owned core children before releasing the lease. A second foreground run or tick
  refuses an unexpired holder; a board waits and can take over after expiry.
