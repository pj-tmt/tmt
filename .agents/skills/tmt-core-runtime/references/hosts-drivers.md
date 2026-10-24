# Hosts, process effects, agent drivers and channels

Per-module rules for the host port, tmux and process adapters, agent drivers, provider
channels and the driver protocol. The owner map is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#tmux-and-process-effects); wire formats are in
[driver-protocol-v1](../../../../contracts/driver-protocol-v1.md),
[claude-channel-v1](../../../../contracts/claude-channel-v1.md) and
[codex-channel-v1](../../../../contracts/codex-channel-v1.md).

## Process and tmux effects

- `Host::for_server` selects the recorded host and socket. Selected tmux snapshots
  are read-only: no marker initialization or ambient fallback; an explicitly
  empty pane scope reads only server evidence. Default caller
  snapshots and explicit binding discovery retain their existing adoption rules.

- A running-command handle separates launch from wait but keeps the original deadline and
  cleanup; an abandoned handle stops and reaps its child. There is no second runner.
- `process::interactive` runs direct-terminal children: terminal interrupts reach the child,
  wrapper TERM/HUP are forwarded to the owned child only, and a notification failure degrades
  supervision instead of killing a live agent. Abandonment terminates, then kills and reaps
  that child, never the shared process group.
- tmux deadline, I/O, spawn, output-limit and signal failures during target resolution stay
  `RECONCILIATION_FAILED` (exit 1), not `PANE_NOT_FOUND`; a completed lookup with no valid
  pane ID is "no target". Socket denial is `TMUX_PERMISSION_DENIED`, confirmed from the
  socket path's metadata, effective-user write access or one nonblocking, zero-byte connect
  after a connect failure. Only kernel EACCES/EPERM proves denial, never localized stderr.
  The adapter preserves every child locale variable. The single `Tmux::run` owner adds
  one global `-u` client flag, including bootstrap, evidence, transport and option
  migration calls, so a C-locale client preserves literal UTF-8 output. This changes
  neither server options nor shell selection; command-local option unsets remain
  separate from the client flag.
- `tmt_core::driver::pane_input_text` turns ASCII `!` into fullwidth `！` for all text typed
  into a pane on every host; hosts and drivers add nothing. `check` is bounded diagnostics,
  never a response channel. `response_input` polls against the deadline before each read and
  never mutates stdin flags.

## Host port

- Extensions never reach a host: the guard rejects extension code, tests included, that names
  the host port, the tmux module or core's `binding`, `endpoint` or `host` model.
- `host::driver::{status, send, focus, swap_into}` hold the one binding policy over each `HostDriver`. A
  host without input is `Unsupported` before evidence is read. A send first offers the message
  to the agent the host recognizes in the pane (`prompt`); only `Unsupported` falls back to raw
  `input`, any other answer is final. tmux recognizes no agents. `DeliveryError` is the
  host-neutral failure: stage, whether text may have reached the pane, and the host's cause.
- `tmt_core::host::SwapDirection` is pure command data. Directional focus reuses
  focus target/binding admission; `HostDriver::swap_into`
  defaults to Unsupported with no new external-driver wire operation. The tmux
  adapter owns one caller-window geometry read and the detached swap. It never
  changes caller selection, wraps neighbours or kills panes; zoom refuses.
  Public JSON and selection rules belong to the [host contract](../../../../contracts/driver-protocol-v1.md#capture-input-and-focus).
- A handle has a primary host; a session probes, marks and clears each stored binding on that
  binding's own host. A `Host` states why it was chosen (`for_caller`, `for_server`,
  `for_target`, `for_verified_caller`) and takes endpoints, never loose socket or pane strings.
  The `_with` constructors take an injected runner and approve no driver (tmux only).
- `HostKind` is pure data (token, `is_pane_id`, `is_target`). Evidence from another host is
  `Unknown`; presence groups by host and socket (`ServerSelector`). A stored host is `tmux` or
  `External(name)`, readable whether or not its driver is installed; a NULL fence host is
  tmux. External syntax comes from approved drivers registered once at start
  (`host::external::register_approved`) in core's write-once registry. Names an approved
  driver reads as targets (Herdr's `wN:pM`) are refused only as new names.
- Target syntax is case-sensitive; the
  [contract](../../../../contracts/driver-protocol-v1.md#capabilities) owns its alphabet.
  Explicit target classification (`names::is_pane_target`, `Host::for_target`) reads
  `names::target_text`: trimmed and composed, case kept. A new name is refused when its text or
  its lowercased key reads as a target (`names::names_pane_target`), since an identity is found
  by its key; existing-name validation still reads the key.
- Herdr's stored token, pane IDs, markers and `host_servers` rows predate its move to a driver
  (#1082): they parse as `External(herdr)`, so earlier bindings read as unknown, never lost,
  until the driver is approved. Without an approved driver (a build that ships none, or one
  removed with `tmt driver rm herdr`), a Herdr pane command keeps its result and prints
  `tmt driver install herdr` on stderr once a day per pane (`hint_cadence`). An approval covers
  the operations the driver declared then: a shipped driver that declares a new one, such as
  `name-agent`, reads as changed, and the next command that works with hosts approves it again
  itself (below) unless the user removed it.
- Agent names flow one way, from the identity's name to the host's agent name (Herdr's, never
  its pane label), through `Host::name_agent` and the optional `name-agent` operation; a driver
  that does not declare it is `AgentNaming::NotLinked`. Callers: post-commit in
  `binding_command` after an explicit bind or a changed `tmt mv` (JSON `agentName` member,
  one human warning on stderr), and `name: null` with the identity's name as `expected`
  inside the external `clear` (`BindingEndpoint::clear(binding, identity)`, reached only through
  core's `clear_observed`), before the marker, with half the budget left. `AgentNameRefusal`
  separates a definite `Refused` (JSON status `refused`) and no agent yet (`NoAgent`,
  `NotReady`) from `Uncertain`, where the host may have set it; the last three are JSON
  `warnings` entries, never a status. After `run_bound` admits the child of `tmt run` or
  `tmt resume`, `run_command::agent_name` names it on one thread outside the wait tick: at most
  five tries, no stored state, a stop when the exact launch no longer holds the binding, and
  `Host::for_server` calls (never `SupervisedProbeRunner`, whose abort would kill the group
  that holds the agent); `tmt run` waits at most 3 s for it after the child ends. Read commands
  and provider hooks never set a name; `tmt ls` shows the snapshot's `agentName` and its
  advice. The
  [contract](../../../../contracts/driver-protocol-v1.md#name-agent) owns the driver's guards.
- Cosmetics: tmux badge and marker refresh (names sanitized before markup); `pane_badge` is
  bounded post-commit presentation, never routing evidence. A badge refresh prefixes the
  inherited border format on that pane only, unless a user-local override or existing badge
  reference already supplies it. `@tmt.border` stores the exact installed format;
  cleanup compares it on the server before unsetting only that unchanged pane override.
  Creation uses set-only-if-unset, and publishes ownership after successful format writing;
  failed ownership publication conservatively leaves the format unowned. No shared option or
  configuration file is changed. Only explicit binding/launch callers request the off-border
  enable-command hint; delivery, hooks and automatic refreshes remain silent. The copied
  inherited format stays fixed until unbind and rebind. External hosts refresh only the marker's name:
  after a bind or rename, when `tmt run` binds, admits or ends its runtime, and when delivery admits a
  replacement runtime. Provider hooks skip it.

## External host drivers

- **Registry** (`<global>/drivers.json`): approval refuses a declaration a built-in host or
  another approved driver would read as its own. `inspect` checks ownership, digest, one
  `capabilities` probe and conflicts and writes nothing; `commit` re-checks conflicts and
  writes. `state` is `ok`, `changed` or `missing`. A first-party approval follows the shipped
  companion: an upgraded driver is adopted without asking only if it declares nothing beyond
  the approved protocol, syntax, operations and `callerEnv`; otherwise it is unavailable until
  approved again (explicitly, or automatically as below). Every read-modify-write holds
  `drivers.lock` and renames a staged file.
- A record is written only with explicit consent (`tmt driver install`), never by product
  install or upgrade; without a terminal or with `--json` it refuses with
  `DRIVER_CONSENT_REQUIRED`. Removing a driver leaves bindings stored and unavailable.
- The one exception is the first-party driver the running release ships, which carries no
  trust beyond the `tmt` already running. `registry::approve_shipped` writes the record
  `tmt driver install herdr --yes` writes, once, in `main` before `register_approved`, for
  commands that work with hosts (`skill_reminder::works_with_hosts`: not machine channels,
  workers, driver management or installation plumbing). It acts when there is no approval and
  the environment shows a Herdr pane (`skill_reminder::herdr_pane_environment`, the one place
  core names those variables), or when the release replaced an approved driver with one that
  declares more than was approved. Both checks repeat under `drivers.lock`, so concurrent
  processes approve once. One stderr line says so and names `tmt driver rm herdr`, in human
  and `--json` mode alike; `--json` stdout stays the command's own document. Removing a
  first-party driver adds its name to `declined` in `drivers.json` (written only when not
  empty), which blocks both cases until an explicit `tmt driver install` clears it. A path
  approval, a third-party driver, a release without a verified driver and any failure approve
  nothing and leave the install hint.
- `DriverProcess` runs one operation through the bounded owner under its deadline and output
  bound with `TMT_DRIVER_CALL=1`; `executable_trust` (shared with extension hooks) checks
  ownership and fingerprint before every call and the digest once per process; a `tmt`
  started with `TMT_DRIVER_CALL` refuses everything but help and `--version`.
- Evidence is core-led: server identity is core's own `ps` start token for the pid the
  driver's `server` names (the driver's `startTime` is advisory), gone or replaced is Dead,
  same process plus the driver's snapshot is Live, else Unknown. The optional `probe` is not
  called. A removed or changed driver answers `Unavailable`, never proof of loss.
- A driver's `caller` pane counts only when that pane's shell is an ancestor of the caller
  (`process::ancestry`); for an ordinary command the nearest verified pane wins, otherwise
  the host is tmux. With tmux variables too (nested), an ordinary command orders both panes
  on one pinned ancestry, as hooks do below (`Host::nearest_nested_external`), and re-checks
  the entries up to an external pane before it wins. Unlike a hook it keeps its fallback:
  driver doubt, an ancestry that can't be pinned (a reused pid), both hosts naming one pane
  process, a changed entry, or the shared 1 s budget running out keep tmux, and tmux doubt
  (a stale or partial tmux environment) counts as no tmux pane.
  The tmux pane check waits 1 s (`OPERATION_TIMEOUT`); `name`, `whoami`, `unbind` and
  `ls --here` (`binding_command::labeled_caller_pane` through `Host::caller_pane_required`)
  wait 5 s once, no retry, when `TMUX` and `TMUX_PANE` both name a pane. A late tmux there
  is `RECONCILIATION_FAILED` ("tmux did not answer within 5 s while checking the caller
  pane"), not "no pane"; other doubt still means no pane. The architecture suite pins that
  call-site list (`required_caller.rs`).
  Hooks and the caller-session worker use `Host::for_verified_caller` instead, which uses a
  host only when the caller's locators name exactly that host or prove a nested caller's
  owner. No locator keeps tmux (outside any host), and tmux variables alone keep the tmux
  caller check. One named external host must answer conclusively with the caller's pane
  (`Callers::conclusive`: a driver error, a changed or unopenable approved executable or a
  non-text variable is not); anything less refuses and never tries another host. tmux
  variables beside exactly one external host's (nested, in either order) admit only the
  owner that one pinned ancestry proves (`host/nested.rs`). `process::ancestry::pinned_chain`
  reads the caller's chain once, each entry's parent and start from one native read or one
  UTC `ps` row, and refuses a parent that started after its child (a reused pid). The
  driver's `caller` pane and the tmux pane (one `display-message` on the `TMUX` socket, which
  must report that pane, socket and server pid and a live pane) are located on that chain.
  The nearer located pane owns the caller; a conclusive answer whose pane is off the chain
  is absent (the tmux server daemonizes, so the Herdr pane that runs tmux is never on a tmux
  caller's chain). Doubt on either side (a driver that is not conclusive, only one of
  `TMUX` and `TMUX_PANE`, a stale socket, another server or pane, both hosts naming one pane
  process), no located pane, or a change from the caller up to the owner pane before the
  answer refuses and never falls back to the other host. The hook then requires the pane
  pid it reads to be the pinned owner's (`Host::pinned_caller_pane`). Two external hosts'
  variables still refuse. Limits: a pid reused with the same start within the start
  resolution is not caught (microseconds natively, whole seconds from `ps`; on Linux a
  native start is boot time plus clock ticks, so a clock step between two reads can shift
  one entry by a second, which refuses or misses a reuse within that second). Herdr started
  in a tmux that has since closed keeps a stale `TMUX`: an ordinary command gets Herdr, but
  a hook refuses, so clear `TMUX` and `TMUX_PANE` before you start the agent.
  Caller-session discovery after an ordinary command runs its worker for a nested caller
  when its tmux pane or that external host has a binding; the worker's nested proof alone
  chooses the owner ([identity bindings](identity-bindings.md)). An external caller's server
  must already have a `host_servers` row (`RecordedHostServers` reads it and never records
  one).
  `Host::for_target` picks the host whose grammar reads the text. Only a definite answer is
  "not found"; a failing or late driver is `RECONCILIATION_FAILED`.
- Prompt-first `send` maps `no_agent`/`unsupported` to raw input, `blocked` to
  `AwaitingApproval`, `not_ready`/`not_found`/`bad_request` to not sent and anything else to
  uncertain. Raw input is paste with `enter: false`, core's paste-to-Enter delay, then Enter,
  each call with its own deadline. Without `input`, a send is `Unsupported` and the request
  stays queued.
- `drivers/herdr/rust/tmt-driver-herdr` depends only on the protocol crate, `tmt-invoke`, `serde_json`
  and `semver`, never on core or adapters, and has no `tmt-cli` edge. The CLI
  product still admits `tmt-driver-herdr` as an optional archive companion; Herdr
  also has independent alpha archives. Its marker tokens match
  the former built-in host byte for byte (`src/fixtures/builtin-marker.json`); its `input`
  refuses CR/LF because a line break would submit before core's Enter.

## Driver protocol crates

- `tmt-host-grammar` (leaf, no dependencies) defines host name, pane-ID prefix and target
  template once. `tmt-driver-protocol` adds wire types and limits in one `Op` set (a driver
  answers `unsupported` to the other kind's operations), strict `decode`,
  `serve`/`serve_runtime` and conformance checks, over `serde` and `serde_json` only.
- A host's launch template (`LaunchTemplate` in `tmt-host-grammar`, the optional `launch`
  capability) is data core publishes and never runs: Ops expands and runs it. The tmux
  template is the constant in `tmt-adapters::tmux::launch`; `host::launch::launch_templates`
  composes it with each approved driver's (an `ok` driver only) for `tmt host templates`.
  The mark a template leaves is `openedByTmt` evidence; `tmt stop --close` is its reader. It
  runs the verified stop first (`verified_stop`, shared with `resume --move`), then asks the
  host to close that one pane (`HostDriver::{close_preflight, opened_by_tmt, close_pane}`,
  policy in `host::driver::close_opened_pane`). The op is optional and capability-gated: tmux
  closes through `Tmux::close_opened_pane` (one `kill-pane`, never the server's last pane),
  an external driver through its `close` op, and a host without either leaves the pane open.
  An unmarked, unknown or unlisted pane is never closed, and no close is retried.
- A runtime driver's hook path is declarative (`RuntimeDeclaration` plus `decode_hook`, no
  process); only `locations`, `resume` and `usage` run the driver. The client validates the
  `locations` answer and calls `within(home)` before admitting write targets; approval alone
  writes no provider files. The external runtime protocol is defined, but core
  launch, provider hooks and setup do not consume it.

## Built-in host

The built-in host (#4605, design on #1339/#1340) gives a member a terminal when no
tmux or Herdr hosts it. `run`/`resume` default to this host outside a verified host;
`--host tmux|herdr|pty` selects explicitly and never silently falls back. `HostKind`
keeps the existing `External` shape for the reserved `pty` grammar. `Host::for_server`
reaches its in-process `host::pty` driver; no executable approval is involved.
Attach/focus and the Ops stream are not yet shipped.

- **One process per member, no daemon.** `tmt __pty-serve DIRECTORY ID COLS ROWS PROGRAM...`
  (hidden; `tmt-adapters::pty::serve_process`) leaves its session, takes the member's lock,
  binds the socket, starts the program on a new pseudo-terminal (a new session whose
  controlling terminal it is), prints `ready <program pid>` and points its standard streams
  at `/dev/null`, so a launcher can read its pipes to the end. It ends when the program
  does: it tells attached clients (`Exited`), then removes its socket and lock. A supervisor
  that dies, even by SIGKILL, closes the terminal and the kernel hangs the program up; its
  leftover socket and lock are replaced by the next start of the same id (ids are never
  reused).
- **Ownership.** `tmt-pty-protocol` is a dependency-free leaf for the frames
  (`kind: u8`, `length: u32`, payload of at most 64 KiB; strict decode; the version is in
  `Hello`/`Welcome`). `tmt-adapters::pty` owns the effects (`Supervisor`, `Connection`);
  `tmt-sys` owns only the terminal system calls (`spawn_on_terminal`, `set_window_size`,
  `peer_uid`). Core-internal controls use kinds `0x20–0x2f` and `0xa0–0xaf` on the same socket at version 1; these are separate from the attach stream.
- **Access.** The directory must be owner-only (`private_file::ensure_private_directory`,
  shared with the Claude channel), the socket is 0600, a peer whose user differs from the
  supervisor's is refused, at most 16 clients attach, and a client that does not say hello
  within 5 s is dropped. The socket path stays under 100 bytes.
- **Clients.** Several may attach. Output is read whether or not anyone is attached and is
  not retained as scrollback. The supervisor owns one `vt100` screen with scrollback 0, resized with the terminal; capture returns that screen without a redraw nudge. A client with 256 KiB queued pauses reading the program, so
  a slow terminal slows the program instead of losing output; one that reads nothing for 3 s
  while output waits is evicted (`TooSlow`). `TakeSole` evicts the others (`TakenOver`).
- **Window size.** Each axis is 1–1000 (at most one million screen cells). Larger launch sizes refuse with `PTY_SIZE_INVALID`; larger wire sizes are malformed. Clients clamp before sending a resize. Only a client that sent `Resize` has a size. The terminal takes the size
  of the client that last resized, typed after another did, or took sole use, so a delivery
  that never resizes cannot change it. `Resized` tells every client.
- **Input** from all clients is written to the terminal in arrival order; clients are not
  read while 64 KiB are waiting. Real `Input` alone updates the user-key timestamp.
  One bounded Core paste is queued as one frame, without keystroke-sized fragmentation.
  Core controls paste only within bracketed-paste markers when the program enabled that mode,
  then submit Enter separately; Core owns the `!` rewrite, delay and SQLite delivery stages.
  Control clients drain output and disconnect promptly, including between paste and Enter;
  they never resize. Failed post-paste transport is uncertain and never replayed.
- **Managed startup.** `pty::start_member` uses the existing detached-process owner
  to await the supervisor readiness line, retaining kill/reap ownership until that line.
  Child-only locators replace inherited host variables. The ordinary managed launcher
  runs inside the PTY: it waits until the outer launcher is listening, then applies
  its existing bind/session/channel admission and reports the committed marker. Startup
  receipt controls are ephemeral and grant no session authority. An uncertain launch
  names the pane for inspection and never resends or stops a possibly running member.
- **Evidence and markers.** Core observes the supervisor and pane process incarnations
  and verifies their direct parent/child relationship. Publication requires the matching
  pane PID; removal compares the binding key. Errors stay Unknown; only native process
  loss/replacement proves a supervisor dead. The public identity and durable exchange
  still belong to Core storage, not the supervisor.
- **Tests** run real pseudo-terminals with `sh` programs, in process
  (`tmt-adapters/src/pty/tests.rs`) and as a process (`tmt-cli/tests/pty_supervisor.rs`:
  terminal ownership, SIGKILL hang-up and restart, second supervisor, refused starts).

## Agent drivers

- One `DriverDescriptor` (name, executables, hook format and display hue) plus one
  adapter module per driver; `drivers::Registry` joins them in descriptor order for
  setup, detection, skill targets, launch, runtime registration and caller recognition.
  A test requires one adapter module per descriptor. `locate`
  resolves everything against one captured `ProviderEnvironment`; Claude resolves a nonempty
  `CLAUDE_CONFIG_DIR` against the captured working directory, otherwise `~/.claude`, and
  empty counts as unset.
- Detection (`Present`, `ConfigOnly`, `Absent`, `Broken`) reads only the filesystem.
  `Registry::probe_versions` runs one bounded `--version` for diagnostics only; no setup,
  install or status command calls it, because running an agent can write under `HOME`.
- Muse uses executable `muse`, configuration root `~/.config/muse` and the shared
  `~/.agents/skills` root. Its descriptor and adapter enable detection, consented setup of
  skills and explicit `tmt install muse`; they declare neither lifecycle hooks nor a runtime.
  Managed skill ownership and replacement remain with the
  [installer owner](install-architecture.md#managed-skills).
- **Support level.** Each `DriverDescriptor` carries one `Support` constant, the single
  owner of what a driver provides; no other prose lists per-driver capabilities.
  `drivers::tests` pins `Full` to hooks plus a runtime, so the level cannot drift from the
  declarations. `tmt ls` publishes it as
  [`driverSupport`](../../../../contracts/identity-status-v1.md#list-driver-support).

  | Level   | Drivers                                         | Provides                                                                                                                      |
  | ------- | ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
  | `full`  | Claude, Codex                                   | detection, launch, skills, lifecycle hooks (activity, model, consumption), resume, optional message channel                   |
  | `basic` | Gemini, Antigravity (`agy`), Pi, OpenCode, Muse | detection, skills and `tmt install` only; no driver launch, activity, model, usage or resume, so those stay blank, never idle |

  Herdr is not an agent driver: it is an external **host** driver (pane discovery and
  delivery, [driver protocol](../../../../contracts/driver-protocol-v1.md)) with no level,
  and no agent-session support beyond that is shipped.

- Unsupported is distinct from accepted, queued, failed, denied, approval-blocked and
  uncertain; only unsupported falls through automatically. The `session` interface kind is
  reserved, not shipped.

- Pure declarations live in `tmt-core/src/driver/descriptor.rs` and
  `tmt_core::driver::ALL`; adapter implementations live in `drivers/<name>.rs`.
  Only these owners spell a driver's name; the architecture guard rejects exact
  driver-name production literals elsewhere.

## Agent launch environment

The runtime lifecycle owns final foreground argv policy, after optional hooks and
channel composition, so plain fallbacks retain it. Managed Codex interactive launches
use `--no-daemon` and add only `ConfigPaths.global_dir` through `--add-dir`, including
exact resume and Ops launches through Core. Explicit remote connections and provider
utility commands retain their mode; an enrolled channel retains its private attributed
endpoint. No launch edits provider configuration. Drivers also own typed refusal
explanations (reason, why, consequence and fix); shared command output renders them
without provider-name branches. Codex diagnoses confirmed storage write denial only
when `CODEX_SANDBOX` is set, independently of caller attribution.

Shared run/resume launch sets `TMT_AGENT=1` in the child environment, inherited by
its tool shells. The shared browser opener treats a nonempty `TMT_AGENT` like CI:
print the link without automatic opening. Explicit `--open` still overrides that
check; `--no-open` and `--json` keep their precedence.

## Provider channels

- Global `experimental.channel` defaults false. Shared run/resume composition refuses
  explicit `--channel` before launch effects and selects paste for flagless resume
  without rewriting its remembered choice; enabled launches retain driver defaults
  and Required remembered-channel policy. Positively ended enrollments do not block
  paste into an admitted new launch; live or unverifiable pane evidence still does.
- `tmt run --channel` (`run_command/channel.rs`) is the only entry that enrolls.
  `ChannelError::Unsupported` becomes `CHANNEL_UNSUPPORTED`. A default-mode failure before
  foreground startup may fall back to the original command with one reason line, only when
  binding authority and pane enrollment evidence allow; the launcher never recovers
  unconfirmed failed-start evidence to obtain the fallback.
- The port (`tmt_adapters::runtime::channel`) stays free of provider and transport
  dependencies; CLI channel policy has no provider-name branch. The driver persists pane attribution before the child starts and owns
  everything proving a cleanup is for exactly that launch; the CLI neither parses provider
  arguments nor inspects the lease. The launcher calls `foreground_started` once with the exact
  child incarnation, before admission, and retires the lease only when no child was spawned or
  its wait returned. Endpoint records live under `ConfigPaths::channel_directory()`.
- With no enrollment, or one a different proven-current launch has outlived, `send` is
  `Unsupported`; an opted-in session never gets `NotSent` and an unverifiable launch is
  `Denied`. `Unacknowledged` settles as an uncertain wake and `talk` keeps waiting;
  `ChannelUnavailable` stops with `CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE`,
  `CHANNEL_ENROLLMENT_ENDED` or `DELIVERY_PREPARATION_FAILED`, showing the recovery text.
- `enrolled_in_pane` asks each driver by the pane address its enrollments persisted, never a
  stored binding (observation deletes the binding of a pane that lost its marker).
  `enrolled_harness` picks the carrying driver before any preference exists.
- Claude: every mutation of a record or socket runs under one lock file via
  `file_lock::exclusive` and only while the record carries the caller's generation and launch
  owner. The server enables admission under that lock before the ready record is visible, and
  a publication error ends the conversation without writing queued frames. `enroll` takes over
  a same-binding record only when it is positively over, or only its owner was recorded and
  the launch is in the pane it names. A record whose processes are gone has ended; one with no
  recorded foreground is unknown (terminal for its pane, naming `tmt channel recover`); one
  with no pane is skipped and reported. Without discovered configuration the outcome is
  `Denied`, never `NotSent`.
- Codex: `transport` sets one absolute deadline per stage (three-second preparation, then one
  fresh three-second deadline for the consuming attempt on the same connection; fragments never
  renew it). `record` persists provider-private readiness under the existing file lock with no
  capability material. Exact resume uses `thread/resume` and validates the returned UUID;
  arbitrary argv never selects a session. The supervisor owns the endpoint process; launcher
  EOF cleans the endpoint but keeps the enrollment; withdrawal is only for no-child or
  confirmed reap; `foreground_admitted` runs only after committed Running admission. Pruning
  probes outside locks and revalidates the exact snapshot before removal. `tungstenite` is
  adapter-local with default features off. Codex owns folder-trust onboarding. Only
  `provider_hook_command::verified_caller` uses `Storage::context_by_binding`, after
  `ChannelObservation::verified_binding`.

Each provider keeps record layout, locking and launch comparisons private; record
and socket mutations prove generation and launch ownership before replacing enrollment.
`tmt channel inspect|recover` publishes driver reports without owning record logic.
`delivery::guarded_paste` and `delivery::pane_channel_evidence` are the only paste
gates; absence of a record under a stored binding alone never permits paste.
