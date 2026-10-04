# Test infrastructure and evidence

[DEVELOPMENT.md](../../../../DEVELOPMENT.md#native-process-and-shared-tests)
owns basic setup and selection; this reference owns the helper boundaries and
failure behavior needed when changing native or Docker fixtures. Shared system
invariants stay in [ARCHITECTURE.md](../../../../ARCHITECTURE.md#testing-and-evidence-boundaries).

## Owners and evidence

Rust tests stay beside their owner. TypeScript suites live under
`typescript/test/{native,e2e,tooling,stress}` and share only `test/support`.
The tooling architecture guard rejects support importing a suite, native/E2E
importing each other or tooling, harness helpers importing their fixture, and
scenarios bypassing `harness.ts`. Focused tooling tests may import suite-local
helpers. `test/support/source-imports.ts` extracts literal imports, including
no-substitution templates and dynamic imports with options; computed loaders are
outside that static guard. Module resolution uses the tooling compiler options.

Native process tests prove public CLI contracts through the selected real binary;
storage tests independently prove migrations, rollback, contention, retention
and late-final behavior. Docker adds private tmux, caller/lifecycle, transport
and cross-process ordering. Tooling tests prove verifiers and policy, not runtime
or release archives. Real-companion `office-*` scenarios stay Office-owned,
including stress selection; shared setup stays in support. Retained-release
setup uses the private installer, while public frozen-product refusal belongs in
native lifecycle scenarios. See [scenario ownership](e2e-scenarios.md#scenario-ownership) and the
[smoke matrix](runtime-smoke-matrix.md) before replacing coverage.

Office's opt-in `playwright.visual.config.ts` reuses its local HTTP fixture and
real renderer for reviewed pixel baselines. Its read-only world is not an
admission or persistence oracle; the partition verifier keeps visual scenarios
separate from standard CI and capacity diagnostics. Geometry, gesture history
and native durability retain their own test owners.

## Executable fixtures

`test/support/executable-fixture.mjs` publishes executable and interpreted fixture
bytes for all TypeScript suites. A short-lived Node writer stages, fsyncs and
closes before chmod and atomic rename; the parent waits for writer exit. Synthetic
installers use its `--write FILE MODE` entry. Preserve caller-supplied bytes and
explicit executable or deliberately non-executable modes.

Rust's private `tmt-test-support` library owns only `write_executable`: a bounded
short-lived shell writer through `tmt-invoke`, with exact bytes/mode and no retries
or readiness policy. The reviewed untargeted dev consumers are Adapters, CLI,
Squad, Office, Colab and Office Command; production/build edges are forbidden.
The architecture guard enforces canonical dependency names and `dist=false`.
Every additional helper needs its own two-caller justification and review.
Follow [DEVELOPMENT's ETXTBSY rules](../../../../DEVELOPMENT.md#rust-checks)
for the three publication cases; scenario-local readiness/assertions stay local.

The `colab-runtime-fixture` example owns embedded tiny app bytes and selectable
defects; tooling scenarios own assertions. Its signal-hook dev dependency owns
SIGTERM cleanup. The `recording-cli-fixture` example owns synthetic extension
upgrade delegation with serde_json fixture configuration; release TOML edits
remain with the private release tool. Build and select them using the
[installation fixtures](../../tmt-release/references/installation-fixtures.md).
Tiny fixtures prove sensitivity, never actual archive delivery.

## Native sandbox lifecycle

`tmt-cli/tests/support` owns isolated Rust process environments and direct-child
lifetime. TypeScript `test/support/cli-process.ts` registers each sandbox run and
uses the native `runtime-caller-fixture` example to reparent before CLI spawn,
without a product guard override. A PID-1-adopted supervisor starts a second CLI
group; its bootstrap reports the PID and waits for the harness's ownership
acknowledgment before exec. The selected CLI leads its own group. Direct
runtime-caller positive controls remain separate.

The harness owns short private per-run Unix socket directories under `/tmp`,
avoiding Darwin's socket-path bound. The exact acknowledgment connection becomes
stdin without protocol bytes; no-input uses `/dev/null`. Inherited stdout/stderr
preserve CLI output, and Rust sockets close on exec except the stdin clone.
Reparenting/spawn use the original execution deadline; unknown adoption fails.
`TMUX_TMPDIR` stays sandbox-local so cleared caller variables cannot reach the
host default socket. Native process fixtures never start default-socket servers.

Setup close and socket drain are separate required events. CLI completion starts
both owned groups' cleanup even if descendants hold output pipes. Readiness
arriving during cleanup is still registered; closing acknowledgment cancels an
unstarted bootstrap. Success needs direct close and confirmed absence of both
groups before sockets, server, directory and fixture files are removed.

A SIGKILL or initial group-probe permission error is tolerated only after direct
close and a subsequent ESRCH group probe. Other errors, live groups or unknown
groups fail within the one-second cleanup bound. Never signal an unconfirmed
group. Disposal cancels outstanding runs first; failure retains fixture paths
and reports callback and cleanup failures together.

After callback cleanup, Linux's cwd guard checks inspectable same-user processes
under the canonical sandbox root (permission-denied discovery is skipped). It
reverifies each resident before signalling and confirms absence before deletion.
Inspection/cleanup failure retains files. Darwin and other platforms skip this
guard; descendants that leave the sandbox cwd are outside its claim.

## Docker harness lifecycle

`harness.ts` is the scenario facade. `harness/fixture.ts` owns resources and
process registries; `readiness.ts` observes caller-supplied events/panes/processes;
`cleanup.ts` stops/checks owned groups and clients; `types.ts` owns suite-local
values. Helpers never create a second fixture lifetime. Synchronous tmux calls
have a five-second SIGKILL bound. Install one trace per fixture, reuse `clear()`,
and refuse a second installation before changing its delegate.

Unknown group inspection stays pending inside the one-second cleanup bound;
surviving or uninspectable groups fail cleanup. Preserve fixture cleanup order
and error precedence. Pane-launched foregrounds remain scenario-owned: request
shutdown and await the shell's post-exit status plus channel-server absence on both
success and callback failure before harness deletion. Killing the tmux server alone
does not confirm a foreground's final storage writes have settled. Mock providers
settle their hook/reply children and stdio channel server before exit.
`cli-assertions.ts` owns only the strict success envelope
(zero exit, empty stderr, parsed JSON); exact payload projections stay in each
scenario. A different stderr/parsing contract cannot use that helper.

Public-command scenarios select through `test/support/cli-executable.mjs` and
the harness, including explicit descriptors and nested replies. Adapter scenarios
deliberately select the tmux probe, so their results never stand in for public
command acceptance.
