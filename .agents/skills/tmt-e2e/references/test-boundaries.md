# Test infrastructure and evidence

[DEVELOPMENT.md](../../../../DEVELOPMENT.md#native-process-and-shared-tests)
owns basic setup; this reference owns native CLI selection, helper boundaries and
failure behavior needed when changing native or Docker fixtures. Shared system
invariants stay in [ARCHITECTURE.md](../../../../ARCHITECTURE.md#testing-and-evidence-boundaries).

## Native CLI selection

**Selecting the CLI.** The native selector resolves `rust/target/debug/tmt` and
fails if it is absent. An explicit descriptor may select another absolute
executable; neither an installed host command nor Node is a fallback, and paths
and argv are data, never shell fragments:

```bash
cargo build --locked --manifest-path rust/Cargo.toml
cargo build --locked --manifest-path rust/Cargo.toml --example storage-probe
TMT_TEST_CLI='{"executable":"/abs/rust/target/debug/tmt","args":[]}' \
TMT_TEST_STORAGE_PROBE='{"executable":"/abs/rust/target/debug/examples/storage-probe","args":[]}' \
  pnpm test:native
```

Installation and upgrade tests build their fixtures per the
[release skill](../../tmt-release/references/installation-fixtures.md). The suite
needs a Git checkout and Cargo: version expectations read `cargo
metadata --no-deps --offline --locked`, `rust/Cargo.lock` and `git ls-files -z`
once per suite. Native Rust CI selects the same-checkout release CLI; local
selection defaults to the debug build.

## Owners and evidence

Rust tests stay beside their owner. TypeScript suites live under
`typescript/test/{native,e2e,tooling}` and share only `test/support`.
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
or release archives.

## Scripted model transport

`test/support/mock-model-server.mjs` owns dependency-free loopback Messages and
Responses text streams, not provider launch or scenario assertions. It binds
`127.0.0.1` on an allocated port. Model POSTs require the exported synthetic token;
health and declared GET/HEAD probes return only fixed fixture data.
Callers supply finite text-turn queues per protocol/model, with SSE and JSON replies,
and close the handle in fixture cleanup;
`closed` resolves after listener shutdown. The detached census contains no headers
or prompt bodies. Declared side routes/models have fixed replies and per-route
bounds, are counted separately, and never consume scenario turns; unknown requests
still refuse. Counts prove replies served, not provider completion. A named refusal
remains a fixture failure even if later requests arrive. The server never forwards traffic.

Hard ceilings are 16 turns, 16 side declarations (at most 16 calls each), 64 total
requests, 1 MiB per request, 8 KiB text per turn,
64 KiB encoded response, 5 s per request body and 120 s server lifetime; callers
may lower them. Four concurrent connections are admitted. Tooling wire tests use
synthetic HTTP clients only; they do not qualify real harnesses, hooks, usage
projection or readiness. Real-binary hosted qualification and its narrow skill
exception belong to the separately reviewed advisory lane; this helper grants
no exception to ordinary E2E's deterministic-peer rule.

## Executable fixtures

`test/support/executable-fixture.mjs` publishes executable and interpreted fixture
bytes for all TypeScript suites. A short-lived Node writer stages, fsyncs and
closes before chmod and atomic rename. The parent hands it the bytes as a file on
stdin and takes its output from files, never a pipe, and waits under a bound
(default 60 s): a writer that overruns is killed and the error carries its pid,
`ps` line and `lsof` listing. Synthetic installers use its `--write FILE MODE`
entry. Preserve caller-supplied bytes and explicit executable or deliberately
non-executable modes.

Rust's private `tmt-test-support` library owns only `write_executable`: a bounded
short-lived shell writer through `tmt-invoke`, with exact bytes/mode and no retries
or readiness policy. The reviewed untargeted dev consumers are Adapters, CLI,
Ops and Colab; production/build edges are forbidden.
The architecture guard enforces canonical dependency names and `dist=false`.
Every additional helper needs its own two-caller justification and review.
Follow [DEVELOPMENT's ETXTBSY rules](../../../../DEVELOPMENT.md#rust-checks)
for the three publication cases; scenario-local readiness/assertions stay local.

Adapters' `provider-script-fixture` and `claude-hook-fixture` share the scripted
managed-launch option decoder in `examples/provider_launch`. Build the former
with `cargo build --locked --manifest-path rust/Cargo.toml -p tmt-adapters --example provider-script-fixture`.
Copy it under the provider basename and set `TMT_TEST_PROVIDER_INTERPRETER` to an
absolute interpreter path. Remaining argv is literal, stdio inherited, and child
exit status preserved (signal death is 128 + signal). The native parent forwards
TERM/INT/HUP and reaps its child; the acceptance world owns process-group/tmux
cleanup after uncatchable parent death and must assert no leftover child.

The `colab-runtime-fixture` example owns embedded tiny app bytes and selectable
defects; tooling scenarios own assertions. Its signal-hook dev dependency owns
SIGTERM cleanup. The `recording-cli-fixture` example owns synthetic extension
upgrade delegation with serde_json fixture configuration; release TOML edits
remain with the private release tool. Build and select them using the
[installation fixtures](../../tmt-release/references/installation-fixtures.md).
Tiny fixtures prove sensitivity, never actual archive delivery.

The Claude channel fixture publishes Digest control commands through
`test/e2e/claude-digest-step.ts`: complete a private same-directory staging write,
then rename to the watched path. Callers await consumption before the next command;
the mock keeps malformed published JSON a hard failure.
Its lifecycle mock treats only the exact zero-exit context-unavailable warning as
a counted diagnostic. Read-only committed session and Working activity checks
gate prompt submission/readiness; other stderr, malformed output and nonzero
exits remain failures. These checks share the existing launch deadline.

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
host default socket. Native process fixtures never start default-socket servers: a native
test that needs a real server starts it only through `test/support/private-tmux.ts`
(`privateTmux`), a Rust test through `test_support::assert_private_socket`, and the E2E
fixture proves its socket private at construction. Each refuses, before any tmux command, a
socket outside the test root, a symlink, or one inside a host socket directory or the
socket `$TMUX` names; `test/tooling/tmux-server-boundary.test.ts` lists the only files that
may name `kill-server` or `kill-session`.

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

Linux deadline diagnostics optionally observe the registered CLI and at most eight admitted sandbox residents before termination, with bounded identity-checked facts and explicit unavailability that never supplies signalling or cleanup authority.

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

Strip inherited `TMUX`/`TMUX_PANE` from child environments and verify the fixture's private socket before allowing binding writes; the tmux shim refuses unowned socket selectors. Explicit `-S` also admits exact peer sockets registered by live fixtures sharing the same canonical `globalDir`; fixture teardown revokes that peer entry. Default discovery and `-L` remain on the calling fixture's private namespace.

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

The development-only `tmux-probe caller` adds a bounded `diagnostic` only when
caller observation is absent or fails. It records the same command's typed
failure, deadline budget, elapsed time and retained output without another query
or a changed timeout. `outputAvailable: false` means output was not retained,
not that the command produced none. A later probe never explains an earlier
CLI naming failure by itself; keep their evidence separate.

## Pinned real-harness advisory lane

`test/e2e/real-harness-pins.json` owns exact provider versions and archive
SHA-512 integrity; `scripts/acquire-real-harness.mjs` validates and acquires them
only in the derived image build. `real-harness.config.ts` selects only the
advisory smoke, outside ordinary E2E selection. The separate workflow accepts
manual dispatch or the `real-harness` label on same-repository PRs, never the
required PR/merge-group path. Its container is loopback-only, uses a private
HOME/config and the existing private-tmux fixture, and installs hooks through
public `tmt setup` before public `tmt run` launch. Version mismatch stops before
any model turn. The image stays inside the job, without publication or an
external image cache.

The first smoke's `tmt check` output is diagnostic: a per-run marker occurs only
in the mock response and must appear through bounded public JSON polling.
SessionStart qualification additionally requires the public ls/whoami session
UUID to equal the validated UUID parsed from the pinned client's request.
The mock exposes only `sessionId`, never raw headers, metadata or request bodies.
Wire locations remain provisional until a hosted census proves them for each
pin; missing/moved fields fail qualification. Declarative side routes are added
only from that census and never consume a scripted turn. Public durable
exchange/result assertions belong to scenario 6 (#4549), not this diagnostic.
The workflow preserves bounded census, versions and measured setup/run seconds;
a budget row requires at least five samples. No local provider acquisition or
Docker run is authorized by this reference.
