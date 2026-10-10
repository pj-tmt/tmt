# Identity, bindings and provider hooks

Per-module rules for identity, binding, session, hook and setup code. The owner map is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#identity-names-and-bindings); public shapes are
in `contracts/`.

## Names and identities

- `tmt-core::names` canonicalizes with an ECMAScript whitespace trim, NFKC and root-locale
  lowercase from pinned ICU data, never case folding or compiler-dependent casing. A
  dependency upgrade must not renormalize stored keys. Names are global in the database.
- Metadata and self-reported status are descriptive, untrusted data with no permission,
  availability or prompt authority. Expired status stays inspectable but is not current.
- `identity_metadata` owns validation and conditional expectation evaluation; storage
  rechecks the exact active UUID and checks/applies all keys in one IMMEDIATE
  transaction. CLI and API share adapter admission/projection, with no schema,
  notification or host effects. The public shapes and failure contract belong to
  [conditional metadata](../../../../contracts/extension-api.md#conditional-identity-metadata).
- `identities.auto_named` is set only by an unnamed registered-runtime launch; `name`/`this`
  renames the same UUID while it is set and consumes it, as does an ordinary `mv`.
- A bind commits identity creation before its binding transaction. If the second step is
  refused deterministically (`PaneAlreadyBound`, `NameAlreadyActive`, `PaneNotFound`,
  `TargetChanged`), core retires the temporary identity this invocation created in one
  transaction that re-checks it is still temporary, unretired and unbound. Existing and saved
  identities are never touched and uncertain outcomes keep the row. A failed retirement is
  `BindingError::CleanupFailed`, reported beside the bind error, never instead of it.
- Neither conclusive death nor unbind kills a pane; saved removal needs force.
- Notebooks live at `<global_dir>/notes/<identity-uuid>/notes.md`, created one component at a
  time with owner-only modes and no-follow exclusive creation. A same-name successor has a new
  UUID and path.

## Binding evidence

- Presence reads (`whoami`, named/current-name presence and inventory) capture
  identity/binding records, perform host observations outside SQLite's immediate
  writer transaction, then compare the full records again before reconciliation.
  Changed records are Unknown unless ownership evidence still agrees with the
  current binding; they cannot authorize detachment or retirement.
  Caller-pane reads capture candidates for the caller host and pane, then select
  the observed server before reconciliation; matching unchanged stale records
  still reconcile. Mutating marker operations retain their own coordination
  transactions.
- The pane marker (`@tmt.agent`) proves ownership by identity, binding, server and
  pane-process IDs alone; its name may lag the stored name and readers resolve by ID.
  `binding::rename_identity` renames inside the immediate binding transaction; the post-commit
  refresh rewrites the marker only while it is still this binding's. Read paths (`ls`, `talk`)
  never write it.
- `bindings.pane_incarnation` holds the `ProcessIncarnation` start token core observed for the
  pane shell at creation, never text from a host. A known recorded value that differs from a
  known observed one is `EndpointLost`; NULL or a failed observation is unknown. Read paths do
  not observe (one macOS `ps` costs about 110 ms).
- `ProcessIncarnation` (PID plus core's start token, `ps-v1` UTC seconds; on Linux boot seconds
  plus clock ticks) compares runtimes, launch owners, notification waiters and external host
  servers. Native reads (`tmt-sys::bsd_info`, `/proc/<pid>/stat`) fall back to bounded `ps` for
  the whole batch when any one fails. Vanished, zombie or changed start is ended; stopped,
  traced or inconclusive is unknown and never permits legacy unobserved delivery. For a
  wrapper-launched runtime a surviving child also needs a live matching launch owner, else the
  runtime is unknown (child death is still ended). Owner fields take part in the same
  full-observation compare-and-set.
- `tmt-sys` is the single audited `unsafe` boundary: a leaf over `libc` with no build script
  and one macOS function wrapping `proc_pidinfo`; only `tmt-adapters` may depend on it.
- `whoami --context` reads SQLite read-only in one transaction and still needs fresh driver
  evidence. It never creates, migrates, refreshes timestamps, acknowledges or cleans up, bounds
  output to 4 KiB (keeping counts and inspect commands), and carries no request IDs, bodies,
  receipts or notebook text. Ambiguous or failed evidence renders nothing; only a verified empty
  pane gets the unbound hint.
- Caller identification asks the runtime driver's `identify_caller` first. A shared Codex
  app-server's inherited pane is not evidence of the invoking conversation: a positively observed
  app-server rejects required implicit attribution before any effect, optional senders stay
  anonymous with one stderr notice, and an unavailable probe fails closed only with a thread
  marker present. A thread ID is a hint, never an identity source.
- Ordinary CLI commands may remember a direct provider conversation after flushing
  their own output. Matching remembered coordinates do no host or provider-file
  work. Changed coordinates use a supervised 500 ms observation: verified native
  pane/binding, provider ancestry and incarnation, then the existing starting-hook
  proposal and full binding/preferences fence. Failure never changes the command
  result; no timer, daemon, setup hook or provider settings write is needed.
  `TMT_CALLER_SESSION_DEBUG=1` opts into one stderr diagnostic naming the refusing
  layer, without paths, IDs, provider output or content.
- A storage-only pane-locator precheck suppresses discovery without an active
  binding. Private `caller-session-refusals.json` keeps at most 32 refusal hints
  for 10 minutes; hits do no optional host/provider work and never extend expiry.
  Keys include harness/session, binding, pane PID/incarnation and server locator;
  Claude adds `CLAUDE_PID`. Missing Codex PID uses the stored pane partition.
  Changed coordinates or expiry re-run native admission. Neither env locators
  nor cache contents grant authority; corrupt/missing cache is a miss.
- Claude's documented `CLAUDE_CODE_SESSION_ID` and `CLAUDE_PID` locate the main
  resumable session (including same-process subagents). The PID must equal the
  natively verified provider incarnation; nested providers are refused. No Claude
  transcript or session file is read. Codex's `CODEX_THREAD_ID`, `state_5.sqlite`
  index and rollout metadata are implementation evidence, not a documented
  compatibility interface. The driver reads the exact thread with a read-only,
  no-wait parameterized query after checking the captured table shape, then only
  the bounded first `session_meta` header under `CODEX_HOME/sessions`; root ID,
  CLI/exec source and absence of a parent must agree. Missing/held storage, absent
  WAL shared memory, changed env/index/header shapes or subagent metadata leave
  the session unrecorded, without repair or scanning. Fixtures and recorded
  provider versions live in `tmt-adapters::runtime/fixtures/README.md`.
  `sqlite_home` in `CODEX_HOME/config.toml` precedes `CODEX_SQLITE_HOME` and the
  default `CODEX_HOME`. Unsupported relative config paths, profile configuration
  and CLI config/profile overrides refuse rather than guess across homes.
- `focus` needs present evidence but no running agent and switches only the invoker's own
  client (never a bare "current client"); it sends no input.

## Sessions and driver state

- Session preferences (harness, session ID, resume mode, channel choice) belong to the identity;
  runtime observations belong to the binding row, which resets to unknown when replaced and
  survives an idempotent bind. Retiring an identity clears its remembered session and driver
  state in the same transaction. No session ID or running observation grants ownership.
- Resume coordinates pair session ID, harness and driver-owned mode (Claude `default`, Codex
  `shared`/`embedded`); provider hooks must record the same tokens.
- Driver state is an opaque versioned document of at most 1 KiB that core stores and never
  parses. `RuntimeLifecycle::reads_state` names readable versions and
  `RuntimeRegistry::reconcile` drops what no registered driver can read. It holds resume
  essentials only (model, opt-in usage, activity, consumption), never transcript content,
  arguments or secrets. Bounded launch settings use separate metadata. Claude and Codex
  share `runtime::driver_state` (v1 model, v2 usage, v3
  activity, v4 consumption; a document without a newer field stays byte for byte).
- The model comes only from a starting hook's `model` field; resume replays it only when
  readable and a safe single argv value.
- One identity has one current runtime: only starting provider events replace the session, and
  a launch under another driver drops the previous driver's session and state.
- A resume launch marks the exact session pending before the child starts. At exit the session
  goes stale only if it is still pending, the exit was non-zero and not 128+n, and the TMT
  SessionStart hook was installed at launch; otherwise only the mark clears. `resume` never
  falls back to a fresh start.
- Observation writes compare the complete expected observation inside the binding transaction,
  so a late update for a superseded conversation cannot win. Admission needs fresh live
  evidence; inconclusive admission keeps the previous observation, known-ended included. Clear
  and in-process resume are nonterminal; end/compact must match the exact current key.
  Provider end retains that key and launch owner in Unknown until a fresh start;
  the same live incarnation can then turn over its session. Only conclusive
  process loss/owned-child exit yields runtime Ended. Delivery probes can report
  Running after provider end (including legacy Ended) only for an exact live process
  under the verified pane and an exact live launch owner when recorded, without
  rewriting storage; shared servers outside that ancestry remain unverified.
- The channel choice (nullable) is recorded after an admitted fresh launch or explicit resume
  flag; null keeps the driver default; flagless resume does not rewrite it.
- Activity comes from TMT's own UserPromptSubmit/Stop hooks (Claude runs them synchronously;
  ordering relies on that provider contract) and, for Codex, its first-party turn ID.
  `binding::session::activity` owns the transitions; SessionStart resets activity, duplicates
  never renew timestamps, timestamps are accepted observation times, and `ended` needs a
  conclusive process observation.
- Competing executable claims resolve by descending priority, then harness ID. Explicit launches
  select argv verbatim; optional driver-owned session settings are composed afterward. Bare
  relaunch resolves the registered executable with no arguments. Exact resume instead uses
  the associated `resume.launch` metadata preset: exact executable, driver-extracted model
  and effort, and only `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`/`CLAUDE_MINI_PCT`. It never stores
  full argv. A starting admitted hook associates the preset with its exact launch owner and
  provider session; unassociated presets cannot replace a driver executable. The last
  observed model wins over launch settings; explicit resume model/effort and present env
  values (including empty env) win. Conditional metadata updates fence replacement and clear.
  `resume --show` inspects the record; `--forget-launch` clears it without forgetting the
  session. Unnamed resume uses the shared verified caller selector, never a sole-name fallback.

## Consumption and context usage

- Usage and consumption share the consented `tmt setup` plan; `--no-usage` removes the TMT
  `Stop` entry and records the choice, legacy records keep the installed state, new installs
  default on.
- Drivers read only their own provider's transcript (`runtime::transcript`): a regular `.jsonl`
  under the driver's tree, opened without following a final symlink and without blocking, with
  tail reads bounded to one MiB. Unusable usage writes nothing for that value.
- Consumption counts accepted completed-request provider token units (cached input is a subset
  of input), not cost or throughput. A baseline starts at EOF with zero counters; cursor loss,
  shrink, replacement, limit exhaustion or invalid records start a new epoch at EOF with
  `gap=true, complete=false`; history is never recounted and a partial last line waits for its
  newline. Claude counts each contiguous `message.id` group once keeping only hashed IDs, device,
  inode and offset; Codex baselines at cumulative totals and an invalid newest `token_count` is
  unavailable. Optional `cacheWriteTokens` stays a subset of unchanged input totals;
  missing fields are unknown. `modelId`/`deltaByModel` read completed-request/turn model
  evidence (Claude `message.model`, Codex `turn_context.model`), separately from the
  starting-hook resume model. Deltas group at most four models per sequence, compacted
  only inside the existing opaque cursor; size/evidence loss omits attribution before
  evicting legacy counters. A Codex `token_count` line also supplies optional `rateLimits`
  ([contract](../../../../contracts/extension-api.md#consumption-history)) from that same
  line, with its own timestamp; an invalid object is dropped whole without affecting
  the counters, and it is omitted before attribution when the opaque bound is exceeded.
  History stores additive cache-write and `byModel` evidence
  in the existing buckets, coalesces every model change, and omits unknown mixed portions.
  The Claude scan gets half the hook's remaining deadline and keeps validated counts
  with `complete=false` on expiry; the hook supervisor stays the hard time authority.
- `run_command/run.rs` ticks the foreground wait every five seconds into
  `consumption_sample_command`, one supervised two-second worker that revalidates the full
  binding, session, process incarnations, pane evidence and owned Stop hook, closes SQLite before
  reading and commits against the captured snapshot. It never infers activity.
- `storage::consumption_history` keeps five-second buckets for two hours (at most 1,440 closed
  plus one open per identity), stores no transcript content or absolute path, and commits cursor,
  counter and history in one transaction. A history read keeps one snapshot and reads each
  identity's entire retained range, even for a shorter display window: older rows still determine
  availability, the latest watermark and corruption refusal. Ordered rows are shared by all
  requested windows; disjoint output buckets consume them once per window. Public bounds:
  [extension API](../../../../contracts/extension-api.md#consumption-history).

## Hooks, setup and uninstall

- The observation hook entrypoint (`__hook`) always exits zero with no permission output and supervises one
  worker under the two-second budget (provider settings allow three; Codex channel hooks use
  `RuntimeLifecycle::hook_work_duration`). The remaining budget reaches the worker as a typed
  hidden argument and reaches `turn_state`. A failed worker kills its own group; the supervisor
  owns deadline termination. Hooks open only existing compatible storage with a short lock wait,
  never migrate, and on failure emit no context and at most one fixed stderr line. A hook
  supplies observation only: the binding must match fresh server/pane/marker evidence and
  payload session IDs never create bindings or move identities. Hook storage writer admission
  waits at most 500 ms within the supplied remaining worker deadline, reserving 100 ms for
  commit and context completion; budget exhaustion keeps the unavailable-context behavior.
- Compaction reminders use the admitted callback's normalized `LifecycleObservation::transition`,
  never a retained `last_transition` (a Codex channel startup can preserve it). Saved identities
  only receive the default-on global boolean `notes.compactionReminder` context line; invalid
  config suppresses the advisory without vetoing the hook. Existing notes paths are quoted;
  missing/unsafe/long paths fall back to the explicit notes-path command. Hooks never initialize
  or read/write notebook content. The core line survives optional context trimming within 4 KiB.
- An unknown Claude `SessionEnd` reason is rejected without mutation; its known
  logout/prompt-input-exit/other reasons end the session, not the process. Codex
  maps SessionEnd regardless of reason to the same provider-session boundary.
  Shared Codex app-server hooks need one exact provider-session mapping and then revalidate the
  binding's endpoint and session compare-and-set; a shared client exit is unknown.
- `tmt-adapters::setup` replaces or removes only exact owned hook entries, keeps user values as
  raw JSON, compares the plan again under the setup lock and keeps a byte-exact owner-only
  backup in `.tmt-setup-backups` (more than 32 warns, none is deleted automatically). Final
  settings symlinks are refused, never resolved. `<global>/setup-record.json` records driver,
  settings file and launcher; hooks found without a record are adopted only on an exact match; an
  invalid record stops setup before any provider file changes. Codex setup owns only `hooks.json`
  under `CODEX_HOME`.
- Guided `tmt setup` plans from filesystem-only detection, asks once (`SETUP_CONSENT_REQUIRED`
  without a terminal or `--yes`) and applies skills before hooks; `publish_core` re-classifies
  each target under the installer lock and keeps what is not TMT's.
- `tmt uninstall` plans read-only, asks once (`--yes` never implies `--purge`) and then removes
  hooks (the exact inverse of setup), owners' skills and bundled links, extensions before the
  CLI, the setup record and, with `--purge`, the data directory. An unreadable record stops it
  first; anything differing from what TMT wrote is kept and reported; a failed step stops and a
  rerun resumes. Product removal order derives from `Product::ALL`.

## Provider launch Digest hooks

Launch-baked hook names remain callable while their sessions can still run after
an upgrade. Hidden `__focus-hook` aliases `__digest-hook` with identical arguments
and exit behavior for pre-rename Claude and Codex launches.

`RuntimeLifecycle::prepare_launch_hooks` is an optional driver-owned launch boundary.
Claude composes one inline `--settings` object before channel enrollment for fresh
and resumed launches. Explicit inline or regular-file settings retain unrelated
raw JSON and user hooks. Unsafe settings, duplicated or edited owned hooks, and
explicit hook-disable policies refuse composition; no provider settings are written.
Composition failures leave the user's original command unchanged and emit one
diagnostic line; they never refuse the launch or retire its temporary identity.
Held items remain available to the verified-idle checklist path without Stop hooks.
Setup's exact ownership rule skips per-launch observation entries already installed
in user or explicit launch settings. A `--setting-sources` selection that omits user
settings also omits their hooks from this deduplication. `--bare` and `--safe-mode`
refuse hook composition while the original launch proceeds. Without a consented usage
hook, the per-launch Stop observer updates activity only; it does not enable
transcript collection.

Codex composes invocation-only `-c hooks=<TOML>` for fresh and exact resume launches,
including the owned channel app-server. Definitions use a stable absolute TMT command
and `--discover-launch`, with no identity, process or session coordinates; launch
lookup occurs inside the supervised worker using the existing verified caller binding.
Codex trusts exact hook definition hashes: approval remains user-owned through `/hooks`.
TMT never approves trust, enables disabled hooks or writes Codex settings. Exact setup
observers in eligible user/system `hooks.json` or explicit invocation sources remain
the only recorder; missing Stop observation is activity-only. Non-empty inline hooks
in user/system `config.toml` or `requirements.toml` use the original command fallback
because the invocation layer can shadow their event arrays. Unsupported profiles/plugins,
managed hook directories, project hook sources with uncertain eligibility, unreadable
or edited/duplicated sources, and disabled/managed-only policy use the same original
command fallback. Other invocation config remains intact; explicit invocation hook
entries are retained in the composed session table.

Names and argv written into provider launches, host options or installed shell
scripts remain accepted through at least the next stable release after a rename;
Core reviews renames and removal of the corresponding pinned legacy argv entries.
The hidden `__focus-hook` alias forwards to the same typed `__digest-hook` handler.
A verified legacy main-session Stop may show one best-effort user-visible restart
advisory through the provider's `systemMessage`, alone or merged into its existing
JSON decision. Unsupported output, current names, recursive/auxiliary and unverified
callbacks stay quiet. `ConfigPaths::hook_notice_directory()` owns private 0700
`<global_dir>/hook-notices`; one empty 0600 `<provider>-<sha256(session)[:32]>` marker
is created exclusively before output to elect its writer. Publication failure may
lose the notice for that session; markers grant no authority or delivery evidence.
A successful creation best-effort removes owned empty markers older than 30 days.
No provider settings, SQL, exit status or stderr changes accompany the advisory.

The separate `__digest-hook` accepts only an unrecursive main-agent Stop and emits
both providers' documented `decision: "block"` plus `reason` continuation. It does not use
`additionalContext` alone. Unsupported events, including StopFailure and SubagentStop,
and `stop_hook_active: true` never claim. Exact identity/binding, fresh host marker,
native provider incarnation, live foreground launcher owner, and remembered/current
provider session admit the boundary. Codex additionally requires `turn_id`; an owned
channel caller must prove its exact private generation, Ready thread, live server,
foreground and launcher owner. Arbitrary shared servers remain inadmissible.
The claim transaction fences that binding and
remembered pair; simultaneous telemetry updates do not grant or remove authority.

A bounded worker seals the canonical checklist and builds its digest. Its private
result is not delivered evidence: the supervisor revalidates admission and the active
attempt, then publishes one bounded JSON handoff under the same two-second budget.
Complete raw stdout publication has no userspace buffer left to flush and settles
`delivered`; zero bytes settles `definitely_unsent`, partial publication `uncertain`.
A lost worker response, supervisor crash or failed settlement leaves a discoverable
claim and never grants automatic replay. This is hook handoff, not model consumption
or X acknowledgment. There is no Digest timer, polling worker or settings installation.

## Foreground launch

- `run_command/run.rs` owns the bound launch and completion, `run_command/resume.rs` command
  selection and resume marks. A first operand that is a registered bare runtime executable
  selects an auto-named launch only if no active identity holds that token; a collision refuses
  with explicit alternatives. A failed spawn retires only its exact new temporary automatic
  binding.
- A verified live or stopped previous runtime blocks a second launch; an inconclusive probe
  allows a degraded launch only after fencing that attachment's Running state to Unknown.
  A directly owned child stopped before admission retains its exact attachment in Unknown
  for duplicate refusal; only fresh live evidence can admit it for automatic delivery.
- SQLite is closed before the interactive wait; completion ends only the matching binding, child
  incarnation and launch owner. Storage diagnostics never replace the child's exit code, and
  admission failure never kills a launched child. `HookObserver` runs only for committed
  observations, outside transactions.
- The foreground launcher masks only SIGTSTP during the post-spawn storage close and restores
  its exact prior mask on return or unwind. A pending terminal stop takes effect after close;
  dispositions, SIGSTOP and other signals stay unchanged. The already-spawned provider retains
  normal signal state. The close-gate regression sends real Ctrl-Z while the lock is held,
  proves the child stops first, then the launcher stops with database descriptors closed.
  Ordinary suspension scenarios wait for descriptor closure before Ctrl-Z: committed Running
  can precede SQLite's exclusive WAL-close lock release. All suspension scenarios require shell
  continuation before submitting the conflict command.
- `Storage::identity_candidates` (completion, pickers) opens read-only without migration or
  reconciliation; failure is not evidence an identity is absent. Only hidden
  `__completion-script` emits scripts; no completion command writes startup files.
