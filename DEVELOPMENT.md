# Development

The Rust workspace is the shipped CLI runtime. The nested `typescript` pnpm
workspace owns private developer tooling for Vitest, fixtures and release
verification; it is not an npm product or a CLI fallback. Repository policy is in
[AGENTS.md](AGENTS.md), architecture ownership in [ARCHITECTURE.md](ARCHITECTURE.md)
and style in [CONVENTIONS.md](CONVENTIONS.md). This guide holds the commands and
gates every change shares; per-area procedures live in the skills below.

| Working on                                                                                             | Load                                                                              |
| ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------- |
| Release workflows, CI selection, archives, installer, upgrade, release PRs, release tracking           | [tmt-release](.agents/skills/tmt-release/SKILL.md) (`references/ci-selection.md`) |
| Docker E2E, native process fixtures, test helpers and cleanup, smoke matrix, provider and Herdr checks | [tmt-e2e](.agents/skills/tmt-e2e/SKILL.md) (`references/test-boundaries.md`)      |
| Core runtime internals: identity, bindings, requests, storage, hosts, drivers, channels, installer     | [tmt-core-runtime](.agents/skills/tmt-core-runtime/SKILL.md)                      |
| Driver, runtime, completion, request-ID and CLI-style focused checks                                   | [tmt-dev](.agents/skills/tmt-dev/SKILL.md) (`references/focused-checks.md`)       |
| Ops extension and board                                                                                | [tmt-ops-dev](.agents/skills/tmt-ops-dev/SKILL.md)                                |
| Internal TUI markup (`tmt-tui`)                                                                        | [tmt-tui](.agents/skills/tmt-tui/SKILL.md)                                        |
| Remote door and embedded browser client                                                                | [tmt-remote](.agents/skills/tmt-remote/SKILL.md)                                  |
| Colab executable, app and browser client                                                               | [tmt-colab](.agents/skills/tmt-colab/SKILL.md)                                    |
| Handbook site and translations                                                                         | [tmt-design](.agents/skills/tmt-design/SKILL.md)                                  |
| Adding or moving files                                                                                 | [tmt-layout](.agents/skills/tmt-layout/SKILL.md)                                  |

## Setup

Requirements: Node.js 22.12 or newer, the pinned pnpm (through `corepack`) and the
toolchain in `rust/rust-toolchain.toml`. The workspace MSRV is Rust 1.95 and CI
also runs the pinned release toolchain. Remote-client tests need `python3` for the
independent byte-fixture oracle, shell completion tests need Bash and Zsh, and
runtime proof needs the macOS developer tools or Linux `readelf`.

```bash
(cd typescript && corepack pnpm install --frozen-lockfile)
(cd rust && rustup show)
```

pnpm commands in this repository run from `typescript/` unless a command changes
directory; Cargo and Docker commands run from the repository root. Use the
pinned pnpm lockfile, not another package manager. `better-sqlite3` is a
development-only independent SQLite oracle, never a reason to reopen native
schema state through Node.

Never install the product globally while testing. Keep application state,
provider directories, prefixes, sockets and child processes inside a task-owned
temporary root.

## Keep local development from filling the disk

Docker's build cache and image tags, and one Cargo `target` per worktree, are the
largest waste. Before a Docker suite run the read-only check; it prints free
space (warning below `TMT_DISK_WARN_GB`, default 30) and `docker system df`, and
deletes nothing:

```sh
scripts/dev-disk-check.sh
```

Below the threshold, stop and tell the maintainer. Delete only what you created,
and never restart Docker Desktop.

- **One image tag per worktree.** Name every local verification image after the
  worktree so a rerun replaces it; a tag never names a PR or issue:

  ```sh
  worktree=$(printf %s "$(basename "$(git rev-parse --show-toplevel)")" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9_.-' '-')
  docker build -t "tmt-performance:$worktree" ...   # <purpose>:$worktree
  ```

  `pnpm test:e2e` already uses a per-run tag and removes it on exit.

- **Do not schedule Docker build-cache pruning.** No `docker ... prune` is
  part of this development workflow; follow the disk threshold above.

- **Each worktree keeps its own `rust/target`.** Native selectors,
  `scripts/tmt-dev.sh` and the Docker fixtures read `rust/target/debug/...`, and a
  shared directory would run another worktree's binary. Never point
  `CARGO_TARGET_DIR` at `/tmp` or elsewhere. Reuse a clean worktree instead of
  adding one: `git status --short` empty and the branch pushed or merged, then
  `git fetch origin` and `git switch -c <new-branch> origin/main`.
- **Remove a worktree when its PR merges**, in the same turn, following
  [AGENTS' delivery lifecycle](AGENTS.md#delivery-lifecycle) and
  [`scripts/dev-worktree-remove.sh`](scripts/dev-worktree-remove.sh):

  ```sh
  scripts/dev-worktree-remove.sh <worktree-path> <pr-number>
  ```

  The script requires empty `git status --short` and either non-null REST `merged_at` or
  an upstream with empty `git log @{u}..`; open, closed-unmerged and failed PR
  lookups require the upstream proof, which alone is not enough after a
  server-side rebase. If it refuses, stop and ask the maintainer; never remove by
  hand, with `--force`, or by deleting the branch. Only after it succeeds, remove
  that worktree's images (`docker image rm "<purpose>:$worktree"`; skip never-built images).

## Run the workspace CLI

```bash
(cd rust && cargo build --locked -p tmt-cli)
sh scripts/tmt-dev.sh --version
(cd rust && cargo build --locked -p tmt-ops)
./rust/target/debug/tmt-ops --help
```

`scripts/tmt-dev.sh` and `pnpm tmt` launch only this checkout's
`rust/target/debug/tmt`: they never build, install or fall back to a global
binary, so rebuild after changing Rust sources. For clean JSON stdout use
`(cd typescript && corepack pnpm --silent tmt ...)`. Normal commands still use
the usual application data unless you select isolated settings. Extension
executables resolve core lookups through `TMT_EXECUTABLE` (set when TMT invokes
them) or an executable `tmt` on `PATH`.

Contracts: [local process API](contracts/extension-api.md) for extensions and the
[MCP contract](contracts/mcp-v1.md). Credential-free native MCP commands:
[tmt-dev focused checks](.agents/skills/tmt-dev/references/focused-checks.md#local-mcp).

## Rust checks

Run from `rust/` for a normal native change:

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
cargo build --locked --example storage-probe
cargo build --locked --example tmux-probe
cargo +1.95.0 build --locked
```

Use `CARGO_BUILD_JOBS=2` on shared machines.

**Product builds are package-scoped.** Workspace and combined builds unify
dependency features, so their `tmt` can differ from the product's. Identity proofs
compare `cargo build --locked --release -p tmt-cli` alone, at the same checkout
path, before and after a change. Build the native-test fixtures separately,
after workspace checks, with `CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p
tmt-cli` (no debug symbols, but debug assertions stay on). Every Cargo build stage
in Docker contexts must copy all workspace member directories at their
workspace-relative paths; `docker-workspace.test.ts` checks it.

Executable fixture publication and snapshot updates:
[tmt-dev focused checks](.agents/skills/tmt-dev/references/focused-checks.md#executable-fixtures-and-snapshots).

CLI style and printed-command guard procedures: [enforcement](design/cli-style.md#enforcement).
Focused checks: `cargo test --locked -p tmt-cli --bin tmt cli_style` and
`cargo test --locked -p tmt-ops cli_style`.

### Architecture guard

Part of `cargo test`; a focused offline run:

```bash
cargo test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
cargo +1.95.0 test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
```

Run it on every Rust push: it enforces workspace-unique names, dependency
direction and the reviewed dev edges. When changing a guard, exercise a real
positive and negative source or dependency fixture and restore the checkout
exactly; a stale lockfile or an unexecuted test is not evidence. Core stays free
of concrete I/O, adapters own SQLite/files/processes, the CLI owns grammar and
composition.

The adapter `process::cleanup_policy_tests` must pass under both `cargo test` and
nextest. Driver, runtime, completion and request-ID checks are in
[tmt-dev's focused checks](.agents/skills/tmt-dev/references/focused-checks.md).

## Native process and shared tests

JavaScript suites live under `typescript/test/{native,e2e,tooling,support}/`; Rust
tests stay beside their owner in `rust/crates/*`.

**Isolation.** Fixtures clear the parent environment. HOME, XDG config/data/state/
cache, `CODEX_HOME`, temporary files and the tmux socket directory stay under the
fixture's owned root, and state uses the canonical XDG `tmt` directory. They
inherit no caller/provider markers, recursion flags or color settings.
`tmt-cli/tests/support` owns this for Rust process tests. Native TypeScript
`runCli` also isolates CLI ancestry through
`rust/crates/tmt-adapters/examples/runtime-caller-fixture.rs`
([testing boundaries](ARCHITECTURE.md#testing-and-evidence-boundaries)); build it
before tooling or native tests, as CI does:

```bash
cargo build --locked --manifest-path rust/Cargo.toml -p tmt-adapters --example runtime-caller-fixture
```

CLI descriptors and suite prerequisites: [native CLI selection](.agents/skills/tmt-e2e/references/test-boundaries.md#native-cli-selection).

Process-job fixture builds: [installation fixtures](.agents/skills/tmt-release/references/installation-fixtures.md#process-fixture-builds).

**Rules.**

- Prove the missing-native negative control and the selected-native positive
  control. Child processes are finite, stopped and reaped before fixture deletion,
  and signalled only when task-owned. Never use host tmux, global provider state or
  process-wide environment mutation as setup.
- A settled or deleted batch is not process completion: retain each detached
  worker's exact process incarnation and confirm its exit in scenario cleanup.
- Use `withSandbox` for callback-owned fixtures. Disposal stops outstanding
  commands before deleting files; an unconfirmed process group is never signalled,
  and unconfirmed cleanup fails and reports the retained fixture path instead of
  deleting possibly live state. Lifecycle regressions live in
  `test/tooling/cli-process.test.ts`. On Linux a post-callback cwd guard fails on
  resident processes; it is skipped on macOS.
- Frozen migration fixtures and provenance under
  `typescript/test/fixtures/storage-history/` are immutable evidence: never
  generate expected data with the implementation under test. For schema changes
  update `typescript/test/native/storage-fixture.ts` and the explicit
  migration/table assertions, then run the complete process suite; Rust storage
  tests do not replace process-level migration and future-version rejection.
- Tooling tests: `(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling)`.

Herdr, provider-contract, load and performance checks are in
[tmt-e2e](.agents/skills/tmt-e2e/SKILL.md).

## Docker E2E

For lifecycle, transport, identity, talk or cleanup changes, run one focused
private tmux and caller lifecycle selection for the affected scenarios. Run it
twice only when a lifecycle or transport change needs repeat evidence for leaks,
cleanup ordering or non-idempotent teardown; record why.

```bash
(cd typescript && TMT_E2E_FILES="affected.e2e.test.ts" corepack pnpm test:e2e)
```

Replace the example with the affected plain file names, space-separated.

Docker selection knobs and shard balancing: [E2E scenarios](.agents/skills/tmt-e2e/references/e2e-scenarios.md#selection-and-sharding).

The harness builds its pinned image, then runs it with `--network none`, private
tmux sockets and deterministic mock agents, and must not reach a host tmux server
or a real agent. The wrapper's `--init` is part of orphan-child cleanup evidence. The image
places its binary at `rust/target/debug/tmt` and leaves the selector unset, so it
exercises the developer default path. Scenario ownership and harness rules are in
[tmt-e2e](.agents/skills/tmt-e2e/SKILL.md).

Ordinary developer checks:

```bash
(cd typescript && corepack pnpm check)
(cd typescript && corepack pnpm test:run)
```

`pnpm check` is the quality entrypoint for all retained tooling (`pnpm type:check`,
`pnpm lint`, `pnpm format:check` run its parts); `pnpm docs:format:check` covers
docs. Neither replaces the Rust commands, the native process suite or Docker runs.

## Conventional PR title checks

The single approved type policy is `CONVENTIONAL_PR_TYPES` in
`typescript/scripts/pr-title-check.mjs`; the [release reference](.agents/skills/tmt-release/references/native-release.md#conventional-pr-titles)
owns syntax, released-path scope, edit feedback and cumulative merge-group enforcement.
`--report-only` is explicit observation compatibility, never the required CI gate.

## Installed guidance source ownership

`skills/tmt/SKILL.md` and `skills/tmt-inbox/SKILL.md` are the canonical
installed guidance, one versioned bundle; core install exposes only these. Edit the
single canonical file; never add provider-specific copies. Verify exact embedded
bytes, managed links, repeat no-op, catalog replacement, API conflict/backup and partial warnings,
lock ownership and no effects on SQLite or tmux. `test/native/legacy-extension-skills.test.ts`
owns the legacy-bundle regressions, `skill_installation::owned_tests` the
extension-owned skills, and every fixture uses the isolated HOME/config sandbox.
Provider and custom-root usage: [Settings chapter](site/src/chapters/settings.mdx)
and `skills/README.md`. The Ops lead skill
(`extensions/tmt-ops/skills/tmt-ops/SKILL.md`) and playbooks are embedded by the
Ops executable and sit outside this bundle (see
[tmt-ops-dev](.agents/skills/tmt-ops-dev/SKILL.md)).

## Project tracking

Read progress in [pj-tmt project 1](https://github.com/orgs/pj-tmt/projects/1), filtered
to `label:epic`. Trackers use `Epic: <name>` and label `epic`; only native direct
sub-issues count. Umbrella/findings-log issues stay outside. Never set, clear
or wait for the retired Project `Epic` field.

Project fields:

- `Squad`: the squad whose lead owns the issue.
- `Owner`: reserved; leave empty.
- `Status`: `Todo` (not started); `In Progress` (implementation started, including
  draft or stacked PRs); `In Review` (a PR is ready or queued; in a stacked chain
  while any PR is queued); `Merged` (the last required PR is on `main` and a
  release is pending; a `Fixes #N` merge sets it); `Released` (every affected
  product has a published tag containing the closing merge commits; release
  automation sets it and fills `Released in`); `Done` (closed without a delivering
  merged PR, delivery confined to never-shipped leaves, or only parked-product waits;
  release automation sets it).
- `Agents`: comma-separated agents actively building or coordinating it now, lead
  first; reviewers who build nothing are not listed. Removing a member from
  `Agents` is part of its retirement checklist.
- `Priority`: `urgent`, `xhigh`, `high`, `med`, `low`; squads take work in this order.
  The maintainer ranks epics; empty means parked/unranked on epics.
  PM/leads triage other issues. The maintainer sets `urgent` for meteor feedback:
  a broken main flow, data loss or a regression in a fresh release.
- `Released in`: release automation records the containing product tags.

Tracker rules:

- One lead per tracker, recorded in `Squad`. That lead writes its Status
  and body; each child keeps the Status and Agents of the lead doing the work.
- Keep a three-to-six-line `Now / Next / Blocked` body, outcome first with issue
  numbers, plus `Try it` when runnable. Update on merge, member start or
  retirement, or a blocker; keep logs and evidence in child issues/PRs.
- Tracker Status is `In Progress` while a child is active, `Todo` if nothing has
  started or it is parked (say so in `Now`), `Merged` when all required delivery
  is on `main`, and `Released` when it is in use and acceptance/dogfood gates pass:
  published for products, operating on `main` for CI/tooling/release machinery.
  Automation skips trackers; their lead sets Status, keeps gates under `Blocked`,
  and never reopens a delivered milestone for optional future children.
- New epics require the maintainer's approval; leads propose them through tmt-lead.
  No agent creates one on its own. Epic work requires the maintainer's explicit go; until
  then its issues stay `Todo`. Leads/PM freely open children below approved epics:
  one outcome, acceptance criteria and normally one PR; split work hiding progress.
- PM checks at :05 hourly (usage, idle seats, green unarmed PRs) and updates Project
  status at :35 (progress, releases, seats, usage, epics and owner decisions).
  The first tick after 08:00 JST also posts daily usage on #1518. Leads add
  event-driven updates; [PM procedure](.agents/skills/tmt-pm/SKILL.md) owns the steps.
  Batch edits, never poll; stay near 200 GraphQL calls per lead per day:
  one account-wide GraphQL limit is shared.

## Review and evidence

Before opening a PR, run the relevant checks for the changed owners and report
the reviewed revision, commands and results. Required CI is the full gate;
local focused checks do not replace it. Use the owning references above for
tooling, Rust, native process (including missing/selected-native controls),
Docker, host smoke and matching-host archive/bootstrap proofs. Issue-specific
qualification and admission requirements still apply.

Never turn a filtered, skipped, cross-compiled or failed subprocess into a success
claim. Preserve exact bytes and independent oracles, keep returned errors distinct
from crash recovery, and retain uncertainty, transaction, retention,
acknowledgment, lifetime and cleanup evidence when changing those boundaries.
Compare exact file bytes, not symlink-directory snapshots or enumerated binary
objects; use structured output or a focused formatter test, not mocked
`console.log`. Test design: [CONVENTIONS](CONVENTIONS.md#tests-and-review); isolation, evidence and
maintenance: [ARCHITECTURE](ARCHITECTURE.md#testing-and-evidence-boundaries) and its
[maintenance contract](ARCHITECTURE.md#maintenance-contract).
