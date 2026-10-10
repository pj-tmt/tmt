# Squad configuration and effects

## CLI entry and public JSON

Bare `tmt ops` shows help; bare `tmt ops squad` lists members with an explicit board hint.
Only `tmt ops ui` admits the interactive view. Bare squad `--json` equals squad `ls --json`. Display-ready documents exclude
board-only home/meter/`usage.*` data.

## `ops.toml` reading and writing

- `ops.toml` sits beside the global config that `tmt config show` reports. It is the
  user's file; agents never write it.
- Core executable discovery refuses `TMT_EXECUTABLE` or the first `tmt` on PATH when
  it names the running Squad executable, including symbolic and hard links. This
  rejects direct recursive configuration loading; select the Core `tmt` executable instead.
  Bootstrap config discovery sets `TMT_OPS_CONFIG_LOOKUP=1` only on its Core
  `config show --json` child. An inherited marker refuses another bootstrap spawn
  with `SQUAD_CORE_UNAVAILABLE`, bounding indirect launchers too. Ordinary Core/API
  calls and hook dispatch retain their existing policy; the parent environment is unchanged.
- `Config::write` owns format-preserving replacement for `me`/`me_id`, tab order, board
  views, theme bases and settings edits. It checks the original bytes, edits a cloned
  document, skips unchanged bytes and assigns the new document only after successful
  publication. A changed file is refused, not overwritten. The byte check plus atomic
  replacement is not a locking transaction, and no backups are created. Every named
  `Config::set_*`/`remove_*` edit goes through it.
- Source-bearing readers (`config::sourced`) carry provenance; presentation never
  inspects TOML or resolves values. `config::duration` converts UTF-8-safe whole-unit
  suffixes for provider, refresh and reminder timing; callers keep their own units,
  ranges and error messages, and only refresh wraps `"off"`.
- Layering (`Config` readers): the workflow layout comes from `Config::resolve_layout`,
  the one decision shared by the layout and board readers. Squads with no layout key use
  `team` unless they set the simple board form, which keeps `crew`; explicit `crew`,
  `pr-queue` and `minimal` keep their presets. `Config::board` resolves a hand-written
  per-squad `board.layout` or `panes` first, then per-squad `board.view`, then top-level
  `board.view`, then the boxed `members` arrangement. Existing simple `direction`/`sizes`
  configs without a named view retain their workflow pane preset. A user `rows` or legacy
  `columns` table replaces the grid; `fields.<name>` replaces that provider's whole
  table (extra provider names keep `pr`); reminder keys override individually. A nested
  board requires a full `layout` or `panes` override, and partial `direction`/`sizes`
  overrides are rejected.
- `Config::refresh`: per squad, then top-level `[board]`, then `DEFAULT_REFRESH` (5 s);
  `None` is off. `config::TokenRate` layers the team preset, global `[board.token_rate]`
  and per-squad keys; only Team defaults on.
- Strict validation happens before raw mode: pane/fold keys, `hidden_columns`, theme
  tokens, state entries, bindings and configured views are rejected with located config
  errors, including masked values that no current view would use.

## Ops path migration

`migration` owns one synchronized layout decision per invocation, shared by Core clones
and board workers before config discovery/loading or state access. A deferred decision
can promote to Ops; completed decisions stay cached. Core still supplies both roots
through public `config show` and `storage.root`; Squad never discovers Core paths itself.
The stable `.ops-paths.lock` beside the config serializes first runs and unfinished cleanup; a completed
`.ops-paths-v1` marker makes later decisions check only completion/legacy-name metadata, with no lock or legacy content reads. The marker is read only when the old config name exists, to distinguish an unchanged ignored file from a reappeared one.
Help, completion, embedded skills and offline layout checks never migrate.

| Path                                                                                                                      | Owner and migration                                                            |
| ------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `ops.toml`, `.ops.toml.<pid>`                                                                                             | Config/CAS writer; exact `squad.toml` migrates, staging is disposable.         |
| `<dataRoot>/ops/cron/{jobs.json,jobs.lock,jobs.tmp}`                                                                      | Cron document, stable writer lock and staging.                                 |
| `<dataRoot>/ops/cron/{clock.json,clock.lock,clock.tmp}`                                                                   | Clock lease, stable lock and staging.                                          |
| `<dataRoot>/ops/checklist/<room-uuid>/{items.json,items.lock,items.tmp}`                                                  | Checklist document, stable lock and staging.                                   |
| `.ops-paths-pending.json`, `.ops-paths-cutover-v1`, `.ops-paths-v1`, `.ops-paths*.tmp`, `.ops-migrate-<name>-<timestamp>` | Migration journal, durable cutover/completion markers and unpublished staging. |
| `squad.toml.migrated-<timestamp>`, `<dataRoot>/squad.migrated-<timestamp>`                                                | Retained original backups; no reader enumerates them.                          |
| `$XDG_CACHE_HOME/tmt-ops/{fields,staleness,back}`                                                                         | Disposable observations and navigation.                                        |
| `ops.tmux.conf`, tmux-config `.tmt-ops-backup-*` / staging                                                                | Consented hotkey owner; former `squad.tmux.conf` is retained.                  |

Old-only files are copied and synced, checked for byte/tree equality (config is also
parsed), then all new targets are atomically published and reverified. Only after
`.ops-paths-cutover-v1` is durable are old sources renamed into retained backups;
`.ops-paths-v1` marks completed archival. An interruption before cutover leaves every
old source available for live-lease deferral. After cutover, invocations use Ops and
resume archival without reading legacy content or reverting later Ops edits. A synced
journal distinguishes this work from pre-existing both-present paths. Source changes or unsafe files fail explicitly, preserving originals.
If both names already exist, Ops wins, old paths stay untouched and one notice names
them; completion suppresses repeated notices. Only old clock evidence is checked in an otherwise ignored state tree; unrelated legacy files and writers do not block Ops. User `squad.toml.bak-*` files are untouched.

Legacy state locks remain held during copy and cutover; a busy legacy writer defers instead of blocking startup. A live old clock defers the
whole migration: that invocation continues using legacy config/state and names its
holder PID/pane once. No command startup scans panes or processes for former boards.
A newly started UI stays clock-less while deferred, retrying migration on its existing
one-second interruptible clock-worker cadence. It acquires an Ops clock only after
cutover and config reload; foreground cron commands retain explicit legacy behavior.
When no old clock runs, scheduled sends pause until migration completes; normal clock
slot windows resume without replaying missed sends. The board keeps a short pending
notice, retargets its config watcher on promotion, and reloads once even with refresh off.
Retries use a nonblocking cutover lock and release the decision mutex before file/Core
work; legacy services keep their shared cutover guard while resolving/using the root.
A board that read the old config before archival fails its normal byte CAS instead of
recreating the missing file. Pending new-version writes also hold the cutover lock
and refuse after completion, even with an initially absent config. After cutover,
board watching follows `ops.toml` only. Metadata detection notices an exact legacy
config that reappears; it is never read or merged.

`tmt ops migration switch --yes` owns board discovery, stopping, cutover and
same-pane relaunch. The installer calls the same operation before deleting former
Squad. Nothing at command or board startup runs it: a `ps` plus one `lsof` per
foreground pane process made `ops ui` wait seconds before its first frame on a
board with many panes (#2342). A deferred migration shows its pending notice, and the
user or the installer runs the switch. `--socket` targets an explicit tmux socket when
caller context is unavailable; `--prefix` must match the current receipt-verified Ops
installation. Other servers are never scanned.

Ordinary eligible boards are same-user foreground processes in panes on that
socket whose kernel executable path is under the verified prefix's
`lib/tmt-squad/` and whose argv parses as the former `board` command. This
evidence remains usable after an earlier upgrade deleted the former installation.
Only the live legacy clock holder may instead use its lease PID and pane, a
same-user foreground process in that pane on the selected socket, a process
start no later than the original lease acquisition, and known former-board argv
(`tmt-squad`, `tmt-sq` or `sq`, with `board` or `ui`). This consented exception
allows an old clock's kernel path to be outside the former installation or
unavailable. A name, title, argv[0] or lease PID alone is never authority.
Captured PID/start, argv, tty, ancestry, foreground and lease ownership are
rechecked before TERM; ordinary boards also retain executable provenance checks.
Process start evidence uses the existing second-resolution `ps` observation.
A holder that cannot be verified stays running; the accepted automatic offer
names its pane and the manual exit/retry step once. Only the board PID is signalled;
shells, unrelated processes, existing Ops boards, panes and servers are not killed.

On Linux the switch reads exact NUL-delimited `/proc/<pid>/cmdline`; on macOS,
`ps` text must match the kernel path or former link name, `board`, and only known
flags with `--tabs`/`--squad` values restricted to `[a-z0-9-,]`. The clock-holder
exception admits a named former program outside that namespace and `ui`, with
the same bounded flags and values.
Deliberately forged argv[0] can retire another invocation of the same retiring binary; this bounded one-time risk is accepted.

The switch preserves pane IDs, cwd, `--tabs` (including omitted defaults),
`--squad` and `--popup`. A shell-launched board restarts through the same shell,
inheriting its environment; a dedicated pane respawns with the tmux pane environment.
Both set `TMT_EXECUTABLE` to current Core. Process environments are never captured
from `ps`. The original shell must regain the foreground; dedicated panes are
retained across exit and respawned without `-k`. Unavailable or changed evidence
leaves the board running. Successful relaunch requires acknowledgment after the
first loaded Ops draw.

`<dataRoot>/.ops-board-switch-v1.json` is separate from the migration journal:
private 0600, same-user regular files with no-follow opens, at most 32 boards and
1 MiB, serialized by a nonblocking switch lock. A switch without tmux retains an
empty pending record that binds to the chosen socket before any board effect.
On a consented retry, an older still-running record can acquire freshly verified
clock-lease provenance without replacing its saved identity or launch progress.
Launch details are synced before
TERM; launch submission is recorded before its effect, so an interrupted retry
recognizes acknowledged Ops processes instead of launching duplicates. Uncertain
submission is retained and reported rather than blindly resubmitted. Readiness
markers and the record are removed after completion. Failures retain partial
progress, the recovery command and former install evidence when still available;
they never roll back cutover or bypass configuration CAS fences.

## Team preset

The `team` preset uses the same `Layout`/`Board::preset` and config readers as the
others. Workflow defaults are crew states, pending-first order,
a member/state/task/pr/model grid with a pending line, a 60-second `github-pr` field and
a 30-minute observed-age default. Everything is configurable; the other presets keep
their defaults. Model reads the existing session projection and providers stay on the
shared fetcher path.

## Views (pane arrangements)

- The `view` command module owns the factory catalog and registers `view ls` (hidden
  `list` alias), `set` and `rm`; bare `view` lists. `members` is the default boxed
  squad list with notes below. `team` preserves the previous side-pane arrangement;
  `focus`, `notes`, `detail` and `wide` keep their arrangements and initial folds.
  `Board.members` selects the shared HOME list scene without replacing configured
  row/public-list projections. A view changes no states, rows, providers, reminders or
  meter policy. Explicit per-squad fold settings override factory defaults through the
  same Board reader; pane acquisition reads the resolved Board independently of the
  workflow layout. `wide` folds its middle column below 180 cells and relies on the
  solver's 40:30 redistribution; there is no width-dependent arrangement resolver.
- `Config::set_view`/`remove_view` edit only `view` in the chosen board layer. A scoped
  set refuses a hand-written layout with a manual-removal hint; reset keeps custom keys,
  and drops only a table that the reset emptied when its header has no comments.
  All-boards choices stay masked by custom or scoped arrangements.
- The bindable `view` verb (`l`) opens `board::view_picker`, mirroring the theme picker's
  scope, navigation and save/cancel lifecycle. The opening Config is the save baseline and
  refresh never replaces that draft. `App::effective_board` is the single presentation
  accessor for preview geometry, fold defaults and focus; per-tab `FoldState` keeps session
  overrides. Esc restores the opening Board and focus with the latest data and writes
  nothing; a successful save uses the normal changed-Board fold reconciliation. A custom
  arrangement previews in this-squad scope on a disposable Config copy, but scoped save
  refuses to remove hand-written keys; in all-boards scope a custom squad keeps its Board,
  shows the masking note and saves the global view for other tabs. The reset entry removes
  only the chosen layer's `view` key. The Reload request carries `preview_panes` only while
  the picker is open, loading missing notes/replies through the same loader and
  cancellation fence; closing it returns to resolved-pane acquisition. Built-in leads/all
  tabs keep their fixed composition throughout and offer all-boards scope only.

## Themes

- The `theme` module registers `theme ls` (hidden `list` alias), `set` and `rm`; bare
  `theme` lists. Lists and the picker take names and descriptions from
  `tmt-cli-style::Base`, never a Squad palette. The effective base is `default`, `cli`,
  `board` or `squad`; token overrides resolve independently.
- `Config` reads core's resolved appearance through public `config show`, then applies
  `[board.theme]` and `[squad.<name>.theme]` through `look::board_theme`. Invalid core
  appearance falls back to the built-in base with a notice; invalid Squad layers are
  configuration errors, validated per layer including masked values.
- `Config::set_theme_base`/`remove_theme_base` change only `base`. Squad never writes
  `config.json`, and command and picker text say that CLI colors stay unchanged.
- The bindable `theme` action (`T` in both host presets and the all tab) opens
  `board::theme_picker`. The session reads its Config at opening and keeps that baseline
  across refreshes. Preview applies the same in-memory edit as CLI set, cached by
  selection and scope, without writing; `App::look` supplies it to every pane and tab. Tab
  switches board/squad scope (built-in tabs have board scope only; a masking squad base is
  named). Pickers open in all-boards scope, even when a squad override masks it;
  the notice points to `r reset in ,`. Overlay input cannot operate underlying rows, tabs or panes. Enter calls the
  named edit once; a failed save keeps the draft and notice without retry; Esc drops the
  preview and uses the latest saved view.

## Settings inspection and editing

Board preference choices use the shared layer by default; explicit squad keys
remain opt-in overrides. The comma menu derives its `≠` marks and deduplicated
count from actual squad keys, not inherited source labels. Its selected override
alone offers `r reset`; `Config::reset_setting` removes exactly that key through
the existing validated CAS writer, preserving siblings and comments. Failure
keeps the effective state and existing diagnostic. Reset drops only an empty
immediate parent table whose header has no comments. Quick theme/view/window rows
share the full settings entry's provenance. Workflow and geometry edits retain
their required squad scope.

- `settings` coordinates arrangement, rows, notebook/state, meter, theme and tab/program
  projections. `config show` and the bindable inspection overlay (comma by default) share
  those results; the overlay owns its scroll position, blocks underlying input and keeps
  its opening snapshot through refresh (close/reopen reads later configuration). Provider
  argv, run bindings, state patterns and nested split structures are read-only, and neither
  inspection nor edit validation ever executes configured programs. `Config::bindings_for_tab`, `action::effective_bindings`
  and `tab_view::rows` keep inspection and loaded tab/section rules together. Aggregate
  tabs show fixed grids and global appearance without squad providers. `config show`
  without scope inspects board defaults; `--squad` and `--tab` are exclusive.
- `board.view` is editable globally and for squads without custom layout/panes.
  `preview_setting` uses the same `view_draft` as the view command/picker; it never
  writes during preview and global choices remain masked by scoped/custom layouts.
- Editing in the overlay (`board/settings.rs`): an editable entry opens a local input
  prompt. Each valid value calls `Config::preview_setting`, and the app applies the
  disposable board, rows, notes mode, state colors, interval and tab policy to the newest
  acquired data; the loader keeps raw core squad order so clearing tab order previews the
  same fallback as a reload. Invalid input leaves no draft. Esc restores the opening
  configuration and focus without discarding refreshed rows. Enter saves only through
  `Config::set_setting`, then refreshes values and sources; a file conflict stays in the
  prompt and never replaces concurrent edits. While the overlay is open the ordinary loader
  acquires preview notes/replies and metadata behind its existing cancellation fence, and
  closing returns to resolved-pane acquisition.
- `board.home_replies` remains a recognized obsolete global key so existing TOML
  loads. Its value is ignored, with one deprecation notice; inspection exposes no
  editable entry, and `config set` refuses it without changing authored bytes.
  HOME reply previews are collapsed by default; row expansion belongs to `row_detail`.
- `config::edit` owns the shared edit policy and a disposable validated Config draft.
  `sq config set KEY VALUE` accepts layout preset, flat split panes/direction/sizes,
  refresh, notes mode, hidden tracks, exact state colors, global tabs order/hide and the
  selected squad's reminder enable/threshold (booleans `true`/`false`, thresholds through
  the `Config::reminders` whole `s`/`m`/`h` validation, 1 m–24 h); arrays use JSON. Structural edits of a nested split tree refuse rather than flatten a
  custom or factory tree. Partial flat edits keep the workflow preset and seed missing flat
  split keys from the resolved arrangement. The draft runs the existing area validators,
  then `Config::set_setting` calls only the `Config::write` compare-and-set path; a changed
  file is refused and comments, order and unrelated keys survive. CLI edits touch no roster
  or member metadata, install no provider hooks and grant no extension consent. A reminder
  preview reclassifies only known ages in memory from the board's newest
  `staleness::Snapshot` (disabled previews remove marks; unknown ages stay unknown) and does
  no observation, cache publication or reminder claim; confirmed edits reach the shared
  observer on the ordinary reload.
- `board::help` projects navigation, effective bindings and meter explanations into
  `tmt-tui::components::KeyHelp` sections; `Action::description` owns binding wording for
  help and settings, while settings keep their literal JSON value and source apart from
  presentation prose. Meter input keeps observed roster names (including the lead and
  members omitted from displayed rows) for excluded labels.

## Actions and effects

- `action` parses `[bind]` and `[squad.<name>.section.bind]` once per load into events and
  actions whose arguments are templates; bad events, actions or field syntax are config
  errors. The board resolves the selected row's section binding, then `[bind]`, then the
  host preset (tmux: Enter and double-click jump; plain terminal: they open the row's
  action menu) into a fully filled request before anything runs; a missing value is a
  notice, never a partial action. One parser/dispatcher owns `toggle <pane>...`.
- `effects` holds the row actions behind plain `jump`, `open` and `copy` and the board.
  `template` fills `{field}` placeholders into one value and refuses empty values.
  Programs run as argv, never through a shell: the top-level `opener` and `clipboard`
  arrays, or the system opener. An opener starts in its own process group with null stdio
  and a reaper thread. `run` fills one argv element per template (refusing a value that
  would start an argument with `-`) and starts it the same way.
- Copy prefers the configured program; inside tmux it then uses
  `tmux -S <invoker socket> load-buffer -w -`, where `-V` must report 3.2 or later and
  `show -sv set-clipboard` decides whether the text reached the clipboard or only a buffer;
  otherwise it writes OSC 52 to `/dev/tty`.
- `jump` checks membership and calls `tmt focus`; Squad has no focus logic of its own.
  `jump --lead` finds the squad from `--squad`, else the caller's identity (`tmt whoami`) in
  exactly one roster, else the only squad, and jumps to that roster's lead; no lead is a
  refusal before any focus.
- `back` keeps a disposable stack per tmux server and client
  (`$XDG_CACHE_HOME/tmt-ops/back`, 0700, atomic replacement, `LIMIT` 32 entries; corrupt
  or foreign files read as empty). Every jump pushes the pane the client left, under the
  client `tmt focus` reports; `back` asks core for the invoker's client with
  `tmt focus --client`, pops its entry and focuses it.
- `send` uses public commands only: detached `talk --identity <sender> --room squad-<name>`
  with operands after `--`; annotations as a talk tagged `[<squad> · <row>]`; answers as one
  `tmt answer <member> --request <id>` (core selects and proves the request, no receipt
  passes through Squad); nothing acknowledges. Squad has no talk, reply, replies or annotate
  command: conversation is core's, and a note on a row is the board's `a` key only (an in-process
  `Request::Annotate`), never a command or a hidden `__` entry.
- `hotkeys` generates `ops.tmux.conf` (bindings noted `tmt ops popup|pane|back|lead`;
  the optional lead key's `run-shell` job has `TMUX` but no `TMUX_PANE`, so it passes
  `TMUX_PANE=#{pane_id}` for core to name the caller) and owns one `source-file` line in
  the user's tmux configuration. It edits that file only after consent, rereads it before
  publication, keeps a byte-exact backup and replaces it atomically with the original mode;
  install retires the exact former source line; removal drops exact current/former lines.
  The former generated file is retained. A linked configuration is resolved (at most eight
  hops, each relative to the link's real directory) and written beside its real file;
  dangling or looping links are refused before consent. Bindings record the first `tmt` on
  PATH that resolves to the running executable, not the release path. Collisions and
  ownership on the running server come from `list-keys -N -P "" -T prefix` (notes) and
  `list-keys -T prefix` (commands), since `list-keys -F` postdates tmux 3.2; Squad unbinds
  only keys whose note is its own. `board --popup` ends the session after a successful jump.

## Playbooks

Optional playbooks (`tmt ops playbook ls|show|install|rm`, first `tmux-squad`) live in
`extensions/tmt-ops/playbooks/`, deliberately not under `skills/`: the release archive
ships and the extension installer offers every skill under `skills/`, while a playbook is
installed only on request, and a test pins that no playbook is in that tree. `playbook.rs`
holds the one catalog of embedded sources and registers the subtree through `tmt-cli-style`.
`show` prints the exact bytes; `install` asks (the same `consent` helper as `hotkeys`), then
calls `skills.install` as owner `squad`; `rm` calls `skills.remove` with the playbook's name,
so the lead skill and `tmt extension rm squad` are unaffected. Squad never writes a provider
directory and never executes a playbook. The lead skill source is embedded only in the squad
executable, never in the core skill bundle.

## HOME lead sends

`send::leads` owns all-leads and picked-lead audience validation; `send::dispatch` owns the shared frozen intent, private journal and acceptance recovery used by it and status announcements. The existing `App.input`
keeps the opening user UUID and squad/lead occurrences; both submission and the
send effect validate current authority. The public dispatch deduplicates recipient
UUIDs. `send::new_operation` supplies a fresh UUID for explicit sends, shared with
manual cron sends. Before dispatch, a private 0600 intent under
`board-dispatches/<operation>.json` beside Squad configuration is synced. Confirmed
acceptance removes it; uncertain acceptance retains it for manual inspection.
One `dispatch.show` read can recover lost output. There is no automatic resend or wake retry. An explicit status notification retry first recovers uncertain acceptance; after the owned invocation has closed and a definitive missing receipt, it may create only the same frozen operation/intent again. Confirmed acceptance is never created again. Feedback distinguishes each recipient's queued or
unavailable acceptance without claiming delivery or processing.
