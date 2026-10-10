# Hosts, process effects, agent drivers and channels

Per-module rules for the host port, tmux and process adapters, agent drivers, provider
channels and the driver protocol. The owner map is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#tmux-and-process-effects); wire formats are in
[driver-protocol-v1](../../../../contracts/driver-protocol-v1.md),
[claude-channel-v1](../../../../contracts/claude-channel-v1.md) and
[codex-channel-v1](../../../../contracts/codex-channel-v1.md).

## Process and tmux effects

- A running-command handle separates launch from wait but keeps the original deadline and
  cleanup; an abandoned handle stops and reaps its child. There is no second runner.
- `process::interactive` runs direct-terminal children: terminal interrupts reach the child,
  wrapper TERM/HUP are forwarded to the owned child only, and a notification failure degrades
  supervision instead of killing a live agent. Abandonment terminates, then kills and reaps
  that child, never the shared process group.
- tmux deadline, I/O, spawn, output-limit and signal failures during target resolution stay
  `RECONCILIATION_FAILED` (exit 1), not `PANE_NOT_FOUND`; a completed lookup with no valid
  pane ID is "no target". Socket denial is `TMUX_PERMISSION_DENIED`, confirmed from the
  socket path's metadata and effective-user write access, never from localized stderr. The
  adapter preserves every child locale variable. The single `Tmux::run` owner adds
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
- `host::driver::{status, send, focus}` hold the one binding policy over each `HostDriver`. A
  host without input is `Unsupported` before evidence is read. A send first offers the message
  to the agent the host recognizes in the pane (`prompt`); only `Unsupported` falls back to raw
  `input`, any other answer is final. tmux recognizes no agents. `DeliveryError` is the
  host-neutral failure: stage, whether text may have reached the pane, and the host's cause.
- A handle has a primary host; a session probes, marks and clears each stored binding on that
  binding's own host. A `Host` states why it was chosen (`for_caller`, `for_server`,
  `for_target`) and takes endpoints, never loose socket or pane strings.
- `HostKind` is pure data (token, `is_pane_id`, `is_target`). Evidence from another host is
  `Unknown`; presence groups by host and socket (`ServerSelector`). A stored host is `tmux` or
  `External(name)`, readable whether or not its driver is installed; a NULL fence host is
  tmux. External syntax comes from approved drivers registered once at start
  (`host::external::register_approved`) in core's write-once registry. Names an approved
  driver reads as targets (Herdr's `wN:pM`) are refused only as new names.
- Herdr's stored token, pane IDs, markers and `host_servers` rows predate its move to a driver
  (#1082): they parse as `External(herdr)`, so earlier bindings read as unknown, never lost,
  until the driver is approved. Without it, a Herdr pane command keeps its result and prints
  `tmt driver install herdr` on stderr once a day per pane (`hint_cadence`).
- Cosmetics: tmux badge and marker refresh (names sanitized before markup); `pane_badge` is
  bounded post-commit presentation, never routing evidence. A badge refresh prefixes the
  inherited border format on that pane only, unless a user-local override or existing badge
  reference already supplies it. `@tmt.border` stores the exact installed format;
  cleanup compares it on the server before unsetting only that unchanged pane override.
  Creation uses set-only-if-unset, and publishes ownership after successful format writing;
  failed ownership publication conservatively leaves the format unowned. No shared option or
  configuration file is changed. Only explicit binding/launch callers request the off-border
  enable-command hint; delivery, hooks and automatic refreshes remain silent. The copied
  inherited format stays fixed until unbind and rebind. External hosts refresh only the marker's name.

## External host drivers

- **Registry** (`<global>/drivers.json`): approval refuses a declaration a built-in host or
  another approved driver would read as its own. `inspect` checks ownership, digest, one
  `capabilities` probe and conflicts and writes nothing; `commit` re-checks conflicts and
  writes. `state` is `ok`, `changed` or `missing`. A first-party approval follows the shipped
  companion: an upgraded driver is adopted without asking only if it declares nothing beyond
  the approved protocol, syntax, operations and `callerEnv`; otherwise it is unavailable until
  approved again. Every read-modify-write holds `drivers.lock` and renames a staged file.
- A record is written only with explicit consent (`tmt driver install`), never by product
  install or upgrade; without a terminal or with `--json` it refuses with
  `DRIVER_CONSENT_REQUIRED`. Removing a driver leaves bindings stored and unavailable.
- `DriverProcess` runs one operation through the bounded owner under its deadline and output
  bound with `TMT_DRIVER_CALL=1`; `executable_trust` (shared with extension hooks) checks
  ownership and fingerprint before every call and the digest once per process; a `tmt`
  started with `TMT_DRIVER_CALL` refuses everything but help and `--version`.
- Evidence is core-led: server identity is core's own `ps` start token for the pid the
  driver's `server` names (the driver's `startTime` is advisory), gone or replaced is Dead,
  same process plus the driver's snapshot is Live, else Unknown. The optional `probe` is not
  called. A removed or changed driver answers `Unavailable`, never proof of loss.
- A driver's `caller` pane counts only when that pane's shell is an ancestor of the caller
  (`process::ancestry`); the nearest verified pane wins, otherwise the host is tmux.
  `Host::for_target` picks the host whose grammar reads the text. Only a definite answer is
  "not found"; a failing or late driver is `RECONCILIATION_FAILED`.
- Prompt-first `send` maps `no_agent`/`unsupported` to raw input, `blocked` to
  `AwaitingApproval`, `not_ready`/`not_found`/`bad_request` to not sent and anything else to
  uncertain. Raw input is paste with `enter: false`, core's paste-to-Enter delay, then Enter,
  each call with its own deadline. Without `input`, a send is `Unsupported`, the request is
  kept and `--inbox` queues.
- `rust/crates/tmt-driver-herdr` depends only on the protocol crate, `tmt-invoke`, `serde_json`
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
- A runtime driver's hook path is declarative (`RuntimeDeclaration` plus `decode_hook`, no
  process); only `locations`, `resume` and `usage` run the driver. The client validates the
  `locations` answer and calls `within(home)` before admitting write targets; approval alone
  writes no provider files. The external runtime protocol is defined, but core
  launch, provider hooks and setup do not consume it.

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
- Unsupported is distinct from accepted, queued, failed, denied, approval-blocked and
  uncertain; only unsupported falls through automatically. The `session` interface kind is
  reserved, not shipped.

- Pure declarations live in `tmt-core/src/driver/descriptor.rs` and
  `tmt_core::driver::ALL`; adapter implementations live in `drivers/<name>.rs`.
  Only these owners spell a driver's name; the architecture guard rejects exact
  driver-name production literals elsewhere.

## Provider channels

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
