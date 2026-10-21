# Architecture

The CLI runtime is the Rust workspace in `rust/`. The `typescript` pnpm workspace owns Vitest, fixture
and release-verification tooling; the repository root has no Node package.
Contributors run Cargo and pnpm scripts directly. The pnpm workspace is not a second CLI runtime, npm product or source-install fallback. A native source checkout selects
`rust/target/debug/tmt` (or an explicitly supplied native executable); a missing
native build is an error. No test, script, or installer may silently execute an
installed host `tmt` or a retired TypeScript product implementation. Node may
run explicit developer fixtures and verifiers, never serve as a product fallback.

Published releases are immutable. Source changes do not publish replacements
or migrate application data.
TMT remains an invocation-owned local CLI, without a remote MCP server, identity
memory or a separate inbox service. The one MCP server it ships is the hidden
`__channel-server`, a stdio server that an opted-in Claude launch starts as its own
child; it listens on no network port.

`better-sqlite3` is a private tooling oracle, never a Rust dependency or native state owner.

## Repository layout

Infra reviews the [layout map/allowlist](.github/repository-layout.json).
[Component ownership](.github/components.json) is separate.

| Home                      | Responsibility                                                                       |
| ------------------------- | ------------------------------------------------------------------------------------ |
| Repository root           | Entry points, contributor guidance, license and required configuration               |
| `.agents/`                | Contributor skills and their area references                                         |
| `.claude-plugin/`         | Claude Code marketplace manifest for the bundled Claude plugins                      |
| `.github/`                | Ownership/layout maps, workflows, shared Actions and isolated release tooling        |
| `rust/`                   | CLI, core, adapters, neutral leaves, private fixtures/release tools and archive note |
| `typescript/`             | Private developer tooling, tests and shared fixtures                                 |
| `extensions/<extension>/` | Product-owned runtimes, contracts, skills, docs and assets                           |
| `drivers/<driver>/`       | Product-owned driver runtimes, contracts and assets                                  |
| `contracts/`              | Core public contracts and normative fixtures                                         |
| `scripts/`                | Shared shell/build/development helpers                                               |
| `skills/`                 | Canonical bundled user-agent guidance                                                |
| `site/`                   | Public Home/Colab landing, blog, handbook sources and translations                   |
| `design/`                 | Tokens/CLI; [private browser-ui](design/gui-components.md); no Core/CLI dep/embed    |

New homes or exceptions need infra review and coordinated map/allowlist changes; ignored outputs
are outside the tracked-file map. The [layout skill](.agents/skills/tmt-layout/SKILL.md)
owns moves and the tracked-file guard. Handbook language exceptions belong to
[AGENTS](AGENTS.md#repository-content-language) and the allowlist.

`tmt-ux-lead` owns shared visual tokens and [private browser UI](design/browser-ui/README.md).
`./ui` isolates React/Radix/cmdk/CVA, CSS/themes/portals; legacy/static unchanged; no domain state.
Layers retain Radix focus/dismiss ownership and host close admission; layer integration contracts live in the linked leaf README.
[Release procedures](.agents/skills/tmt-release/SKILL.md) include the archive’s
product-neutral `rust/archive/NATIVE-INSTALL.md`; [dev-only embed](site/README.md) stays site-owned.

## Public blog boundary

`site/src/blog/` builds `site/src/posts/` into the shared site artifact.
[The site guide](site/README.md#blog) owns content, approval, CSP and base paths.

## TypeScript workspace boundary

The `typescript` pnpm workspace owns private Node tooling/tests and one lockfile,
with parent-relative packages for the parked browser-addon shell, Remote client SDK,
and Colab browser/client packages.
It is not a second CLI runtime. Packages resolve declared dependencies, never
root-hoisted tooling; browser specs reach the tooling SQLite oracle through
`typescript/test/support`. Rust, root launchers, core contracts and canonical
skills stay outside this boundary; extension contracts stay with their product.

Vite+ supplies one Vitest runner, aliased Vite core and bundled Oxfmt/Oxlint.
Packages retain their separate configs, plugins, formatter selections and warning
policies. `typescript/scripts/format-workspace.mjs` owns root code/docs selections;
`lint-config.mjs` owns shared lint rules. Compiler commands retain their owners;
site and release tooling stay outside this lockfile.

### CI selection and worker model

[`.github/components.json`](.github/components.json) is the one component map:
`owns`/`excludes` define path roots, `selectedBy` overrides ownership for scattered
files, and ordered rules select CI consumers independently. `release:false`
excludes a component from automatic cuts/publication. Only non-released components
may declare `releaseStatus:never` (never shipped) or `releaseStatus:parked`
(explicitly deferred); absence means awaiting activation. `releaseConsumers` names
packaged consumers of private components. Registration alone never authorizes activation.

`typescript/scripts/ci-scope.mjs` owns map validation, path ownership, conservative
CI selection and final gate validation. Its `releasedComponentsForPath` is the
shared release-attribution owner: released roots plus each binary's transitive
Cargo normal/build workspace dependencies; dev edges do not count. Explicit
private non-Rust consumers are additive. `cargo-workspace.mjs` supplies resolved
Cargo metadata; version inheritance/editing has its own private release-tool owner.
CI scope, ownership, binary consumption and version inheritance are separate contracts.

Project-only never-shipped path declarations are defined in the
[release-tracking reference](.agents/skills/tmt-release/references/native-release.md#project-release-tracking).
The separate `tmt-cli` architecture guard checks all Rust files and macro tokens outside
declared roots, packaging and canonical generated inputs without cfg/reachability inference.
Required CI covers the component map, declaring crates and release build script; the Node
Project sweep validates declarations and never executes captured source.

Selected missing, failed, cancelled or unexpectedly skipped work and empty discovery
cannot satisfy required gates; selection, worker, cache and advisory-browser details live in the
[CI reference](.agents/skills/tmt-release/references/ci-selection.md). `pr-title-check.mjs` owns
released-path and cumulative squash-title gates; the [release reference](.agents/skills/tmt-release/references/native-release.md#conventional-pr-titles)
owns edit-only feedback and explicit report-only compatibility. Publication reuses the native
aggregate's scope-skip proof with check-suite provenance, not recomputed selection or bare skips.

## Runtime layers

The Rust crates have deliberately narrow responsibilities. Module-level rules for
the core crates live in the [tmt-core-runtime skill](.agents/skills/tmt-core-runtime/SKILL.md).

| Layer             | Owner                           | Responsibility                                                                                                                                                                                                  |
| ----------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure domain       | `rust/crates/tmt-core/src/`     | Identity, names, bindings, profiles, settings, retention, request state and native-install version policy. No filesystem, process, SQLite, tmux, network or CLI framework.                                      |
| Concrete adapters | `rust/crates/tmt-adapters/src/` | Config files, SQLite, bounded files and processes, signals, tmux evidence/transport, response input, HTTP acquisition, native release publication and managed skill files.                                      |
| Application/CLI   | `rust/crates/tmt-cli/src/`      | Core grammar, typed invocations, preflight and use-case composition, completion and the executable entry point. It chooses adapters; it does not duplicate their storage, file, installation or process policy. |

`rust/crates/tmt-command-output` owns shared command output/error values and
formatting. It renders human text through `rust/crates/tmt-cli-style`, the one
implementation of the [CLI style](design/cli-style.md) (palette, themes over the
design tokens, marks, values, messages, lists, tables, the column-width solver
`grid` that tables and extension boards share, the help registration contract and
`Interaction`). Both are leaves with no TMT dependency so extension CLIs can share
them; the architecture guard enforces that. `mark::Mark` owns each shared mark's
symbol, description and style token.

`rust/crates/tmt-tui` is an internal, unpublished, application-neutral presentation
leaf for TMT terminal UIs (markup admission, one Taffy geometry computation, Ratatui
paint, reusable components). It has no application vocabulary, acquires no terminal, clock,
settings or provider data, and takes its tokens from `tmt-cli-style` roles. The guard
permits only XML parsing, borrowed JSON, shared style, private Taffy and Ratatui, never
core, adapters, CLI or extension behavior. Ops is its sole reviewed consumer;
new consumers/dependencies go to tmt-lead. See the [`tmt-tui` skill](.agents/skills/tmt-tui/SKILL.md).

`rust/crates/tmt-invoke` owns executable discovery, bounded byte capture, child-only
environment overlays and browser launch. It has no TMT dependencies; callers own
command selection, interaction and presentation.

`rust/crates/tmt-cli/tests/architecture.rs` owns the syntactic module/dependency guard;
[Development](DEVELOPMENT.md#architecture-guard) owns its checks. It supplements behavior review.
`tmt-sys` is the single audited `unsafe` boundary: only `tmt-adapters` may depend
on it and every other crate forbids unsafe code.

Workspace quality checks cover the unified feature graph. Native process fixtures
build products separately to keep ordinary CLI feature isolation;
[Development](DEVELOPMENT.md#rust-checks) owns their selection.

## Public command boundary

The grammar (`rust/crates/tmt-cli/src/grammar.rs` and its `grammar/` modules) owns
core command registration, option placement and rejection, and the help projection;
each owner's parser turns its grammar into typed invocations and publishes through
`tmt-command-output`. Hidden commands parse for internal workflows but never appear
in help or completion. Handlers never search raw argv or reinterpret payload text
as flags. Human/JSON share typed result/status contracts; [list](contracts/identity-status-v1.md#list-runtime-driver) owns runtime-driver proof.
Command dispatch, help and completion rules are in the
[extension surface reference](.agents/skills/tmt-core-runtime/references/extension-surface.md).

### External command contract (v1)

An unknown root command named `[a-z0-9][a-z0-9-]*` resolves `tmt-<name>` on PATH;
core wins every collision. On Unix the adapter `exec`s, so the extension inherits
stdio, TTY and signals and its exit status is the command's. `core_executable` hands
companions the managed stable launcher via `TMT_EXECUTABLE`, else the invoking executable.
Core keeps no registry, manifest, daemon or extension
state. Only arguments after the name are passed, verbatim. Completion v1 is
optional, bounded and falls back to file completion on any failure.

### Local extension API (v1)

`tmt api` is the public, same-user process port for machine-shaped gaps in the
ordinary CLI: one versioned JSON request on stdin, one JSON resource or error on
stdout. It is neither an authentication boundary nor a daemon, batch or stream.
`tmt-adapters::api` owns envelope admission and composition; the CLI owns bounded
stdin, publication and exit status. Protocol major 1 accepts additive operations and
fields; incompatible changes need a new major. `extensions.uses` answers an extension's optional use of another from installed
receipts only (no network, storage or extension process); the extension checks it
when the feature starts. Human-shaped operations remain their
ordinary JSON commands, not duplicate API implementations. The
[extension API contract](contracts/extension-api.md) owns the protocol and operation contracts.

### Local MCP (v1)

`tmt mcp --identity <saved-name-or-uuid>` is an agent-launched stdio interface over
the existing exchange. The [MCP contract](contracts/mcp-v1.md) owns its wire,
schemas and bounds; `tmt-adapters::mcp` owns admission and framing and
`tmt-cli::mcp_command` composes the existing command owners in process. It adds no
exchange state, persistence or retry semantics and is separate from the private
Claude channel server.

### Extension hooks (v1)

`tmt-adapters::extension_hooks` delivers consented, best-effort lifecycle
observations and context lines to verified `tmt-<name>` executables
([wire contract](contracts/extension-api.md#lifecycle-hooks)). PATH discovery alone
never runs a hook; ownership and a stat fingerprint are re-checked before each
delivery. Capture is per connection and transactional, delivery is bounded and
never changes a command's result, and a nested `tmt` captures nothing. Extension
summaries are untrusted informational data. With no consent file a command spawns
nothing.

### Core command surface

The grammar owns primary names and accepted aliases; core reserves them before
external dispatch. The public surface groups into setup and guidance (`init`,
`config`, `completion`, `help`, `learn`, `install`, `setup`); identities and
bindings (`identity`, `ls`, `add`, `name`/`this`, `marked`, `whoami`, `unbind`,
`rm`, `mv`, `notes path`); foreground launch and channels (`run`, `resume`,
`channel`); profiles and exchange (`role`, `preamble`, `x`, `reply`, `result`,
`inbox`, `answer`, `talk`/`send`, `check`/`read`); rooms and recovery (`room`,
`workspace`); `focus`; local extension interfaces (`api`, `mcp`); native
installation (`upgrade`/`update`, `uninstall`, `extension`, `driver`). Hidden
internal commands are not a public surface.
Core owns workspace geometry/generations; adapters, atomic publication/fenced
creation; CLI, consent/output. Guarded restore pins bytes; full restore uses ordinary
resume/external dispatch in created panes. Layout-only grants no launch authority.
One typed result serves human and JSON output; `identity show`
without a name uses the shared verified-caller selector and never falls back to
a working directory, active pane or sole identity.

## Domain and state ownership

### Identity, names and bindings

- `tmt-core::names` owns canonical names; hosts own pane-target syntax.
  `identity` owns lifetime/create/promote; `identity_metadata`/`identity_status`
  own untrusted descriptions, never authority ([atomic metadata](contracts/extension-api.md#conditional-identity-metadata)).
  `binding` owns evidence, retirement, binds and UUID/name-CAS rename.
  [Discovery](contracts/discovery-v1.md) owns candidate bounds and native/UUID fences; guarded `add` uses `binding`.
- Unknown or conflicting endpoint evidence is never proof of death. Saved
  identities detach and stay offline; temporary identities retire only on
  conclusive evidence or explicit unbind, and exchanges are kept. Presence is
  observation, not routing permission: a marker or socket cannot authorize a
  different identity, and the pane marker proves ownership by IDs, never by name.
- `binding::session` separates identity-owned preferences from binding-owned runtime
  observations, with compare-and-set writes inside the binding transaction.
  Drivers verify processes, map events and own state; core never parses it. CLI learns direct sessions after output via this admission/CAS,
  with bounded silent refusal, storage-only prechecks and private TTL hints.
  Provider end leaves readiness Unknown until fresh start; only proven process loss
  ends the incarnation. Claude approval notifications set waiting;
  only admitted events clear it, never a timer.
- Presence reads acquire host evidence outside the database writer lock and
  recheck their captured records before reconciliation. Changed records never
  authorize retirement or detachment; unchanged records retain conclusive stale
  binding cleanup.
- Lifecycle hooks observe existing bindings; they never create or move identities.
  Bounded callbacks exit zero; launch hooks retain reviewed externalized-name compatibility. Persistent
  provider configuration changes only through consented `tmt setup`.
- `tmt-core::endpoint::ProcessIncarnation` (PID plus core's own start token) is the
  one value for comparing local processes.
- Concrete implementations: `storage::{identities,identity_metadata,identity_status,bindings}`
  and `tmux::{metadata,evidence,binding,caller,transport}`; `binding_command`
  checks caller/target; CLI `target` selects recorded servers. Module rules:
  [identity and bindings](.agents/skills/tmt-core-runtime/references/identity-bindings.md)
  and [workspace recovery](.agents/skills/tmt-core-runtime/references/workspace.md).

### Saved identity notes

`NotesIdentityId` (a saved identity's canonical UUIDv4) is the capability boundary
for notebook storage; display names never become paths. `ConfigPaths` is the sole
layout owner and `tmt-adapters::notes` alone creates
`<global_dir>/notes/<identity-uuid>/notes.md` with owner-only, no-follow creation.
The file body, concurrency and retention are ordinary user-filesystem concerns:
there is no SQLite copy, lock, size policy or secure deletion, and retirement leaves
notebooks in place.
Admitted compaction context reminds saved identities through global `notes.compactionReminder`
(default true), within the existing hook budget and without notebook creation or content access.

### Settings and configuration

`tmt-adapters::config::ConfigPaths` is the sole path owner and managed-release-only default cutover owner, refusing a new database beside a former store;
`config::document` preserves unknown JSON fields and validates known settings
through `tmt-core::settings`; `init` creates the local file exclusively and never
opens SQLite or tmux. Global `theme.base` writes reuse the CLI style base registry; the global `theme` object is presentation, interpreted only by
`tmt-cli-style` (and read by Ops through `config show`); a bad theme never fails
configuration loading. Only `tmt-cli-style` names colors. Details are in the
[storage and requests reference](.agents/skills/tmt-core-runtime/references/requests-storage.md#configuration-and-theme).

### SQLite and durable exchanges

`tmt-adapters::storage` owns one private synchronous `rusqlite` connection, the
schema migrations, WAL/foreign-key/FTS5 setup, busy and transaction boundaries and
cleanup, and exposes narrow ports to core services. Migrations keep recorded names
and retention, refuse customized table definitions instead of rebuilding them, and
every core-owned table advances the durable change cursor through triggers that a
test requires new tables and columns to extend. Typed not-writable storage failures
are classified once and projected through `tmt-command-output::Failure::storage_access`.

`tmt-core::request::RequestService` owns preparation, delivery-state transitions,
exact final submission, waiter release, attention revisions and bounded retention
housekeeping; `storage::requests` owns SQL and cleanup; `request::attention` owns attention.
`request::digest` owns [delivery/history](contracts/extension-api.md#digest-policy-and-checklist); `request::flow` owns [content-free aggregates](contracts/request-response-v1.md#content-free-exchange-flow).
Adapters admit idle handoff; extensions schedule. Clocks sample at transaction entry;
no transaction spans transport, and uncertainty never authorizes replay. Finals are
immutable and terminal text never proves completion. Reads never acknowledge; originator and recipient
acknowledgment are independent. `RequestRoute` separates unbound pane delivery from
the durable identity inbox, which settles `queued`. Unbound identity delivery uses
the foreground observer and recipient pull. Reply notice windows are persisted and composed by
`request::notification` and `delivery::notices`, with finite detached workers owned
by `process::detached`. See the [request contract](contracts/request-response-v1.md)
for behavior and limits; module rules: [storage and requests reference](.agents/skills/tmt-core-runtime/references/requests-storage.md).

### Tmux and process effects

`tmt-adapters::process` is the one bounded subprocess owner (output caps, monotonic
deadlines, process-group cleanup, reaping); `process::interactive` owns direct
terminal children without taking the shared process group. `tmux` uses explicit
socket/server evidence, bounded budgets and no ambient host fallback; a failed paste
or Enter is uncertain and never retried as unsent; [pane cosmetics](.agents/skills/tmt-core-runtime/references/hosts-drivers.md) own only pane-local badge overrides.

The CLI and the `delivery` and `pane_badge` adapters reach a terminal only through
`tmt-adapters::host::Host`; extensions never do (they read `tmt ls --json` and
`tmt whoami`), and the architecture guard rejects extension code that names the host
port, the tmux module or core's `binding`, `endpoint` or `host` model. Every host,
the built-in tmux and external drivers alike, implements `host::driver::HostDriver`,
and one binding policy in `host::driver::{status, send, focus, swap_into}` decides which
evidence makes a binding present and when input is blocked. A host with agent
names (Herdr) gets the identity's name as the agent's name, in one direction
only: the host's name is shown, never evidence and never copied back. Endpoint identity is
opaque to everything but its host: `HostKind` is pure data, evidence from another
host is `Unknown`, and only `tmt-core/src/host.rs`, `tmt-adapters/src/host.rs` and
`tmux/` may spell a built-in host's name. Core's delivery policy rewrites ASCII `!` to
fullwidth `！` in any text typed into a pane. Details are in the
[hosts and drivers reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md).

### Agent drivers

Each `DriverDescriptor` and adapter feed `drivers::Registry` for detection, setup,
launch, caller recognition, final foreground argv and typed caller/storage refusal explanations. The descriptor owns each driver's support level, `full`
(hooks and a runtime) or `basic` (detection and skills), which `tmt ls` publishes as
`driverSupport`. Detection never starts an agent. Claude owns the prompt-picker plugin source (`drivers/claude/plugins/`, published by the root `.claude-plugin` marketplace and installed by the user) and a read-only check that hints when it is missing; a launch adds no plugin arguments and no persistent provider settings change. Module rules and name ownership:
[agent drivers](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#agent-drivers).

### Provider channels

`tmt-adapters::runtime::channel` is the provider-neutral port; drivers own exact-launch
enrollment, transport and recovery. Delivery prefers the channel and forbids paste
on enrolled or uncertain evidence; no receipt means no replay. Module gates:
[provider channels](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#provider-channels).
The [Claude](contracts/claude-channel-v1.md) and [Codex](contracts/codex-channel-v1.md)
contracts own behavior and limits.

### Driver protocol

`tmt-driver-protocol` owns host/runtime wire types and `tmt-host-grammar` owns host
syntax; [driver-protocol-v1](contracts/driver-protocol-v1.md) owns the wire contract.
`tmt-adapters::driver_protocol` owns explicit approval, the automatic approval of the
driver the running release ships (`registry::approve_shipped`) and bounded calls, while
`host::external` composes approved hosts. Core decides evidence; unavailable drivers
never prove loss. External runtime launch/setup/hooks remain unwired. Details:
[hosts and drivers](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#driver-protocol-crates).

## Managed skills and native installation

Managed agent guidance is a filesystem concern separate from application state.
`tmt_core::skill_catalog` is the one list of bundled skill names; the bundle is
embedded and materialized by digest under `skill_installation`, and the
architecture test fails on a skill-name list anywhere else. Core install exposes
only `tmt` and `tmt-inbox`. Retired bundle names are verification metadata only: full
historical inventories and framed digests still prove old Core-name replacement.
Refresh and default uninstall preserve retired skill links, owner records and backing
generations regardless of the recorded owner; explicit generic owner removal retains
its existing authority, and explicit `--purge` retains its existing semantics. No retired product
catalog, embedded source, installation or service cleanup remains. Skill installation never opens configuration,
SQLite or tmux. Core bundled and verified official release publication replaces
any existing leaf at a catalog skill name in selected roots, without prompt, flag, backup or prior target
ownership/content checks. Other names and symlink destinations remain untouched;
immutable source validation and overlap guards remain. Missing recorded targets
stay missing during refresh.
Extension-owned skills arrive as bytes through `skills.install`/`skills.remove`
(explicit consent), are stored per owner and linked into the same roots; the first
owner of a name keeps it until an explicit force and core names are reserved.
API-supplied names refuse unmanaged targets; explicit force backs them up before
replacement. An API name or owner alone cannot authorize catalog replacement.
Native release skills publish after binary consent. Skill failures after successful
binary activation are warnings with path/cause and partial publication; human and
JSON summaries retain each product's activation and skills outcome.

Native executable installation is a different owner under `tmt-adapters::native_install`.
`Product` owns installable products; archive data adds none. Every product uses one acquisition,
receipt and atomic-publication path with independent links, lock and current
release, and the active executable is the authority for a managed update: receipts
anchor to the installation prefix, not to configuration roots. Verification precedes
execution, publication runs the release verifier before the receipt so a rejection
keeps the previous release, and failure or cancellation never leaves a half-published
current release. CLI self-upgrade delegates to the verified candidate under the
[handoff contract](contracts/native-install-handoff-v1.md); PR channel trust,
compiled schema export and admission are owned by the [PR channel contract](contracts/native-pr-channel.md). The candidate then lets that CLI
run the consented extension phase; there is no rollback or second installer.
`tmt extension` owns consented install/upgrade/removal and read-only listing; install/upgrade accept `--to`.
Receipt-frozen [CLI minima](contracts/extension-api.md#optional-cross-extension-uses) gate activation; listing compares locally.
[Installer architecture](.agents/skills/tmt-core-runtime/references/install-architecture.md) owns retention,
discovery, receipts, skills, repair and handoff; [release procedures](.agents/skills/tmt-release/SKILL.md) own verification and publication.

Digest owns [history, pricing and valuation](extensions/tmt-digest/contracts/digest-v1.md); tick collects, status reads.
A `user`-marked identity is never held. An `auto` member is held through the same Core policy as an interval member: the tick renews the hold and offers idle and capped [auto delivery](extensions/tmt-digest/contracts/digest-tick-v1.md#auto-delivery), while Core and the provider's turn end deliver; Core's one cap option skips only the live-idle evidence.
Prices are one [public file](extensions/tmt-digest/contracts/digest-v1.md#pricing-file), embedded as the built-in table and refetched anonymously by status only.
The [usage report](extensions/tmt-digest/contracts/usage-v1.md) joins Core's rollup, limits and `x flow --bucket` per run; it stores nothing.

## Ops extension

`extensions/tmt-ops/rust/tmt-ops` builds optional `tmt-ops`, dispatched as `tmt ops`.
`ui`, `hotkeys`, `skill` and `playbook` are Ops commands; member/state commands
use `squad` (alias `sq`). Releases use `tmt-ops-v<version>`.
[Ops](.agents/skills/tmt-ops-dev/SKILL.md) and [TUI](.agents/skills/tmt-tui/SKILL.md)
own module/drawing boundaries and verification.

- **Seam.** Ops uses public `tmt --json`/`tmt api` (minute `workspace.capture`) and starts `tmt digest tick`
  through `TMT_EXECUTABLE` (else PATH), bounded by `tmt-invoke`. Digest owns
  locking, deadlines and delivery. No direct Core state, tmux or provider writes.
  The guard rejects TMT-to-Ops and Ops-to-Core Cargo/source edges. Dependencies
  are `tmt-tui`, `tmt-cli-style`, `tmt-invoke`, `tmt-extension-state` and dev-only
  `tmt-test-support`. New API methods need tmt-lead review.
- **Data.** Squads are Core rooms `squad-<name>`; fields are identity metadata
  `squad.<name>.<field>`, with no Ops membership store. Ops owns `<dataRoot>/ops`
  (`storage.root`) and disposable `$XDG_CACHE_HOME/tmt-ops` caches. The workspace
  never touches the retired internal `cache/board` display format; existing
  files remain. `ops.toml` is user-owned: agents never write it;
  Ops uses CAS; cron/checklists live in
  [`ops.db`](.agents/skills/tmt-ops-dev/references/data-and-state.md#ops-state-database).
  `migration` owns locked byte-preserving legacy cutover. Its shared invocation notice warns once on CLI stderr; the UI projects the same notice through App without out-of-band terminal writes.
  No Ops
  data enters `config.json` or Core's database.
- **Workspace.** Agents/Digest share one acquired roster; the flat Agents list caches canonical all-agent projections, while tree projections are frame-scoped.
  UUID squad anchors and virtual Common pool define selection; paint/input do no Core reads. WP1 owns conversation/editor/drafts.
  `board::buttons` owns chip paint and hover, including ChipGroup selectors, through neutral Strip paint and shared screen styles; callers keep activation authority.
  The existing quiet cache lane publishes per-name view hints (default keeps its
  original path): destination/UUID anchors, expansion, committed filters/sort/search,
  chart controls, pane ratios and pane scroll. Shared Config preferences stay separate; exec carries the layer
  and session-only visibility. [View-state ownership](.agents/skills/tmt-ops-dev/references/board.md#workspace-and-retained-identity).
  The shell replaces named-tab/factory/switcher navigation and HOME cursors; reload carries UUID anchors.
  `labels::Reader` acquires Config-selected windows; `labels::digest` sums
  [supplier estimates](extensions/tmt-digest/contracts/digest-v1.md#savings), never provider history.
  `usage_report::Reader` acquires public Core usage/traffic off-thread;
  [Ops acquisition](.agents/skills/tmt-ops-dev/references/refresh-and-meter.md) owns its range and lifecycle.
  Member references resolve through the acquired global identity inventory;
  Markdown carries UUID tags into clipped frame hits. Existing menu/actions own
  navigation, while copy/annotation retain raw source. No text grants new delivery
  authority or paint/input I/O. [Reference ownership](.agents/skills/tmt-ops-dev/references/board-notebooks.md).
  Membership removal pauses jobs before leave;
  only explicit Resume clears pause. Rename CAS keeps the room UUID, then moves
  Ops metadata/config and cron namespaces under owned locks; partial results
  offer repair without room rollback. [Surface ownership](.agents/skills/tmt-ops-dev/references/board-home.md).
- **Board rendering.** The [Ops board guide](.agents/skills/tmt-ops-dev/references/board-home.md#shared-board-components) owns shared painter responsibilities and their host adapters.
- **Discovery.** The board consumes [Core discovery
  v1](contracts/discovery-v1.md) through finite public `discover` and guarded
  `add` commands on its existing ordered lane. Its private controller owns
  explicit review, bounded scan generations and retained per-item outcomes;
  paint/input perform no discovery I/O. Core retains native/digest/identity
  authority, acquisition limits and binding effects. Successful public refresh
  selects the returned UUID through the workspace projection.
- **Checklist.** `checklist_command` uses the existing caller/room admission and
  versioned room-UUID JSON in `ops.db`. Reads create no files;
  failed prepublication preserves bytes; uncertainty remains Unknown after
  readback. The board consumes the same typed service.
- **Contracts.** The [embedded skill](extensions/tmt-ops/skills/tmt-ops/SKILL.md)
  owns `sq ls`, `config`, `checklist` and `cron` JSON; native `ops.test.ts` verifies
  them. Row JSON changes update that skill and its native regression in the same
  PR. The Ops developer references own worker fences, cache/timing fields,
  Core-owned Digest policy and independent CLI display projections.

## Testing and evidence boundaries

Rust tests stay beside their owners. TypeScript native, E2E and tooling
suites share `test/support`, which imports no suite; native and E2E import neither
each other nor tooling. Scenarios retain assertions; helpers own fixture
mechanics. Frozen inputs and independent SQL/schema oracles must not derive
expected results from the implementation under test.

Tests select a task-owned native executable or the checkout build; missing
builds fail without host or retired-runtime fallback. State, provider roots,
prefixes, sockets and processes remain fixture-owned. Adapters' default-off
`test-isolation` feature admits private discovery paths in consumer test builds.
`typescript/scripts/private-world.mjs` runs suite commands in a private HOME/XDG/TMPDIR world
(under `/tmp` when the temporary directory is too long for sockets) and reports what they leave behind; Ops test builds resolve their cache to a process-private root.
Native fixtures isolate ancestry and tmux discovery; Docker owns isolated tmux
and peers. Host tmux, provider installs and global environment mutation are not evidence.

Cleanup confirms owned child/group absence before deleting fixture state;
unknown inspection, leaks and false positives fail. Signals target only verified
owned processes. Runtime tests prove CLI behavior, Docker proves transport and
lifecycle, and release tooling proves actual archives and public installation;
one layer's success cannot substitute for another's evidence.

Helper ownership, fixture publication and lifecycle details live in the
[E2E references](.agents/skills/tmt-e2e/references/test-boundaries.md); shared
commands remain in [DEVELOPMENT](DEVELOPMENT.md#native-process-and-shared-tests).

## Release boundary

Release tooling shares native runtime/linkage proof across cargo-dist archives,
installers, upgrades and public smoke. Generated extension minima cover product
registration and declaration support; cut metadata freezes the published CLI.
Raw binaries prove neither archives nor public installs. [Handoffs](contracts/native-install-handoff-v1.md)
and [records](contracts/release-index-v1.md) bind release bytes to identity.
Archive, installer, verifier, publication, compiled schema and PR producer/cleanup sources belong to [tmt-release](.agents/skills/tmt-release/SKILL.md).

### Main release cuts

A release is a product-prefixed tag on a main commit. `release.yml` admits main
pushes by cadence, with hourly backup and manual dispatch. Allocation captures
main once and reserves each released component's next alpha number from drafts
and tags; each allocated tag owns an independent pipeline. New work is measured
from the newest non-failed allocated ancestor cut, whether in flight or published.
Verification-failed drafts reserve numbers but allow replacement cuts, including
at the same main commit; they stay unpublished and do not hold later cuts or merges.

The release version is injected at build through the private `tmt-release-tool`:
only the selected version declaration and implied Cargo lock entries may differ
from the captured source. Build metadata and executable versions must agree;
nothing is committed back to main. Main retains development versions. Workflow jobs
own Node architecture selection; version injection preserves it.

Notes, migration comparison and breaking authorization share the newest published
ancestor. Before own publication, a predecessor supplies history; both lines reserve versions.
Retired identities stay historical. Drafts never advance published evidence. Tags become
immutable after all gates pass; only the CLI converges latest to its highest publication.

Automatic publication covers authorized existing alpha products only. Ben retains
stable, breaking, version-line changes and manual publication authorization.
Activating a new released product is a component-map change accepted by tmt-lead
and the owning squad lead. Exact gates and owner recovery
operations belong to the [release skill](.agents/skills/tmt-release/SKILL.md) and
[main-cut reference](.agents/skills/tmt-release/references/main-cuts.md).

Delivery and publication evidence are separate. Release reconciliation rules and
procedures live in [tmt-release](.agents/skills/tmt-release/SKILL.md#project-release-reconciliation);
shared issue lifecycle definitions stay in
[DEVELOPMENT](DEVELOPMENT.md#project-tracking).

## Maintenance contract

Update this map in the same change when responsibility, dependency direction,
command/error contracts, storage schema or lifecycle, trust boundaries,
resource ownership, shared test infrastructure or release evidence changes.
Keep a significant decision's alternatives, failure behavior and verification
plan in its issue and reflect the delivered boundary here. A green formatter or
checkmark is not architecture evidence.

This file keeps owner maps, dependency direction and cross-cutting invariants, within
the line and byte budgets `typescript/test/tooling/guide-budget.test.ts` enforces. Module-level
rules belong in the owning area skill (`.agents/skills/tmt-core-runtime` for core); a
line that only explains one module's code goes there, not here.

Every change reports its architecture impact and names the affected Rust owner,
adapter, CLI composition and tests. New policy belongs in the existing owner;
do not add a parallel TypeScript implementation, provider inventory, config path
registry, release catalog, process runner, archive parser or memory/MCP layer.

## Shared extension state layout

`rust/crates/tmt-extension-state` is Remote's unpublished leaf.
Store owns managed/adopted SQLite transactions and typed newer refusals.
Document/Guard own atomic byte updates/fail-fast ownership.
Existing Store files use lstat to preserve locks.
Remote, Colab, Ops, adapters' config and Digest history consume it; no TMT/crypto.
Core and Colab's model cannot consume it. Guards check reviewed
manifest/source edges across dependency kinds, aliases and targets.

[Crate docs](rust/crates/tmt-extension-state/src/lib.rs) own admission and publication.
Schemas, identity derivation and Remote's serve-lock proof remain extension-owned;
the leaf discovers no roots and interprets no Core state or provider configuration.

## Remote extension pilot

`tmt remote`: `extensions/tmt-remote`. Fixed public Core JSON uses absolute
`TMT_EXECUTABLE` via `tmt-invoke`. Remote owns `<dataRoot>/remote/` via
[state leaf](#shared-extension-state-layout). Core delegates replaced Remote restart;
Remote owns listener/state/[lifecycle admission](contracts/remote-channel-v1.md#local-cli-discovery).
Home/Remote/SDK; read-only auto-pair; no companions/skills; [release gates](.agents/skills/tmt-release/SKILL.md).

[Contract](contracts/remote-channel-v1.md): driver auth, wire, operations,
settings/mounts/describe/lifecycle/discovery and [hard lines](contracts/remote-channel-v1.md#user-path).
Remote owns authority; UI grants none. Local: keys/grants/signatures/last-use/cleanup.
Firestore: Auth/Rules only; Core never uses Remote trust.
`message`: local envelopes/grants/scope/allowlist/hold/uncertainty/audit.
Proposed (#3290): `talk` → Remote interface → target host → driver.
Local: `transport`/`routes` → `message` → `session`/`operations`; same-ID public Core API dispatch/recovery.
Door: `remote-v1.js` ([SDK](extensions/tmt-remote/typescript/remote-client/README.md));
owner-installed [Colab](#colab-extension) mounts at `/r/<prefix>/x/colab/`; public CLI/SDK/`tmt-extension-objects` elsewhere; never Remote state files.

`object_service`: leased object/v3 carrier; `firebase_auth`: credentials/HTTP.
Serve: Auth/runtime/factory/tick/joins/flush; v3 opt-in.
`native_sync`: COUNT/read limits; Colab cursors; no Core/carrier. Emulation never ships.
[object-backends](.agents/skills/tmt-remote/references/object-backends.md): channels/backends/quotas.
Deploy: `hosting` hash; frozen consent/history/`--details`; Rules/Hosting fences; EXPECTED+ACTIVE/live ID/readiness.
[Modules](.agents/skills/tmt-remote/references/architecture-internals.md);
[door/discovery](.agents/skills/tmt-remote/references/door-and-discovery.md): lifecycle/status/management.
[Background](contracts/extension-serve-v1.md).

## Colab extension

Colab (`extensions/tmt-colab/`: `tmt-colab`, `tmt-colab-model`, `@tmt/colab-client`,
`@tmt/colab-app`) embeds its app and skill; the opt-in archive installer uses `skills/`.
`tmt colab skill` reads embedded bytes without core or storage.
[colab-v1](extensions/tmt-colab/contracts/colab-v1.md) owns behavior and hosted packaging;
the [development skill](.agents/skills/tmt-colab/SKILL.md) owns modules and checks.
Hosted links reuse Remote's SDK, joined interests and Colab admission/own sealing.

- **Layer.** Colab is an app on Remote with no door of its own. `tmt-colab serve`
  listens only on the owner-only socket `<dataRoot>/colab/door.sock`. Remote mounts it at
  `/r/<prefix>/x/colab/`, owns Host/Origin, cookies, pairing and grants, forwards the
  verified device as `tmt-device-context`, and never forwards the reserved `/.tmt/` subtree
  from a browser. Public short entries bootstrap from validated `tmt-mount` without device context;
  Colab resolves page prefixes from its verified catalog; reader seeds stay in fragments.
  It stores ciphertext, never decodes Yjs; creation opening ignores TTY/JSON.
- **Dependency direction.** `tmt-colab` depends on `tmt-colab-model` (pure codecs and fixed
  crypto), `tmt-extension-state`, `-objects`, `-serve` and `tmt-invoke`/`tmt-cli-style`; the browser
  depends on Remote's served SDK (`/sdk/remote-v1.js`). Never `tmt-core`, `tmt-adapters`,
  `tmt-remote`; core is reached through `$TMT_EXECUTABLE api` and the fixed,
  bounded `identity show --json` command at CLI page create/write and proposal operations.
  The architecture guard enforces this set, that only `tmt-colab` consumes the model and `html5ever`,
  and that only `decoder/child.rs` imports `yrs`.
- **Seams.** With Remote: the mount socket, `tmt-device-context`, the device-events callback
  and the browser SDK; the Ask agent sends through Remote's SDK operations helper as the
  paired owner device, with no native bridge, ledger or migration, and the read-only Agents view
  stays within the [observation boundary](.agents/skills/tmt-colab/references/ask-agent.md).
  After a Remote restart, Colab's public recovery entry reopens the paired device's session once,
  through the same tab claim; all other app assets stay owner-gated. With core: `Product::Colab` registers
  the executable with the installer, and the app is served from `serve --app-dir`, else
  bytes embedded from `TMT_COLAB_APP_DIR`, else the checkout's Vite output.
  Colab owns layout, routing, state words, trusted recovery, archived reads and bounded missing-title folds ([browser contract](extensions/tmt-colab/contracts/colab-v1.md#implemented-mounted-browser-assets-1253)).
  One `tmt colab serve` is enough for a browser: it attaches to a running door, else starts
  one as a supervised child it alone stops.
  This edge (Colab → Remote) uses the public CLI only: no Remote state files and no crate
  dependency. Serves follow verified current releases ([cutoff and door ownership](.agents/skills/tmt-colab/references/architecture-state.md)).
- **Message editing.** The shared rich input and its MessageComposer follow the [editing boundary](.agents/skills/tmt-colab/references/architecture-state.md#message-editing-boundary): the editor grants no dispatch authority, and one Send records one comment and one Ask per distinct visible mention.
- **Renderer invariant.** Parent chrome allows only self-hosted scripts and styles (no
  `unsafe-inline`). Author HTML runs only in `renderer.html` inside an opaque
  `sandbox allow-scripts` frame whose own policy permits inline scripts and styles but no
  network. Everything the frame reports is a cosmetic claim; only parent controls admit
  discussion or sends, and highlights carry only IDs, quote selectors and bounded paint ordinals, never bodies or labels.
  This contains author code; page self-navigation can still leak a request. Theme projection and the cosmetic port are in [page-chrome](.agents/skills/tmt-colab/references/page-chrome.md).
- **Attachments.** [Grammar](extensions/tmt-colab/contracts/attachment-v1.md) and [staging](extensions/tmt-colab/contracts/colab-v1.md#native-attach-command-2291) use crypto, authenticated cuts, fold metadata and Saves.
  The mount-owned object adapter joins generation-scoped callbacks, original uploads, committed reads and detached history; root-local reads need an established channel.
  Remote owns backend/quota/origin; Colab owns crypto/admission. Remote declares Colab Local; snapshot/retained-reference persistence (#2299) remains planned in the [storage proposal](extensions/tmt-colab/contracts/storage-v1-proposal.md).
- **Firestore admission.** Hidden `tmt colab deploy-declaration --json`; the [declaration subsection](extensions/tmt-colab/contracts/colab-v1.md#firestore-deployment-declaration) owns admission and codecs. `object_channel::cloud_sync` publishes rows, authority and originals once Remote activates; hosted folds verify log anchors before checkpoint pairs. Native bootstrap installs a verified checkpoint pair; other verified originals use serve append ([architecture-state](.agents/skills/tmt-colab/references/architecture-state.md)).
- **Plaintext.** Source, shell, discussion reads and export are root-local: only the isolated decoder
  child decodes Yjs, no browser route serves plaintext, and the Worker is resource
  containment, not a security sandbox. Pure [publication codecs](extensions/tmt-colab/contracts/colab-v1.md#content-publication-1908-1928-1934) validate sealed intent; native code atomically stores content or a `kind:"own"` status action with its scoped terminal outcome in the Store. Source/shell actions, thread status and proposals publish through it (offline or the local `page-publish` route), browser Save uses the owner sync socket, signed by the root-local writer.
