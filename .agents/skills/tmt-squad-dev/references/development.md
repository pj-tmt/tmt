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
- Cron management (service, announcements, retirement): `cargo test --locked -p tmt-squad
cron_service` and the native `squad.test.ts` cron cases cover actor permission, locked
  room/revision refusal, exact messages, post-commit announcement recipients, hook
  registration rollback and obsolete/replayed retirement references; the private-tmux
  `squad.e2e.test.ts` cron case reads committed announcement and hook state independently.
- Tab parity: `built_in_board_documents_equal_ls_tab_documents` and
  `user_board_and_ls_share_members_sections_bindings_and_failed_reads` require board
  views and `ls --tab` to project identical documents, including hidden squads/tabs
  and partial-read recovery. `board::home::tests` checks the retained board model
  against that aggregate. Use an isolated `XDG_CACHE_HOME` when testing board
  observation.
- Home tile checks cover literal 160/100/80 geometry, exclusive non-lead marks/counts,
  missing/zero/partial observations, configured and mixed window labels, full selection
  in `tmt`, `tmt-light` and `NO_COLOR`, continuation clicks, complete selected-range
  reveal, clipped hits and resize without target drift. Capture quiet/waiting/blocked/many
  squads with isolated state; frozen board/list parity remains unchanged.
- Settings editor changes: cover live preview, focus and age-evidence restoration on cancel,
  invalid input, read-only command entries and stale-file refusal (native edits verify shared
  staleness after reload, including the disabled no-publication path); capture normal and
  narrow states from isolated HOME/`TMUX_TEAM_HOME` and a private tmux socket. The
  [Squad architecture](../../../../ARCHITECTURE.md#squad-extension) owns the preview and writer
  contracts.
- Board picker regressions cover shared query/list focus, identity retention on refresh,
  consumed close, scoped preview/save/cancel and stale-file refusal; check clipped mouse maps
  after resize or model replacement and semantic attention styles under background and
  reverse selection (same isolated 160/100/80, dark/light/`NO_COLOR` captures as below).
- Observed token usage checks cover per-identity window boundaries, configured
  1m–24h retention, tab/member cleanup, no-data/zero/gap aging and current model
  attribution. App projection preserves public `ls` JSON and invalidates only
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
