# Squad development

The shipped lead skill is `extensions/tmt-squad/skills/tmt-squad/SKILL.md`; this skill
is for developing the extension. Shared gates are in
[DEVELOPMENT.md](../../../../DEVELOPMENT.md). Board rendering uses the internal TUI
markup: see [tmt-tui](../../tmt-tui/SKILL.md).

## Checks

```bash
(cd rust && cargo build --locked)                       # default tmt-squad for the native test
(cd rust && cargo test --locked -p tmt-squad)
(cd rust && cargo test --locked -p tmt-squad --lib cron)
(cd rust && cargo test --locked -p tmt-squad cli_style_tests)
```

- `typescript/test/native/squad.test.ts` runs `rust/target/debug/tmt-squad` (or an
  absolute path in `TMT_TEST_SQUAD`) through real `tmt` dispatch, with a sandbox
  `PATH` holding the `tmt-squad` and `tmt-sq` links and no installed-copy fallback.
  It reads rooms and metadata through an independent SQLite reader.
- Squad-scoped CI runs `squad.test.ts`, `extension-install.test.ts`,
  `extension-upgrade-proof.test.ts` and the `squad` E2E files listed under
  `scopedChecks` in `.github/components.json`.
- The Docker Squad lifecycle test kills temporary and saved panes before the first
  list read, then checks the roster and SQLite retirement/binding state; run it
  twice for cleanup.
- Dependency changes: compare `cargo tree -p tmt-cli -e normal,build -f '{p} {f}'`
  with `main` and the package-scoped release `tmt` to prove the CLI is unchanged.
- Cron tests use disposable roots and must not touch the core database or
  `squad.toml`.
- `main::print_help` sends both routed help and clap `DisplayHelp` through
  `tmt_cli_style::rendered_help` before core discovery. Specs and argument help
  remain in `specs.rs`/`main.rs`; the shared style crate owns terminal wrapping.
  Main's help regression pins 80-cell wrapping and unchanged pipe bytes across
  root and nested commands; verify the real `tmt sq cron --help` in a private tmux.
- Hidden commands: `cargo test --locked -p tmt-squad cli_style_tests` also runs
  `tmt_cli_style::audit::hidden_report` over Squad's grammar against `HIDDEN` in
  `cli_style_allowlist.rs` (today only `__complete`). Board-only actions are not
  commands: do not add a CLI or hidden twin for a board key
  ([rule](../../../../design/cli-style.md#hidden-commands)).
- Cron management (service, announcements, retirement): `cargo test --locked -p tmt-squad
cron_service` and the native `squad.test.ts` cron cases cover actor permission, locked
  room/revision refusal, exact messages, post-commit announcement recipients, hook
  registration rollback and obsolete/replayed retirement references; the private-tmux
  `squad.e2e.test.ts` cron case reads committed announcement and hook state independently.
- Cron clock: `cargo test --locked -p tmt-squad --lib cron` covers lease competition,
  stale takeover, ownership-checked release, slot windows and operation IDs;
  `cargo test --locked -p tmt-squad cron_clock` covers fresh service admission,
  anonymous dispatch, same-ID receipt recovery and cancelled/joined children.
  Native `squad.test.ts` checks read-only status, explicit manual actors, exact
  paused sends and JSON/help. The private-tmux `squad.e2e.test.ts` clock case
  corroborates one slot acceptance and one causal peer wake with independent SQL,
  rejects a second clock, replays standalone ticks without another send and
  verifies signal cleanup. Run Docker lifecycle verification twice.
- Board cron: `cargo test --locked -p tmt-squad cronboard` covers the projection, line, list and
  controls against disposable roots (`cron_service::test_support::Fixture`); `view::tests::cron`
  covers the split squad tab, focus transfer, scoped keys and forms; `home::tests::cron` the home
  cursor. Capture the home line, `c` list, split tab, expanded job and forms at 160/100/80 in
  `tmt`, `tmt-light` and `NO_COLOR` with a private HOME, `TMUX_TEAM_HOME` and tmux socket.
- Row paint: `cargo test --locked -p tmt-squad board::view` covers painted cells and hits. A
  scene change keeps the frozen parity capture and compares the old and new renderers on
  literal buffers, styles and hits (themes, depths, widths, selection, stale, waiting, cron labels,
  annotation) before the old one goes.
- Pinned selection/focus evidence uses the ignored `capture_selection_focus_packet`
  test with explicit `TMT_SELECTION_PACKET` and `TMT_SELECTION_OUTPUT` paths.
  The existing `status::with_now_ms` test-only scoped override freezes integrated
  draws on that thread, restores nested/unwound scopes and leaves worker/production
  clocks unchanged. These buffers are deterministic renderer evidence, not a
  frozen live-terminal clock or permission to regenerate parity.
- Frame cost: the `#[ignore]` harness `board/view/tests/frame_timing.rs` times one 160x50 frame
  of crew rows, team rows, home with attention sections only (meter on) and home with every
  section through `Terminal::draw`, split by the phases of
  `view::render`, plus a `strip::paint_left` microbench. Run it with
  `CARGO_BUILD_JOBS=2 cargo test --release -p tmt-squad frame_timing -- --ignored --nocapture`;
  compare runs from one machine and build, never as a CI threshold. A render change keeps
  `render_replica_matches_render` passing (it pins the replica to `view::render`).
- Layout validation: `cargo test --locked -p tmt-squad layout` and the native `squad.test.ts`
  offline case cover invalid core/config inputs, located errors, the size bound and the
  human/JSON exit codes.
- Tab parity: `built_in_board_documents_equal_ls_tab_documents` and
  `user_board_and_ls_share_members_sections_bindings_and_failed_reads` require board
  views and `ls --tab` to project identical documents, including hidden squads/tabs
  and partial-read recovery. `board::home::tests` checks the retained board model
  against that aggregate. Use an isolated `XDG_CACHE_HOME` when testing board
  observation.
- Home tile checks cover literal 200/160/113/100/80 single-column geometry, exclusive non-lead marks/counts,
  missing/zero/partial observations, configured and mixed window labels, full selection
  in `tmt`, `tmt-light` and `NO_COLOR`, continuation clicks, complete selected-range
  reveal, clipped hits and resize without target drift. Capture quiet/waiting/blocked/many
  squads with isolated state. The board glyph guard reads registered marks from
  `design/tokens/tokens.json`, rejects Ambiguous and emoji-presentation decorations,
  and requires spaces before and after state marks, including overflow tab labels.
  Only the admitted local selection prefixes (`◆>`, `>◆ ` and `│>◆ `, with the
  existing semantic mark) substitute one adjacent blank; embedded or misplaced
  cues do not relax spacing. Flat lists retain one blank former-wall cell before `>◆`.
  Structural exceptions have explicit
  reasons in the guard; dynamic names, tasks and notebooks are outside its scope.
  Decode cell/style/hit differences from current main parity before requesting approval
  for any fixture regeneration; an approved regeneration has its own attributed commit.
- Settings editor changes: cover live preview, focus and age-evidence restoration on cancel,
  invalid input, read-only command entries and stale-file refusal (native edits verify shared
  staleness after reload, including the disabled no-publication path); capture normal and
  narrow states from isolated HOME/`TMUX_TEAM_HOME` and a private tmux socket. The
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
  at top and end of scroll, with isolated HOME, `TMUX_TEAM_HOME` and XDG cache, and
  the help modal at 160/100/80 columns. Meter CPU measurements (matched 60-second
  idle off/on/reduced-motion; budget below 0.5 percentage point of one core) are
  local evidence, never a CI threshold.

## Embedded skills

The squad executable embeds the lead skill and the playbooks
(`extensions/tmt-squad/playbooks/<name>/SKILL.md`), outside the core skill bundle.
`playbook.rs` tests pin the catalog to the source files; `squad.test.ts` covers
install and removal against isolated provider roots and checks the documented
status row shape against real output. Every command a playbook tells an agent to run
is executed once in a disposable tmux server and git repository before it is
written down. A row JSON or SKILL.md row-doc change also runs the native
`squad.test.ts`; give an optional row key its own bullet.

Squad archive build and verification are in
[tmt-release](../../tmt-release/references/native-release.md#squad-archives).
