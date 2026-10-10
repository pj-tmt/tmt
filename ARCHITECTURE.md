# Architecture

The shipped CLI runtime is the Rust workspace in `rust/`. An optional Office
SPA foundation lives in `extensions/tmt-office/typescript/apps/office`; it is not a CLI fallback or a
shipped connector. The nested `typescript` pnpm workspace owns Vitest, fixture
and release-verification tooling; the repository root has no Node package.
Contributors run Cargo and the nested pnpm scripts directly. The pnpm workspace
is not a second CLI runtime, an npm product, or a source-install fallback. A native source checkout selects
`rust/target/debug/tmt` (or an explicitly supplied native executable); a missing
native build is an error. No test, script, or installer may silently execute an
installed host `tmt` or a retired TypeScript product implementation. Node may
run explicit developer fixtures and verifiers, never serve as a product fallback.

Published releases are immutable. Source changes do not publish replacements
or migrate application data.
TMT remains an invocation-owned local CLI, without a remote MCP server, identity
memory or a separate inbox service. The one MCP server it ships is the hidden
`__channel-server`, a stdio server that an opted-in Claude launch starts as its own
child; it listens on no network port. The independently installed Office companion may
run one explicit loopback-only browser service; it does not execute CLI work or change
the CLI's invocation-owned storage policy.

Any retained `better-sqlite3` use belongs to private developer tooling as an
independent oracle. It is not a Rust runtime dependency or an alternate owner
of native schema and application state.

## Repository layout

Infra reviews the layout map and its machine-readable allowlist,
[`.github/repository-layout.json`](.github/repository-layout.json). Component
ownership comes separately from [`.github/components.json`](.github/components.json).

| Home                      | Responsibility                                                                       |
| ------------------------- | ------------------------------------------------------------------------------------ |
| Repository root           | Entry points, contributor guidance, license and required configuration               |
| `.agents/`                | Contributor skills and their area references                                         |
| `.github/`                | Ownership/layout maps, workflows, shared Actions and isolated release tooling        |
| `rust/`                   | CLI, core, adapters, neutral leaves, private fixtures/release tools and archive note |
| `typescript/`             | Private developer tooling, tests and shared fixtures                                 |
| `extensions/<extension>/` | Product-owned runtimes, contracts, skills, docs and assets                           |
| `contracts/`              | Core public contracts and normative fixtures                                         |
| `scripts/`                | Shared shell/build/development helpers                                               |
| `skills/`                 | Canonical bundled user-agent guidance                                                |
| `site/`                   | Public Home, handbook sources and translations                                       |
| `design/`                 | Tokens/CLI; [private browser-ui](design/gui-components.md); no Core/CLI dep/embed    |

New homes or exceptions need infra review and coordinated map/allowlist changes;
ignored local outputs are outside the tracked-file map. The
[layout skill](.agents/skills/tmt-layout/SKILL.md) owns add/move procedures and the
tracked-file guard. Handbook language exceptions belong to
[AGENTS](AGENTS.md#repository-content-language) and the allowlist.

`tmt-ux-lead` owns shared visual tokens and the private browser leaf.
[Browser component contracts](design/gui-components.md) point to the leaf's generation and consumer boundaries.
Release procedures belong to the
[release skill](.agents/skills/tmt-release/SKILL.md), including the archive's
product-neutral `rust/archive/NATIVE-INSTALL.md`; [dev-only embed](site/README.md) stays site-owned.

## TypeScript workspace boundary

The `typescript` pnpm workspace owns private Node tooling/tests and one lockfile,
with parent-relative Office packages under `extensions/tmt-office/typescript`,
the parked browser-addon shell, Remote client SDK, and Colab browser/client packages.
It is not a second CLI runtime. Packages resolve declared dependencies, never
root-hoisted tooling; browser specs reach the tooling SQLite oracle through
`typescript/test/support`. Rust, root launchers, core contracts and canonical
skills stay outside this boundary; extension contracts stay with their product.

Vite+ supplies one Vitest runner, aliased Vite core and bundled Oxfmt/Oxlint.
Packages retain their separate configs, plugins, formatter selections and warning
policies. `typescript/scripts/format-workspace.mjs` owns root code/docs selections;
`lint-config.mjs` owns shared lint rules. Compiler commands retain their owners;
site and release tooling stay outside this lockfile.

Office is frozen and optional. Its browser runtime imports no local SQLite,
process adapters or native test helpers. Build the SPA before its embedded native
companion, then verify installed-browser behavior; ordinary CLI builds and commands
remain independent. Office reuses canonical identity, room, request and installation
owners; it adds no identity registry, exchange engine or command runner. Core's
[request owner](#sqlite-and-durable-exchanges) retains atomic membership admission,
immutable receipts, retention and uncertain-delivery policy. Native admission and
Rules/trusted issuers enforce authority; browser rendering, structural conformance,
scene placement and untrusted content grant none.

The [Office architecture](extensions/tmt-office/docs/architecture.md) owns app/module,
storage, renderer and resource lifecycles; its
[contract index](extensions/tmt-office/contracts/README.md) owns versioned data,
compatibility and literal vectors, and the
[Office skill](extensions/tmt-office/skills/tmt-office/SKILL.md) owns retained-install
workflows. [Office design](extensions/tmt-office/docs/design.md) and
[workshop references](extensions/tmt-office/docs/references/workshop/README.md)
label proposals and visual intent separately from implemented behavior.

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

Office is frozen and lives outside core: see
[`extensions/tmt-office/docs/architecture.md`](extensions/tmt-office/docs/architecture.md).
Core keeps only the reserved `tmt office` facade (`tmt-cli/src/office_facade.rs`);
the architecture guard rejects Office modules in core crates except that facade.

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
stdio, TTY and signals and its exit status is the command's. `TMT_EXECUTABLE` names
the invoking executable. Core keeps no registry, manifest, daemon or extension
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
nothing. Verified former-product replacement withdraws the former extension's hook consent; consent never transfers to its successor.

### Core command surface

The grammar owns primary names and accepted aliases; core reserves them before
external dispatch. The public surface groups into setup and guidance (`init`,
`config`, `completion`, `help`, `learn`, `install`, `setup`); identities and
bindings (`identity`, `ls`, `add`, `name`/`this`, `marked`, `whoami`, `unbind`,
`rm`, `mv`, `notes path`); foreground launch and channels (`run`, `resume`,
`channel`); profiles and exchange (`role`, `preamble`, `x`, `reply`, `result`,
`inbox`, `answer`, `talk`/`send`, `check`/`read`); rooms and recovery (`room`,
`workspace`); `focus`; local extension interfaces (`api`, `mcp`); native
installation (`upgrade`/`update`, `uninstall`, `extension`, `driver`); and the
frozen `office` facade. Hidden internal commands are not a public surface.
Workspace recovery keeps geometry in Core, snapshot IO and fenced creation in
adapters, and CLI composition/output. Full restore uses ordinary resume and
external dispatch in created panes; layout-only grants no launch authority.
Output uses one typed result for human and JSON projections; `identity show`
without a name uses the shared verified-caller selector and never falls back to
a working directory, active pane or sole identity.

## Domain and state ownership

### Identity, names and bindings

- `tmt-core::names` owns canonical identity classification; pane-target syntax
  belongs to each host. `identity` owns lifetime and storage-only create/promote
  policy, `identity_metadata` and `identity_status` own descriptive, untrusted data
  that grants no authority (including the [atomic metadata contract](contracts/extension-api.md#conditional-identity-metadata)), and `binding` owns evidence evaluation, retirement
  authorization and binding use cases.
- Unknown or conflicting endpoint evidence is never proof of death. Saved
  identities detach and stay offline; temporary identities retire only on
  conclusive evidence or explicit unbind, and exchanges are kept. Presence is
  observation, not routing permission: a marker or socket cannot authorize a
  different identity, and the pane marker proves ownership by IDs, never by name.
- `binding::session` separates identity-owned session preferences from
  binding-owned runtime observations, and observation writes are compare-and-set
  inside the binding transaction. Drivers own process verification, event mapping
  and driver-state persistence; ordinary CLI calls learn direct sessions after output through the same admission/CAS, with bounded silent refusal, storage-only prechecks and private TTL hints; core stores driver state without parsing it.
  Provider end leaves stored readiness Unknown pending a fresh start; only conclusive
  process loss ends the runtime incarnation.
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
  performs caller/target preflight and composes them. Module rules:
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

`tmt-adapters::config::ConfigPaths` is the sole application path and managed-release-only default-directory cutover owner;
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
`request::digest` owns [delivery](contracts/extension-api.md#digest-policy-and-checklist).
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
and one binding policy in `host::driver::{status, send, focus}` decides which
evidence makes a binding present and when input is blocked. Endpoint identity is
opaque to everything but its host: `HostKind` is pure data, evidence from another
host is `Unknown`, and only `tmt-core/src/host.rs`, `tmt-adapters/src/host.rs` and
`tmux/` may spell a built-in host's name. Core's delivery policy rewrites ASCII `!` to
fullwidth `！` in any text typed into a pane. Details are in the
[hosts and drivers reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md).

### Agent drivers

Each agent driver has one pure `DriverDescriptor` and one adapter module;
`drivers::Registry` composes detection, setup, launch and caller recognition.
Detection never starts an agent. Module rules and name ownership:
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
`tmt-adapters::driver_protocol` owns explicit approval and bounded calls, while
`host::external` composes approved hosts. Core decides evidence; unavailable drivers
never prove loss. External runtime launch/setup/hooks remain unwired. Details:
[hosts and drivers](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#driver-protocol-crates).

## Managed skills and native installation

Managed agent guidance is a filesystem concern separate from application state.
`tmt_core::skill_catalog` is the one list of bundled skill names; the bundle is
embedded and materialized by digest under `skill_installation`, and the
architecture test fails on a skill-name list anywhere else. Core install exposes
only `tmt` and `tmt-inbox`. Skill installation never opens configuration,
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
[handoff contract](contracts/native-install-handoff-v1.md); persisted PR channels,
compiled schema export and admission are owned by the [PR channel contract](contracts/native-pr-channel.md). The candidate then lets that CLI
run the consented extension phase; there is no rollback or second installer.
`tmt extension install|upgrade|rm|ls` is the public surface for extensions and
requires consent. Acquisition, receipts, companions, skills trees, repair and the
upgrade handoff are in the
[installer architecture reference](.agents/skills/tmt-core-runtime/references/install-architecture.md);
build, publication and verification procedures are in the
[tmt-release skill](.agents/skills/tmt-release/SKILL.md).

Digest’s unpublished [entry](extensions/tmt-digest/rust/tmt-digest/src/grammar.rs) lives in `extensions/tmt-digest`.

## Ops extension

`extensions/tmt-ops/rust/tmt-ops` builds optional `tmt-ops`, reached through
external dispatch as `tmt ops`; `ui`, `hotkeys`, `skill` and `playbook` are Ops commands,
while member and state commands live under `tmt ops squad` (alias `sq`).
Independent releases use `tmt-ops-v<version>`; module/drawing ownership and guard verification: [Ops](.agents/skills/tmt-ops-dev/SKILL.md) and [TUI](.agents/skills/tmt-tui/SKILL.md) developer skills.

- **Seam.** Ops uses public `tmt --json`/`tmt api` and starts `tmt digest tick`
  via `TMT_EXECUTABLE` (else PATH `tmt`), bounded by `tmt-invoke`. Digest owns
  locking, deadlines and delivery ([clock](.agents/skills/tmt-ops-dev/references/data-and-state.md#cron)). No direct core state, tmux or provider writes.
  The guard rejects Cargo/source edges from TMT crates to Ops or Ops to core.
  Its TMT dependencies are `tmt-tui`, `tmt-cli-style` and `tmt-invoke`,
  plus dev-only `tmt-test-support`.
  New `tmt api` methods need tmt-lead review.
- **Data ownership.** A squad is the core room `squad-<name>`; member fields are
  identity metadata `squad.<name>.<field>`, with no Ops membership store. Ops owns
  `<dataRoot>/ops` (`storage.root`), including private, bounded display snapshots in `cache/board` ([cache contract](.agents/skills/tmt-ops-dev/references/refresh-and-meter.md#display-snapshot-cache)), and disposable `$XDG_CACHE_HOME/tmt-ops` caches.
  `ops.toml` is the user's file: agents never write it; Ops uses its compare-and-set
  writer. `migration` owns the locked, byte-preserving legacy cutover and `board_switch` the
  consented switch of former boards ([config reference](.agents/skills/tmt-ops-dev/references/config-and-effects.md)); no startup path runs the switch. No Ops data goes into `config.json` or the core database.
- **Checklist.** `checklist_command` exposes the native grammar over the existing `checklist`
  caller/room admission and the versioned room-UUID JSON under `<dataRoot>/ops/checklist`.
  Reads create no checklist files, a failed prepublication preserves bytes, and uncertainty stays
  Unknown after readback; the board controller consumes the same typed service
  ([data and state reference](.agents/skills/tmt-ops-dev/references/data-and-state.md#checklist-storage-and-service)).
- **Row detail, digest, timing and entry.** Shared detail ownership and Core-owned digest policy live in the [Ops skill](.agents/skills/tmt-ops-dev/SKILL.md); the [board reference](.agents/skills/tmt-ops-dev/references/board.md) owns worker fences; the [refresh reference](.agents/skills/tmt-ops-dev/references/refresh-and-meter.md#load-timing-trace) owns timing fields; the [config reference](.agents/skills/tmt-ops-dev/references/config-and-effects.md#cli-entry-and-public-json) owns CLI entry and display-document contracts.
- **Contracts.** The [embedded lead skill](extensions/tmt-ops/skills/tmt-ops/SKILL.md) owns the public JSON
  shapes of `ops sq ls`, `config`, `checklist` and `cron`; `typescript/test/native/ops.test.ts` checks them.
  A change to row JSON updates the lead skill and runs the native row-shape test in the same PR.

## Testing and evidence boundaries

Rust tests stay beside their owners. TypeScript native, E2E, tooling and stress
suites share `test/support`, which imports no suite; native and E2E import neither
each other nor tooling. Scenarios retain assertions; helpers own fixture
mechanics. Frozen inputs and independent SQL/schema oracles must not derive
expected results from the implementation under test.

Tests select an explicit task-owned native executable or the checkout build;
a missing build fails, with no host CLI or retired-runtime fallback. State,
provider roots, prefixes, sockets and processes remain fixture-owned. Native
process fixtures isolate caller ancestry and host tmux discovery; Docker supplies
network-isolated private tmux and deterministic peers. No host tmux server,
provider installation or global environment mutation is test evidence.

Cleanup confirms owned child/group absence before deleting fixture state;
unknown inspection, leaks and false positives fail. Signals target only verified
owned processes. Runtime tests prove CLI behavior, Docker proves transport and
lifecycle, and release tooling proves actual archives and public installation;
one layer's success cannot substitute for another's evidence.

Helper ownership, fixture publication and lifecycle details live in the
[E2E references](.agents/skills/tmt-e2e/references/test-boundaries.md); shared
commands remain in [DEVELOPMENT](DEVELOPMENT.md#native-process-and-shared-tests).

## Release boundary

Release tooling uses cargo-dist manifests and product-owned archives, sharing native
runtime/linkage proof across archive, installer, upgrade and public smoke checks.
Raw executables prove neither archives nor public installation. Candidate handoff:
[native-install-handoff-v1](contracts/native-install-handoff-v1.md). [Records](contracts/release-index-v1.md) bind verified release bytes to identity.
Archive, installer, verifier, publication, compiled CLI schema and the PR release-candidate producer/cleanup sources belong to
[tmt-release](.agents/skills/tmt-release/SKILL.md).

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
Retired identities stay historical. Rename staging binds two published CLI drivers by registration
ancestry; one verifier checks replacement state and skills after digest readback. Drafts never advance published evidence. Tags become
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

`rust/crates/tmt-extension-state` is a library-only, unpublished filesystem leaf
owned by the Remote component. Only the Remote and Colab executables consume it;
its sole dependency is the existing `nix` pin, with no TMT, crypto or storage
crate dependency. Core, adapters and the Colab model do not consume it. The
architecture guard enforces the reviewed manifest, source edges and all dependency
kinds, including aliases and target-specific dependencies.

`Layout` admits an extension-selected private subtree beneath the injected
absolute core-reported data root. It preserves existing root permissions and
canonicalizes aliases only in that trusted root. Private directories must be
owned 0700 directories; allowlisted files must be owned regular 0600 files,
opened with no-follow and nonblocking flags. Read-only lookup and lock probes
create nothing. Reads retain the caller's byte bound.

`Publication` holds a nonblocking lock through stale temporary admission,
cleanup and publication. Only the selected prefix plus 32 lowercase hex digits
matches a temporary; unsafe or oversized matches refuse and foreign names remain.
A staged file borrows that guard and links create-only after writing and syncing
its bytes. The extension retains entropy, key interpretation, error mapping and
its existing staging/removal/directory-sync failure ordering. Store schemas,
identity derivation and Remote's serve-lock proof remain extension-owned; this
leaf neither discovers roots nor accesses core state or provider configuration.

## Remote extension pilot

`extensions/tmt-remote` is a separate executable run as `tmt remote`. It reaches core only
through the public process/JSON API (fixed `api`, `list --json`, `identity list --json` and
`check <name> --json` subprocesses of the supplied absolute `TMT_EXECUTABLE`, run by
`tmt-invoke`) and owns the private `<dataRoot>/remote/` subtree through the
[shared extension state layout](#shared-extension-state-layout). Core never owns a listener or
Remote state and only registers Remote as an installable product; its archive embeds the static
browser pages, checked shared CSS plus host styles, SDK and wordlist, with no companions or
skills. Publication gates belong to the [release skill](.agents/skills/tmt-release/SKILL.md).

[`contracts/remote-channel-v1.md`](contracts/remote-channel-v1.md) owns the wire, pairing,
sessions (reattach, caps, held work), operations, settings authority, mounts, serve lifecycle,
discovery shapes, and the [hard lines and standing guarantees](contracts/remote-channel-v1.md#user-path)
that open it. Remote owns authority; shared components supply presentation only. The door serves
the browser SDK `remote-v1.js` (built from `remote-client`, whose README owns re-admission
rules) and mounts owner-installed extensions. [Colab](#colab-extension) is mounted at
`/r/<prefix>/x/colab/` and otherwise reaches Remote only through the public CLI, that SDK and
the extension object channel (`tmt-extension-objects`), never through Remote state files.

Remote's lease-bound object service owns `objects` and the `rust/crates/tmt-extension-objects`
wire leaf; [object-backends](.agents/skills/tmt-remote/references/object-backends.md) owns
channels, backends and quotas. Pure declaration, Rules and Hosting composition feeds the
Firestore deploy CLI. [Remote internals](.agents/skills/tmt-remote/references/architecture-internals.md)
own the module table and per-module guarantees;
[door and discovery](.agents/skills/tmt-remote/references/door-and-discovery.md) owns serve
lifecycle, status and management implementation; the background start it shares with Colab
is [extension-serve-v1](contracts/extension-serve-v1.md).

## Colab extension

Colab (`extensions/tmt-colab/`: `tmt-colab`, `tmt-colab-model`, `@tmt/colab-client`,
`@tmt/colab-app`) is a native extension embedding its app and skill.
The opt-in installer uses its archived `skills/`; `tmt colab skill` reads embedded
bytes without core and storage access.
[colab-v1's hosted bundle](extensions/tmt-colab/contracts/colab-v1.md#tmt-colab-hosting-bundle---json) owns the hosting build.
[colab-v1](extensions/tmt-colab/contracts/colab-v1.md) is the normative contract; the
[tmt-colab skill](.agents/skills/tmt-colab/SKILL.md) holds module knowledge and procedures.

- **Layer.** Colab is an app on Remote with no door of its own. `tmt-colab serve`
  listens only on the owner-only socket `<dataRoot>/colab/door.sock`. Remote mounts it at
  `/r/<prefix>/x/colab/`, owns Host/Origin, cookies, pairing and grants, forwards the
  verified device as `tmt-device-context`, and never forwards the reserved `/.tmt/` subtree
  from a browser. Remote forwards public short entries without device context; Colab redirects only
  to validated `tmt-mount`, resolving page prefixes from its verified catalog (ambiguity by parent chrome).
  It stores ciphertext, never decodes Yjs; creation opening ignores TTY/JSON.
- **Dependency direction.** `tmt-colab` depends on `tmt-colab-model` (pure codecs and fixed
  crypto), `tmt-extension-state`, `-objects`, `-serve` and `tmt-invoke`/`tmt-cli-style`; the browser
  depends on Remote's served SDK (`/sdk/remote-v1.js`). Never `tmt-core`, `tmt-adapters`,
  `tmt-remote` or Office; core is reached through `$TMT_EXECUTABLE api` and the fixed,
  bounded `identity show --json` command at CLI page create/write and proposal operations. Its optional caller
  name is publisher-asserted display metadata, never a creator binding or routing authority.
  The architecture guard enforces the dependency set, that only `tmt-colab` consumes the model,
  and that only `decoder/child.rs` imports `yrs`.
  [Colab proposals](.agents/skills/tmt-colab/references/discussion.md) defines provenance, retained-ID recovery and trusted card ownership.
- **Seams.** With Remote: the mount socket, `tmt-device-context`, the device-events callback
  and the browser SDK; the Ask agent sends through Remote's SDK operations helper as the
  paired owner device, with no native bridge, ledger or migration. The read-only Agents view
  reuses that current context and directory owner without admitting Ask destinations or
  invoking session recovery; [Ask modules](.agents/skills/tmt-colab/references/ask-agent.md)
  define the observation boundary. After a Remote restart,
  Colab's public recovery entry reopens the paired device's session once, through the same
  tab claim; all other app assets stay owner-gated. With core: `Product::Colab` registers
  the executable with the installer, and the app is served from `serve --app-dir`, else
  bytes embedded from `TMT_COLAB_APP_DIR`, else the checkout's Vite output.
  Shared chrome serves native `/assets/chrome.css` even without an app build; Colab owns layout, routing, state words and trusted action/recovery. See the [browser contract](extensions/tmt-colab/contracts/colab-v1.md#implemented-mounted-browser-assets-1253).
  One `tmt colab serve` is enough for a browser: it attaches to a running door through
  `tmt remote status --json`, else starts `tmt remote serve --json` as a supervised child
  in its own process group, reading pairing from `tmt remote devices --json`. This optional
  edge (Colab → Remote) uses the public CLI only: no Remote state files and no crate
  dependency. Colab stops only a door it started, with its whole group, after closing its own
  socket. `tmt colab stop` reaches the serving process through a root-local route on that same owner-only socket (no signals, no new surface).
- **Message editing.** The Lexical 0.52.0 plaintext/history composer follows the [editing boundary](.agents/skills/tmt-colab/references/architecture-state.md#message-editing-boundary). The parent records one comment and an Ask per distinct visible mention (up to 8); Ask again keeps the comment and uses a fresh operation after pair-marker/own-ledger checks. The editor grants no dispatch authority.
- **Renderer invariant.** Parent chrome allows only self-hosted scripts and styles (no
  `unsafe-inline`). Author HTML runs only in `renderer.html` inside an opaque
  `sandbox allow-scripts` frame whose own policy permits inline scripts and styles but no
  network. Bound selections/rectangles, height/anchor-offset reports, quote-selector
  highlights and known-thread marker clicks are cosmetic untrusted claims; only parent
  controls admit discussion or sends. The bootstrap installs bounded DOM resolution before
  author HTML and passes no application capability. Parent highlight messages carry only
  anchor IDs and quote selectors; discussion bodies and display labels never enter author code.
  This contains author code; page self-navigation can still leak a request. The parent projects the effective light/dark theme as root `data-theme` over the render-bound cosmetic port; details in [page-chrome](.agents/skills/tmt-colab/references/page-chrome.md).
- **Attachments.** Colab implements [descriptor/manifest/reference grammar and internal read/publication capture](extensions/tmt-colab/contracts/attachment-v1.md) with existing crypto, authenticated cuts and fold metadata.
  The mount-owned object adapter joins generation-scoped callbacks, original uploads, committed reads and detached history; root-local reads need an established channel.
  Remote owns backend/quota/origin; Colab owns crypto/admission. Remote declares Colab Local; snapshot/retained-reference persistence (#2299) remains planned in the [storage proposal](extensions/tmt-colab/contracts/storage-v1-proposal.md).
- **Firestore declaration.** Hidden `tmt colab deploy-declaration --json`; the [declaration subsection](extensions/tmt-colab/contracts/colab-v1.md#firestore-deployment-declaration) owns it.
- **Plaintext invariant.** Page source, discussion reads and export are root-local: only the isolated decoder
  child decodes Yjs, no browser route serves plaintext, and the browser Worker is resource
  containment, not a security sandbox. Private causal preparation returns deltas; pure [publication codecs](extensions/tmt-colab/contracts/colab-v1.md#content-publication-1908-1928-1934) validate sealed intent. The native library prepares a frozen signed packet and chain from one authenticated snapshot, then atomically retains content (or, as `kind:"own"`, a status action) with its scoped terminal outcome in the Store; `tmt colab page write`, `threads resolve|reopen` and proposal record/placement operations publish through it (offline or the local `page-publish` route), and the browser Save does over the owner sync socket (colab-v1 Browser Save), signed by the root-local writer.
