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
- The pane marker (`@tmux-team.agent`) proves ownership by identity, binding, server and
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
  arguments or secrets. Claude and Codex share `runtime::driver_state` (v1 model, v2 usage, v3
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
  keep every argv byte; bare relaunch resolves the registered executable with no arguments.

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
  unavailable. The Claude scan gets half the hook's remaining deadline and keeps validated counts
  with `complete=false` on expiry; the hook supervisor stays the hard time authority.
- `run_command/run.rs` ticks the foreground wait every five seconds into
  `consumption_sample_command`, one supervised two-second worker that revalidates the full
  binding, session, process incarnations, pane evidence and owned Stop hook, closes SQLite before
  reading and commits against the captured snapshot. It never infers activity.
- `storage::consumption_history` keeps five-second buckets for two hours (at most 1,440 closed
  plus one open per identity), stores no transcript content or absolute path, and commits cursor,
  counter and history in one transaction. Public bounds:
  [extension API](../../../../contracts/extension-api.md#consumption-history).

## Hooks, setup and uninstall

- The provider hook entrypoint always exits zero with no permission output and supervises one
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

## Foreground launch

- `run_command/run.rs` owns the bound launch and completion, `run_command/resume.rs` command
  selection and resume marks. A first operand that is a registered bare runtime executable
  selects an auto-named launch only if no active identity holds that token; a collision refuses
  with explicit alternatives. A failed spawn retires only its exact new temporary automatic
  binding.
- A verified live or stopped previous runtime blocks a second launch; an inconclusive probe
  allows a degraded launch only after fencing that attachment's Running state to Unknown.
- SQLite is closed before the interactive wait; completion ends only the matching binding, child
  incarnation and launch owner. Storage diagnostics never replace the child's exit code, and
  admission failure never kills a launched child. `HookObserver` runs only for committed
  observations, outside transactions.
- `Storage::identity_candidates` (completion, pickers) opens read-only without migration or
  reconciliation; failure is not evidence an identity is absent. Only hidden
  `__completion-script` emits scripts; no completion command writes startup files.
