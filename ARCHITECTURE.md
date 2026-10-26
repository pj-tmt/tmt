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
| `.claude/`                | Claude Code project settings: the host-tmux guard hook                               |
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
owns moves and the tracked-file guard.

`tmt-gui-lead` owns shared visual tokens and [private browser UI](design/browser-ui/README.md).
`design/browser-ui` splits React entries (`./ui`, `./react`) from framework-free DOM entries (`./static`, `./shell`, `./chart`, `./menu-host`) and an import-free `./search` registry; hosts own domain state and retain commands and closure admission. The [README](design/browser-ui/README.md) owns modality and header-action rules.
[Release procedures](.agents/skills/tmt-release/SKILL.md) include the archive’s
product-neutral `rust/archive/NATIVE-INSTALL.md`; [dev-only embed](site/README.md) stays site-owned.

## Public blog boundary

`site/src/blog/` builds independently approved `site/src/posts/` language variants into the shared site artifact.
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

Project-only never-shipped path declarations and their CI-only admission guard are defined in the
[release-tracking reference](.agents/skills/tmt-release/references/native-release.md#project-release-tracking).

Selected missing, failed, cancelled or unexpectedly skipped work and empty discovery
cannot satisfy required gates; selection, worker, cache and advisory-browser details live in the
[CI reference](.agents/skills/tmt-release/references/ci-selection.md) and PR-title gates in the
[release reference](.agents/skills/tmt-release/references/native-release.md#conventional-pr-titles).
Publication reuses the native aggregate's scope-skip proof with check-suite provenance, not
recomputed selection or bare skips.

## Runtime layers

[Orchestration](contracts/orchestration-v1.md): Core `effective()`; adapter calendar/SQLite; CLI/API reads/writes; Digest receiving. Restricted holds record one pending marker per identity; the shared wake clock and tagged checklists own summary admission/settlement. [Module rules](.agents/skills/tmt-core-runtime/SKILL.md).

| Layer             | Owner                           | Responsibility                                                                                                                                                                                                  |
| ----------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure domain       | `rust/crates/tmt-core/src/`     | Identity, names, bindings, profiles, settings, retention, request state and native-install version policy. No filesystem, process, SQLite, tmux, network or CLI framework.                                      |
| Concrete adapters | `rust/crates/tmt-adapters/src/` | Config files, SQLite, bounded files and processes, signals, tmux evidence/transport, response input, HTTP acquisition, native release publication and managed skill files.                                      |
| Application/CLI   | `rust/crates/tmt-cli/src/`      | Core grammar, typed invocations, preflight and use-case composition, completion and the executable entry point. It chooses adapters; it does not duplicate their storage, file, installation or process policy. |

`tmt-command-output` owns shared command output/errors and formatting.
Human text uses `tmt-cli-style` for [CLI style](design/cli-style.md), `grid`,
help, `Interaction` and `progress`. This presentation leaf has no TMT
dependencies; the architecture guard enforces that boundary.

`rust/crates/tmt-tui` is an internal, unpublished, application-neutral presentation
leaf for TMT terminal UIs (markup admission, one Taffy geometry computation, Ratatui
paint, reusable components). It has no application vocabulary, acquires no terminal, clock,
settings or provider data, and takes its tokens from `tmt-cli-style` roles. The
[dependency guard](rust/crates/tmt-cli/tests/architecture/policy.rs) owns its allowlist;
core, adapters, CLI and extension behavior are excluded. Ops is its sole reviewed consumer;
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
in help or completion. [Argument parsing](CONVENTIONS.md#commands-effects-and-contracts) stays with typed parsers.
Human/JSON share typed result/status contracts; [list](contracts/identity-status-v1.md#list-runtime-driver) owns runtime-driver proof.
Command dispatch, help and completion rules are in the
[extension surface reference](.agents/skills/tmt-core-runtime/references/extension-surface.md).

### External command contract (v1)

An unknown root command named `[a-z0-9][a-z0-9-]*` resolves `tmt-<name>` on PATH;
core wins every collision. On Unix the adapter `exec`s, so the extension inherits
stdio, TTY and signals and its exit status is the command's. `core_executable` hands
companions the managed stable launcher via `TMT_EXECUTABLE`, else the invoking executable.
Only the tail is forwarded; missing official names reuse native installation.
Completion v1 is optional, bounded and falls back to file completion on any failure.

### Local extension API (v1)

`tmt api` is the public, same-user process port for machine-shaped gaps in the
ordinary CLI: one versioned JSON request on stdin, one JSON resource or error on
stdout. Explicit `--session` uses ordered JSON lines and a lazy resource connection;
workspace capture retains its bounded read-only snapshot.
Request admission stays fresh and a schema change ends it with a reopen code.
`tmt-adapters::api` owns composition/storage; shared `framed_stdio` bounds API/MCP
frames. The CLI owns publication, request boundaries and cleanup, with no listener
or daemon. Protocol major 1 accepts additive operations and fields; incompatible
changes need a new major.
[Optional extension uses](contracts/extension-api.md#optional-cross-extension-uses) follow the receipt-only query contract.
Human-shaped operations remain their
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

The [grammar](rust/crates/tmt-cli/src/grammar.rs) owns primary names, aliases and
the public help surface; core reserves its names before external dispatch.
Hidden internal commands are not a public surface.
Core owns [workspace geometry/codecs](contracts/workspace-v1.md#restore-authority) and generations; adapters, atomic publication/fenced
creation; CLI, consent/output. Restore pins bytes; full restore uses ordinary
resume/external dispatch in created panes. Layout-only grants no launch authority.
One typed result serves human and JSON output; `identity show`
without a name uses the shared verified-caller selector and never falls back to
a working directory, active pane or sole identity.
`check --capture-only` shares UUID binding selection and `identity_context`'s
expected-name guard with pinned identity reads, before host capture; ordinary
check keeps name/pane routing and its existing Digest opportunity.

## Domain and state ownership

### Identity, names and bindings

- `tmt-core::names` owns canonical names; hosts own pane-target syntax.
  `identity` owns canonical UUID spelling and lifetime/create/promote; `identity_metadata`/`identity_status`
  own untrusted descriptions, never authority ([atomic metadata](contracts/extension-api.md#conditional-identity-metadata)).
  `binding` owns evidence, retirement, binds/moves and CAS rename.
  [Discovery](contracts/discovery-v1.md) owns candidate bounds and native/UUID fences for tmux and admitted host drivers; guarded `add` uses `binding`.
- Unknown or conflicting endpoint evidence is never proof of death. Saved
  identities detach and stay offline; temporary identities retire only on
  conclusive evidence or explicit unbind, and exchanges are kept. Presence is
  observation, not routing permission: a marker or socket cannot authorize a
  different identity, and the pane marker proves ownership by IDs, never by name.
- `binding::session` separates per-driver identity resume records from the one binding/runtime,
  with transactional CAS. [Switching](contracts/driver-protocol-v1.md#built-in-driver-switch)
  hands off once and uses the existing stop/move and caller-owned launch paths.
  Drivers verify processes, map events and own state; core never parses it. CLI learns direct sessions after output via this admission/CAS,
  with bounded silent refusal, storage-only prechecks and private TTL hints.
  Provider end leaves readiness Unknown until fresh start; only proven process loss
  ends the incarnation. Claude approval notifications set waiting;
  only admitted events clear it, never a timer.
- Presence reads acquire host evidence outside the database writer lock and
  recheck their captured records before reconciliation. Changed records never
  authorize retirement or detachment; unchanged records retain conclusive stale
  binding cleanup.
- Lifecycle hooks complete existing bindings, never create or move identities.
  Binding-owned starts and launch learning are diagnostic; [projections](contracts/identity-status-v1.md#session-binding-observation) read stored facts.
  Bounded hooks exit zero; launch hooks keep externalized-name compatibility. Only consented
  setup publication (including interactively approved upgrade scopes) and recorded refresh change provider settings.
- `tmt-core::endpoint::ProcessIncarnation` (PID plus core's own start token) is the
  one value for comparing local processes.
- Concrete caller/target and storage rules:
  [identity and bindings](.agents/skills/tmt-core-runtime/references/identity-bindings.md)
  and [workspace recovery](.agents/skills/tmt-core-runtime/references/workspace.md).

### Saved identity notes

`NotesIdentityId` (saved identity's canonical UUIDv4) bounds notebook storage;
display names never become paths. `ConfigPaths` owns layout; only
`tmt-adapters::notes` creates `<global_dir>/notes/<identity-uuid>/notes.md`,
owner-only and no-follow. Bodies, concurrency and retention are user-filesystem
concerns: no SQLite copy, lock, size policy or secure deletion. Retirement
preserves notebooks.
[Compaction reminders](.agents/skills/tmt-core-runtime/references/identity-bindings.md#hooks-setup-and-uninstall) follow the provider-hook owner.

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
board-relevant core tables advance the durable change cursor through triggers that a
test requires new tables and columns to extend. Typed not-writable storage failures
are classified once and projected through `tmt-command-output::Failure::storage_access`.

`tmt-core::request::RequestService` owns preparation, delivery-state transitions,
exact final submission, waiter release, attention revisions and retention
housekeeping. [Delivery/history](contracts/extension-api.md#digest-policy-and-checklist),
[message state](contracts/request-response-v1.md#message-state) and
[content-free aggregates](contracts/request-response-v1.md#content-free-exchange-flow)
have one Core owner each; the [storage and requests reference](.agents/skills/tmt-core-runtime/references/requests-storage.md)
maps their modules. A [passthrough](contracts/request-response-v1.md#passthrough) is
a person's typed text, not a request; it settles that agent's oldest open request
and earns its turn's final. CLI and API share admission. Capture admits the full runtime registry before Digest;
[batches](contracts/extension-api.md#ordered-captured-conversation) order inputs and finals, granting no runtime authority.
Adapters admit idle handoff; extensions schedule. Talk and Digest share one item
frame; checklist claims seal whole-item prefixes within 32 KiB (live paste may
carry one oversized first item; hooks leave it held). Clocks sample at transaction entry;
no transaction spans transport, and uncertainty never authorizes replay. Finals are
immutable and terminal text never proves completion. Reads never acknowledge; originator and recipient
acknowledgment are independent. `RequestRoute` separates unbound pane delivery from
the durable identity inbox, which settles `queued`. Unbound identity delivery uses
the foreground observer and recipient pull. Reply notice windows are persisted;
detached workers are finite. The [request contract](contracts/request-response-v1.md)
owns behavior and limits.

### Tmux and process effects

`tmt-adapters::process` owns bounded child execution and byte observations (stream caps,
monotonic deadlines, group cleanup, reaping and invocation-owned SIGINT forwarding) and
native-first process-incarnation acquisition. A leaf probe such as `ps` never forks and
stays in its caller's process group, so a supervisor's group kill also ends it;
`process::interactive` owns direct terminal children without taking the shared process
group. `tmux` pins caller or recorded
sockets, with bounded budgets and no ambient fallback; a failed paste
or Enter is uncertain and never retried as unsent; [pane cosmetics](.agents/skills/tmt-core-runtime/references/hosts-drivers.md) own only pane-local badge overrides.

The CLI and the `delivery` and `pane_badge` adapters reach a terminal only through
`tmt-adapters::host::Host`; extensions never do (they read `tmt ls --json` and
`tmt whoami`), and the architecture guard rejects extension code that names the host
port, the tmux module or core's `binding`, `endpoint` or `host` model. Each built-in or external host implements `host::driver::HostDriver`,
and one binding policy in `host::driver::{status, send, focus, swap_into}` decides which
evidence makes a binding present and when input is blocked. A host with agent
names (Herdr) shows the identity's name as the agent's; the host's name is never
evidence and never copied back. Endpoint identity is
opaque to everything but its host: `HostKind` is pure data, evidence from another
host is `Unknown`, and only `tmt-core/src/host.rs`, `tmt-adapters/src/host.rs` and
`tmux/` may spell a built-in host's name. Core's delivery policy rewrites ASCII `!` to
fullwidth `！` in typed text and the ESC of a bracketed-paste marker to `␛`. The built-in host uses this seam in process.
`tmt-adapters::pty` owns per-member supervisors, crash residue cleanup under nonblocking
flocks, current screens and live attach; CLI restores terminal modes.
[Terminal stream v1](contracts/terminal-stream-v1.md) and the
[host reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#built-in-host) own snapshots and lifecycle.

### Agent drivers

`DriverDescriptor` and adapters feed `drivers::Registry` for detection, setup,
launch, caller recognition, foreground argv, allowance reads and typed refusals.
Each descriptor sets `full` (hooks/runtime) or `basic` (detection/skills), published
as `driverSupport` by `tmt ls`. Detection never starts agents. Launch adds no plugin
arguments or persistent provider settings. Module rules and name ownership:
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
`tmt-adapters::driver_protocol` owns explicit approval, automatic approval of the
shipped driver (`registry::approve_shipped`) and bounded calls; `host::external`
composes approved hosts. Core decides evidence; unavailable drivers
never prove loss. External runtime launch/setup/hooks remain unwired. Details:
[hosts and drivers](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#driver-protocol-crates).

## Managed skills and native installation

Managed agent guidance is a filesystem concern separate from application state.
`tmt_core::skill_catalog` is the one list of bundled skill names; the bundle is
embedded and materialized by digest under `skill_installation`, and the
architecture test rejects duplicate multi-name lists in production Rust. Core install exposes
only `tmt` and `tmt-inbox`.
[Retired bundle verification and preservation](.agents/skills/tmt-core-runtime/references/install-architecture.md#managed-skills) follow the installer owner;
explicit generic owner removal retains its existing authority, and explicit `--purge` retains its existing semantics.
Refresh and default uninstall preserve retired links, owner records and sources.
Skill installation never opens configuration,
SQLite or tmux.
[Catalog publication](.agents/skills/tmt-core-runtime/references/install-architecture.md#managed-skills) follows the installer owner.
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
run the consented extension phase; there is no rollback or second installer. Upgrade shares setup's missing inventory and provider publisher; only interactive approval can add a provider scope.
`install_guidance` projects setup's verified inventory as public
[`guidance.actions`](contracts/extension-api.md#installation-guidance-through-the-ordinary-cli);
PATH only resolves commands.
Receipt-frozen [CLI minima](contracts/extension-api.md#optional-cross-extension-uses) gate activation; listing compares locally.
[Installer architecture](.agents/skills/tmt-core-runtime/references/install-architecture.md) owns retention,
discovery, receipts, skills, repair and handoff; [release procedures](.agents/skills/tmt-release/SKILL.md) own verification and publication.

Digest owns [history, pricing and valuation](extensions/tmt-digest/contracts/digest-v1.md); tick collects, status reads.
A `user`-marked identity is never held, nor anyone while the [global switch](extensions/tmt-digest/contracts/digest-v1.md#global-switch) is off. An `auto` member is held like an interval one: the tick renews the hold and offers [auto delivery](extensions/tmt-digest/contracts/digest-tick-v1.md#auto-delivery); Core and the turn end deliver.
Prices are one [public file](extensions/tmt-digest/contracts/digest-v1.md#pricing-file).
The [usage report](extensions/tmt-digest/contracts/usage-v1.md) joins Core's rollup, limits and `x flow --bucket` per run; it stores nothing.

## Ops extension

`extensions/tmt-ops/rust/tmt-ops` builds optional `tmt-ops`, dispatched as `tmt ops`.
`ui`, `hotkeys`, `skill` and `playbook` are Ops commands; member/state commands
use `squad` (alias `sq`). Releases use `tmt-ops-v<version>`.
[Ops](.agents/skills/tmt-ops-dev/SKILL.md) and [TUI](.agents/skills/tmt-tui/SKILL.md)
own module/drawing boundaries and verification.

- **Seam.** Ops uses public JSON/API (minute `workspace.capture`), `tmt digest tick` and
  [`host templates`](contracts/driver-protocol-v1.md#launch-templates) via
  `TMT_EXECUTABLE` (else PATH), bounded by `tmt-invoke`. `board/host_launch`
  prepares/rechecks frozen argv for New member (new UUID before join)
  and Switch tool (same UUID, requested running driver). Core owns
  handoff/stop; Ops never writes private Core/host/provider state. Digest owns
  locks/deadlines/delivery. No Core/Ops Cargo/source edges. tmt-lead reviews new APIs.
  Leaves: `tmt-tui`, `tmt-cli-style`, `tmt-invoke`, `tmt-extension-state`,
  `tmt-pty-protocol` (a built-in host's terminal-stream frames); dev-only `tmt-test-support`.
- **Data.** Squads are Core rooms `squad-<name>`; fields are identity metadata
  `squad.<name>.<field>`, with no Ops membership store. Ops owns `<dataRoot>/ops`
  (`storage.root`) and disposable `$XDG_CACHE_HOME/tmt-ops` caches. Retired
  `cache/board` files stay untouched. `ops.toml` is user-owned: agents never write it;
  Ops uses CAS; cron/checklists live in
  [`ops.db`](.agents/skills/tmt-ops-dev/references/data-and-state.md#ops-state-database).
  `migration` owns locked byte-preserving cutover; CLI warns once, UI paints notices through App.
  No Ops data enters `config.json` or Core's database.
- **Workspace.** Agents/Digest share one acquired roster; the flat Agents list caches canonical all-agent projections, while tree projections are frame-scoped.
  UUID squad anchors and virtual Common pool define selection; paint/input do no Core reads. WP1 owns conversation/editor/drafts.
  `board::unread` and `board::selected_conversation` project Core answers off-thread and acknowledge only through public `requests.ack`; `board::buttons` owns chip paint and hover. Lifecycles and module rules: [Ops references](.agents/skills/tmt-ops-dev/references/refresh-and-meter.md#refresh-worker).
  The quiet cache lane owns retained per-name view state, kept apart from shared Config preferences: [View-state ownership](.agents/skills/tmt-ops-dev/references/board.md#workspace-and-retained-identity).
  The shell replaces tab/factory/switcher navigation and HOME cursors.
  Savings and usage figures come from `labels::Reader`/`labels::digest` (supplier estimates, never provider history) and `usage_report::Reader`; `usage_report::valuation` is the one reader of TOK and input price, for the Digest table and the Agents TOK figures alike. [Ops acquisition](.agents/skills/tmt-ops-dev/references/refresh-and-meter.md) owns ranges and lifecycles.
  Member references resolve through the acquired global identity inventory and grant no delivery authority or paint/input I/O ([reference ownership](.agents/skills/tmt-ops-dev/references/board-notebooks.md)).
  Membership removal pauses jobs before leave; only explicit Resume clears pause. Rename CAS keeps the room UUID, then moves Ops metadata/config and cron namespaces under owned locks; partial results
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
  versioned room-UUID JSON in `ops.db`, with a mutation log committed in the same transaction. Reads create no files;
  failed prepublication preserves bytes; uncertainty remains Unknown after
  readback. The board consumes the same typed service.
- **Contracts.** The [embedded skill](extensions/tmt-ops/skills/tmt-ops/SKILL.md)
  owns `sq ls`, `config`, `checklist` and `cron` JSON; native `ops.test.ts` verifies
  them. Row JSON changes update that skill and its native regression in the same
  PR. The Ops developer references own worker fences, cache/timing fields,
  Core-owned Digest policy and independent CLI display projections.

## Testing and evidence boundaries

Rust tests stay owner-local. TypeScript suites share `test/support`, which imports
no suite. Native/E2E import neither each other nor tooling. Scenarios own assertions;
support owns fixtures and scripted loopback models, never provider launch.
Frozen inputs and independent SQL/schema oracles cannot derive expectations from
the implementation under test.

State, provider roots, prefixes, sockets and processes remain fixture-owned.
Adapters' default-off `test-isolation` feature admits private discovery paths in consumer test builds.
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

Build-time version injection belongs to the [main-cut reference](.agents/skills/tmt-release/references/main-cuts.md#cut-and-source-tooling).

Notes, migration comparison and breaking authorization share the newest published
ancestor. Before own publication, a predecessor supplies history; both lines reserve versions.
Retired identities stay historical. Drafts never advance published evidence. Tags become
immutable after all gates pass; only the CLI converges latest to its highest publication.

The [release skill](.agents/skills/tmt-release/SKILL.md#main-cut-authorization) owns publication authorization and product activation; the [main-cut reference](.agents/skills/tmt-release/references/main-cuts.md#publication-gates-and-recovery) owns gates and recovery.

[Release reconciliation](.agents/skills/tmt-release/SKILL.md#project-release-reconciliation) separates delivery from publication; [DEVELOPMENT](DEVELOPMENT.md#project-tracking) owns issue lifecycle.

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
It owns SQLite/atomic document updates and fail-fast guards; newer state is refused,
and admission preserves live locks.
Remote, Colab, Ops, adapters' config/hook consent and Digest history consume it; no TMT/crypto.
Core and Colab's model cannot consume it. Guards check reviewed
manifest/source edges across dependency kinds, aliases and targets.

[Crate docs](rust/crates/tmt-extension-state/src/lib.rs) own admission and publication mechanics.
Schemas, identity derivation and Remote's serve-lock proof remain extension-owned;
the leaf discovers no roots and interprets no Core state or provider configuration.

## Remote extension

`extensions/tmt-remote` owns the listener, restart/[lifecycle](contracts/remote-channel-v1.md#local-cli-discovery),
`<dataRoot>/remote/` ([state leaf](#shared-extension-state-layout)), the door and
[SDK](extensions/tmt-remote/typescript/remote-client/README.md). Owner-installed
[Colab](#colab-extension) mounts at `/r/<prefix>/x/colab/`.

Remote uses only public Core JSON via absolute `TMT_EXECUTABLE`/`tmt-invoke`; agent checks use capture-only and UUID-pinned name queries.
Core never uses Remote trust. Other extensions use public CLI/SDK/`tmt-extension-objects`,
never Remote state files. Remote owns member/channel authority, roster, revision-fenced local management and invitations; UI grants none;
Firestore uses Auth/Rules only. Read-only auto-pair; no companions/skills.
Emulation never ships.

`talk --remote` → [message v1](contracts/remote-message-v1.md) → serving owner,
with scope/allowlist/hold and UUID/name checks and same-ID Core dispatch/recovery.
Finals use receipt-bound Core `reply`.

[Contract](contracts/remote-channel-v1.md); [hard lines](contracts/remote-channel-v1.md#user-path);
[internals](.agents/skills/tmt-remote/references/architecture-internals.md);
[door/discovery](.agents/skills/tmt-remote/references/door-and-discovery.md);
[object backends](.agents/skills/tmt-remote/references/object-backends.md);
[release gates](.agents/skills/tmt-release/SKILL.md);
[extension-serve](contracts/extension-serve-v1.md).

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
  It stores ciphertext, never decodes Yjs ([sync rules](.agents/skills/tmt-colab/references/architecture-sync-decoder.md#stream-sync)).
- **Dependency direction.** `tmt-colab` depends on `tmt-colab-model` (pure codecs and fixed
  crypto), `tmt-extension-state`, `-objects`, `-serve` and `tmt-invoke`/`tmt-cli-style`; the browser
  depends on Remote's served SDK (`/sdk/remote-v1.js`). Never `tmt-core`, `tmt-adapters`,
  `tmt-remote`; core is reached through `$TMT_EXECUTABLE api` and the fixed,
  bounded public `identity show ... --json` observations for display and routing references,
  and `ls --json` presence snapshots for advisory edit-lock lifecycle.
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
  Colab owns the mounted browser app, including trusted recovery and archived reads ([browser contract](extensions/tmt-colab/contracts/colab-v1.md#implemented-mounted-browser-assets-1253)).
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
- **Firestore admission.** Hidden `tmt colab deploy-declaration --json`; the [declaration subsection](extensions/tmt-colab/contracts/colab-v1.md#firestore-deployment-declaration) owns admission and codecs. Hosted folds verify log anchors before checkpoint pairs. ([native opening](.agents/skills/tmt-colab/references/architecture-state.md#native-opening-and-admission))
- **Plaintext.** Source, shell, discussion reads and export are root-local: only the isolated decoder
  child decodes Yjs, no browser route serves plaintext (bar owner-only local titles in `/api/pages`; [detail](.agents/skills/tmt-colab/references/architecture-state.md#native-opening-and-admission)), and the Worker is resource
  containment, not a security sandbox. Pure [publication codecs](extensions/tmt-colab/contracts/colab-v1.md#content-publication-1908-1928-1934) validate sealed intent; native code atomically stores content or a `kind:"own"` status action with its scoped terminal outcome in the Store, signed by the root-local writer. Metadata commands retain expected-revision and save operation/status fences; [editor metadata requests](extensions/tmt-colab/contracts/colab-v1.md#editor-metadata-requests-5195-a-5256) pass automatic owner admission and preserve root-local content authority. Per-action [routing](.agents/skills/tmt-colab/references/architecture-sync-decoder.md#page-source-and-export) is in the sync reference.
