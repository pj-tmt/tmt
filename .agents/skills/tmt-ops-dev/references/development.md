# Ops development

The shipped lead skill is `extensions/tmt-ops/skills/tmt-ops/SKILL.md`; this skill
is for developing the extension. Shared gates are in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md). Board rendering uses the internal TUI
markup: see [tmt-tui](../../tmt-tui/SKILL.md).

## Checks

```bash
(cd rust && cargo build --locked)                       # default tmt-ops for the native test
(cd rust && cargo test --locked -p tmt-ops)
(cd rust && cargo test --locked -p tmt-ops --lib cron)
(cd rust && cargo test --locked -p tmt-ops cli_style_tests)
```

- `typescript/test/native/ops.test.ts` runs `rust/target/debug/tmt-ops` (or an
  absolute path in `TMT_TEST_OPS`) through real `tmt` dispatch, with a sandbox
  `PATH` holding only the `tmt-ops` link and no installed-copy fallback.
  It reads rooms and metadata through an independent SQLite reader.
- Ops-scoped CI runs `ops.test.ts`, `extension-install.test.ts`,
  `extension-upgrade-proof.test.ts` and the `ops` E2E files listed under
  `scopedChecks` in `.github/components.json`.
- The Docker Squad lifecycle test kills temporary and saved panes before the first
  list read, then checks the roster and SQLite retirement/binding state; run it
  twice for cleanup.
- Dependency changes: compare `cargo tree -p tmt-cli -e normal,build -f '{p} {f}'`
  with `main` and the package-scoped release `tmt` to prove the CLI is unchanged.
- Cron tests use disposable roots and must not touch the core database or
  `ops.toml`.
- Checklist storage/service: `cargo test --locked -p tmt-ops checklist` exercises
  real temporary documents, literal schema/tombstones, separate revisions, contention,
  lifecycle and independently controlled public-port admission changes. Publication fault
  tests inspect committed bytes, temporary cleanup, lock release and retained Unknown
  after readback. Run unfiltered Squad tests on current/MSRV toolchains, owning clippy,
  fmt, CLI architecture/current+MSRV and the native Squad regression as well. This storage
  service remains the only admission/persistence owner. `cargo test --locked -p tmt-ops
checklist_command` covers adapter grammar, exact typed requests, literal JSON projections,
  alias/help/completion, authorized conflict/deletion errors and retained Unknown. Native
  `ops.test.ts` exercises actual dispatch, streams/exits and independently read durable
  bytes across revisions, archive/order, assignment departure and exact room lifecycle.
  No dependency, board action or parity fixture regeneration is introduced.
- `main::print_help` sends both routed help and clap `DisplayHelp` through
  `tmt_cli_style::rendered_help` before core discovery. Specs and argument help
  remain in `specs.rs`/`main.rs`; the shared style crate owns terminal wrapping.
  Main's help regression pins 80-cell wrapping and unchanged pipe bytes across
  root and nested commands; verify the real `tmt ops sq cron --help` in a private tmux.
- Hidden commands: `cargo test --locked -p tmt-ops cli_style_tests` also runs
  `tmt_cli_style::audit::hidden_report` over Squad's grammar against `HIDDEN` in
  `cli_style_allowlist.rs` (today only `__complete`). Board-only actions are not
  commands: do not add a CLI or hidden twin for a board key
  ([rule](../../../../design/cli-style.md#hidden-commands)).
- Cron management (service, announcements, retirement): `cargo test --locked -p tmt-ops
cron_service` and the native `ops.test.ts` cron cases cover actor permission, locked
  room/revision refusal, exact messages, post-commit announcement recipients, hook
  registration rollback and obsolete/replayed retirement references; the private-tmux
  `ops.e2e.test.ts` cron case reads committed announcement and hook state independently.
- Cron clock: `cargo test --locked -p tmt-ops --lib cron` covers lease competition,
  stale takeover, ownership-checked release, slot windows and operation IDs;
  `cargo test --locked -p tmt-ops cron_clock` covers fresh service admission,
  anonymous dispatch, same-ID receipt recovery and cancelled/joined children.
  Native `ops.test.ts` checks read-only status, explicit manual actors, exact
  paused sends and JSON/help. The private-tmux `ops.e2e.test.ts` clock case
  corroborates one slot acceptance and one causal peer wake with independent SQL,
  rejects a second clock, replays standalone ticks without another send and
  verifies signal cleanup. Run Docker lifecycle verification twice.
- Board cron: `cargo test --locked -p tmt-ops cronboard` covers projection/list/forms
  with disposable service roots. Workspace App tests cover scoped room/member entry;
  native service tests pin rename/counters and sticky missing-owner pauses. Capture
  Timers/list/forms at 160/100/80/60, including low height, dark/light/16-colour/
  `NO_COLOR`, with private HOME, `TMT_HOME`, XDG roots and tmux socket.
- Row paint: `cargo test --locked -p tmt-ops board::view` covers painted cells and hits. A
  scene change keeps the frozen parity capture and compares the old and new renderers on
  literal buffers, styles and hits (themes, depths, widths, selection, stale, waiting, cron labels,
  annotation) before the old one goes.
- Pinned selection/focus evidence uses the ignored `capture_selection_focus_packet`
  test with explicit `TMT_SELECTION_PACKET` and `TMT_SELECTION_OUTPUT` paths.
  The existing `status::with_now_ms` test-only scoped override freezes integrated
  draws on that thread, restores nested/unwound scopes and leaves worker/production
  clocks unchanged. These buffers are deterministic renderer evidence, not a
  frozen live-terminal clock or permission to regenerate parity.
- Frame cost: from `rust/`, run `CARGO_BUILD_JOBS=2 cargo test --release --locked
-p tmt-ops conversation_frame_timing -- --ignored --nocapture --test-threads=2` for
  steady, scoped update, response append, Reader scroll, real backend resize,
  1/10/100 KiB paste and typing after paste (36 squads, 150/1,000/10,000 agents,
  2,000 response lines, 160×36 cells). To separate event/paint/diff-and-ANSI phases,
  run `TMT_WORKSPACE_ANSI_SAMPLES=/absolute/task-owned/samples.json
CARGO_BUILD_JOBS=2 cargo test --release --locked -p tmt-ops workspace_ansi_phase_timing
-- --ignored --nocapture --test-threads=2`. Run its resulting test executable
  directly under `/usr/bin/time -lp` with the same environment and filter to
  measure process CPU/peak footprint separately; fixture/startup cost is included.
  The sink is memory, not a physical terminal latency measurement. Retain raw
  samples/outliers and match baseline/candidate source, toolchain and profile.
  The agreed local regression profile is Mac16,8, 12 logical CPUs, 48 GiB RAM,
  macOS 26.6.2, arm64 Rust 1.98.1/LLVM 22.1.8, release opt-level 3. Event+draw
  p95 must be ≤10/50/50 ms at 150/1,000/10,000 agents for each vector, with footprint
  ≤51 MB. Run main and candidate once, back to back on the same quiet machine and
  build mode; the ANSI probe's candidate whole-process user+system CPU must be
  ≤main × 1.05. Retain both results and all samples, without retries or dropped samples.
  Unchanged content keeps identical ANSI-byte/changed-cell vectors; approved new
  surfaces change only their expected regions. Input/paint performs no source I/O.
  An exceeded limit requires diagnosis or an agreed adjustment, not relabelling.
  These are probe/profile limits, not CI gates or user-facing/cross-machine SLOs.
  `workspace_frame_cost` is the separate roster-only 160×50 steady/scroll/resize
  probe; `frame_timing` is the independent CLI crew/team row-painter harness.
- Layout validation: `cargo test --locked -p tmt-ops layout` and the native `ops.test.ts`
  offline case cover invalid core/config inputs, located errors, the size bound and the
  human/JSON exit codes.
- Tab parity: `built_in_board_documents_equal_ls_tab_documents` and
  `user_board_and_ls_share_members_sections_bindings_and_failed_reads` require board
  views and `ls --tab` to project identical documents, including hidden squads/tabs
  and partial-read recovery. `board::home::tests` checks the retained board model
  against that aggregate. Ops test builds resolve `cache::directory` to one
  create-new directory private to the test process (never the host's `XDG_CACHE_HOME`/`~/.cache`;
  a later run sweeps the directories of finished owners), so tests need no cache injection;
  process tests still set their own roots.
- Workspace checks cover zero/8/36 squads, dense identities, UUID selection, scoped
  state/count truth, low-height scroll, clipped/stale hits, modal lifecycle and
  reload. `capture_workspace_packet` writes explicit cells/styles/hits to the
  task-owned `TMT_WORKSPACE_CAPTURE` path, never a golden fixture. Real private-tmux
  captures corroborate runtime behavior. The glyph guard uses the shared registered
  marks; untrusted names/tasks/notebooks remain inert text.
  Frozen HOME and row parity follow the owning
  [board parity gate](../../tmt-tui/references/development.md#board-parity-baseline),
  including decoded evidence and approval before the separate attributed recorder commit.
- Settings editor changes: cover live preview, focus and age-evidence restoration on cancel,
  invalid input, read-only command entries and stale-file refusal (native edits verify shared
  staleness after reload, including the disabled no-publication path); capture normal and
  narrow states from isolated HOME/`TMT_HOME` and a private tmux socket. The
  [settings editing reference](config-and-effects.md#settings-inspection-and-editing) owns the preview and writer
  contracts.
- Board picker regressions cover shared query/list focus, identity retention on refresh,
  consumed close, scoped preview/save/cancel and stale-file refusal; check clipped mouse maps
  after resize or model replacement and semantic attention styles under background and
  reverse selection (same isolated 160/100/80, dark/light/`NO_COLOR` captures as below).
- Observed token usage checks cover per-identity window boundaries, configured
  1m–24h retention, tab/member cleanup, no-data/zero/gap aging and current model
  attribution. Use the shared consumption-history contract vector for seed/live
  watermark subtraction, re-entry replacement and no-proration boundaries; verify
  public API batching and seed-on-entry without polling on ordinary reload. App projection preserves public `ls` JSON and invalidates only
  changed row derivations. Verify summary animation coordinates, sampled member
  cells and disabled buffer/ANSI equality through the normal renderer.
- UI changes: verify real private-tmux captures in `tmt`, `tmt-light` and `NO_COLOR`,
  at top and end of scroll, with isolated HOME, `TMT_HOME` and XDG cache, and
  the help modal at 160/100/80 columns. Meter CPU measurements (matched 60-second
  idle off/on/reduced-motion; budget below 0.5 percentage point of one core) are
  local evidence, never a CI threshold.

## Embedded skills

The squad executable embeds the lead skill and the playbooks
(`extensions/tmt-ops/playbooks/<name>/SKILL.md`), outside the core skill bundle.
`playbook.rs` tests pin the catalog to the source files; `ops.test.ts` covers
install and removal against isolated provider roots and checks the documented
status row shape against real output. Every command a playbook tells an agent to run
is executed once in a disposable tmux server and git repository before it is
written down. A row JSON or SKILL.md row-doc change also runs the native
`ops.test.ts`; give an optional row key its own bullet.

Squad archive build and verification are in
[tmt-release](../../tmt-release/references/native-release.md#ops-archives).
