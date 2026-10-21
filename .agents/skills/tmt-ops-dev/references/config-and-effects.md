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
  publication. A changed file is refused, not overwritten. The shared
  `tmt-extension-state` Document lock serializes runtime writers: the byte check and the
  atomic replacement run under it, a busy file is refused after five seconds, and a
  no-op edit takes no lock. Publication remains bounded to 1 MiB, and no backups are
  created. Every named `Config::set_*`/`remove_*` edit uses this owner. The migration
  cutover guard stays separate.
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

Shared unread markers live in `[workspace.read_markers.<actor-uuid>]`, keyed by
member UUID with a positive safe-integer submission timestamp. `config::unread`
validates canonical UUID pairs and max-merges updates into a freshly read document
under the same document lock. Separate layouts cannot regress or erase one another's
markers or unrelated authored settings. The field is separate from `OpsPreferences`,
so a quiet read update cannot invalidate a panel/pin preference comparison. It is
shared Config data, never a named-layer ViewState hint; reset-view and exec reload
leave it intact. The existing ordered action lane owns these writes and their
visible failure/retry outcome. Failed saves remain pending without an automatic
retry; quitting or exec reload reports an unresolved save after restoring the
terminal, even if its lane event was not consumed. A popup stays open after a
successful jump whose marker save failed. Fresh shared Config observations also
resolve a pending failure when another layout already saved that exact watermark.
No Core acknowledgment is changed.

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
| `ops.toml`, `.ops.toml.tmt-staging`                                                                                       | Config/CAS writer (shared Document); exact `squad.toml` migrates.              |
| `<dataRoot>/ops/ops.db` (+ SQLite `-wal`/`-shm`)                                                                          | Cron and checklist [documents](data-and-state.md#ops-state-database).          |
| `<dataRoot>/ops/cron/{clock.json,clock.lock,clock.tmp}`                                                                   | Clock lease, stable lock and staging.                                          |
| `<dataRoot>/ops/cron/jobs.json.migrated-<date>`, `<dataRoot>/ops/checklist/<room-uuid>/items.json.migrated-<date>`        | Previous release's documents after import; kept, never read or deleted.        |
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

Nothing stops, switches or relaunches a former board: the Squad installation is retired
(#3836, [install architecture](../../tmt-core-runtime/references/install-architecture.md#retired-squad-installation))
and no `migration` command exists. A live old clock defers the migration as above; the
invocation and the board name the holder (PID and pane) and ask the user to quit that
board (`q`), after which Ops retries. No command startup scans panes or processes for
former boards (#2342).

## Team preset

The `team` preset retains the same `Layout`/`Board::preset` and config readers for
CLI listing: crew states, pending-first order, member/state/task/pr/model values,
a pending line, a 60-second `github-pr` field and a 30-minute observed-age default.
These do not compose a second interactive grid. Model reads the existing session
projection and providers stay on the shared fetcher path.

## Views (pane arrangements)

The new interactive shell uses `config::workspace` preferences Auto/Side/Stack,
room-UUID pins and optional hidden room UUIDs, written by the existing
Config CAS owner. Authored sort/reverse values are cold defaults; interactive
sort, filters, committed search, authored chart Range/Each, quota-window choice and pane scroll belong to the selected
[view-state layer](board.md#workspace-and-retained-identity). `workspace.hidden` is a unique array of at most 1,024 canonical
lowercase room UUIDs. An absent key means none; an empty saved set removes the key.
Unknown/deleted room IDs remain stored without pruning and do not inflate the
live hidden count. Pins remain independent. Session launch scope/reveal belongs
to App and ephemeral reload carry, never Config or retained view hints.
`workspace.overview_sort` names an existing identity sort field;
`workspace.overview_reverse` reverses its initial direction; optional
`workspace.agent_pins` retains up to 1,024 canonical identity UUIDs. Pinned agents
precede the regular flat Agents order and appear in the Agents Pinned tree.
`workspace.digest_window` selects 24h (default), 7d or lifetime (shown as all time)
for every Digest estimate surface, through the same format-preserving Config CAS;
`workspace.savings_unit` (`tokens` default, `percent`, `cost`) is the sibling key on the
same write. `SavingsUnit::shown` reads percent as tokens for lifetime, and `next` skips
it there. The `savings-unit` action ships without a default key so that no frozen
binding table moves; a click on a painted estimate (`Choice::SavingsUnit`) and the Actions
menu reach it, and the footer hint appears only while a frame painted an estimate.
Older Ops builds reject that unknown key; alpha publication is automatic, but
installation requires an explicit upgrade. One effective action table resolves defaults/global/host/section overrides for
dispatch, footer and help. Retired pane/view/tab keys get located notices; authored
bytes remain intact. The shell has no legacy mode or factory-view picker.

Workspace settings → Panels previews Agents and Digest independently, with
show/hide, order, a separate Conversation/Details switch and Reset to default.
Save uses the existing asynchronous workspace preference CAS; a changed source
retains the draft and error. Cancel drops the preview. Closing a pending save
neither cancels its admitted write nor applies a late result to a reopened picker.

`workspace.overview_panels` stores `order`, `hidden` and `conversation` over IDs
`summary`, `digest`, `subscriptions`, `usage`, `monitor`, `exchanges`.
Agents exposes only Usage, inline quota, the list and the side column in
its picker (a hidden list collapses the list rows, a hidden side column the replies); fixed positions ignore retained vertical
order. Retired Summary and Recent controls remain absent. Usage has a fixed full-width header position and `subscriptions`
controls quota cells inside its provider rows. Their order entries are retained
in the stored complete ID set, but the picker identifies them as fixed and refuses
move actions. Digest retains independently movable panels.
Authored `overview`, `overview-view` and `overview-filter` actions receive a located
retirement notice. Their text stays intact and is not applied; the `agents-filter`
(`f`) and `talk-measure` (`t`) actions replace them.

The explicit `all-timers` action opens the all-room read context and is the
default `c` when nothing is selected. Authored `timers` keeps the selected scope, and compatible global,
host and section overrides retain their normal precedence. The preview advertises
only a key that actually resolves to `all-timers`.
The flat list and reader factory defaults resolve Enter to `reply`. The internal
reader context inherits reader scrolling before the existing authored
key merge. Authored global, host and section overrides keep their meaning;
Agents/Digest reader defaults remain independent. The filled input advertises the
effective admitted Reply/Talk start key, and pointer focus uses that same action.
`workspace.digest_panels` stores `hidden` and `details` over IDs
`subscriptions`, `chart`, `list`; Digest sections have fixed positions. Retired
`order` and `status`/`usage`/`report` entries produce located migration notices
and remain authored, including on panel Save and the Details toggle. Save never
creates `order`. Unknown IDs and duplicate IDs remain invalid; hidden is a unique
subset. Keep list or Details visible. Absence and omitted fields select defaults.
Save preserves table or inline-table form, comments and unrelated values; only
Reset removes the table. Invalid panel values yield a located notice and default
policy; other invalid config still fails admission. IDs are persisted: older
builds still reject IDs they do not know.

Independent `view` CLI configuration, workflow/layout readers and offline markup
validation retain their own command contracts. A CLI view key changes no roster,
state, provider, reminder or sampling policy and no longer selects an interactive
arrangement. `set_view`/`remove_view` preserve unrelated/custom keys and comments,
refuse concurrent file changes and never write during inspection.

Authored `[squad.<name>]` layout/rows/columns/sections remain accepted through their
existing parsers and continue shaping public CLI documents and lists. Their old
grid, wrapping and pending-continuation geometry no longer shapes the workspace;
located migration notices explain this while preserving the file. Invalid values
still fail admission. Section action bindings remain effective on the acquired
membership target; unsupported legacy actions are reported rather than remapped.

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
- The bindable `theme` action, offered in Workspace settings, opens
  `board::theme_picker`. The session reads its Config at opening and keeps that baseline
  across refreshes. Preview applies the same in-memory edit as CLI set, cached by
  selection and scope, without writing; `App::look` supplies it to every pane and tab. Tab
  switches board/squad scope (the flat list and Common pool have board scope only; a masking squad base is
  named). Pickers open in all-boards scope, even when a squad override masks it;
  the notice directs an intentional this-squad edit. Overlay input cannot operate underlying rows, tabs or panes. Enter calls the
  named edit once; a failed save keeps the draft and notice without retry; Esc drops the
  preview and uses the latest saved view.

## Settings inspection and editing

The comma menu offers workspace layout, Digest estimate windows, Host action (j),
Theme, Token window and Actions. It does not expose the former general
configuration-inspection overlay. Host action reuses the Settings list, editor and
Config CAS writer for Focus agent, directional Swap beside board, or Custom
command. `workspace_keys` supplies the effective action; Config's shared provenance
resolver identifies its global/host/selected-section write owner and preserves
other bindings. App supplies the same recipient-dependent factory action as
dispatch. Retired bindings retain their fallback or unset value; a chosen preset
repairs their authored owner. Preview freezes the binding context and recipient:
a squad uses its lead for host/argv evidence without inheriting the lead's member
section. It fills argv without execution and keeps missing context unavailable.
Preset previews are read-only (`s` saves); Custom keeps typing and Enter save.
Esc cancels; a file conflict retains the unsaved draft/error without retry.
Other keys and nested configuration remain file-owned; `config show` retains
independent values/provenance. Ordinary Settings and Theme openings do not
acquire host-action context.

Workspace preferences use one Config CAS writer: compare the opening values and
file bytes, validate before publication and preserve comments/unrelated keys.
Theme picker preview remains disposable across refresh, names masking squad
settings and offers explicit board/squad scope. Enter saves once; failure retains
the draft and error without retry; Esc restores saved appearance. Global choices
never silently remove a squad override. Token window keeps the shared writer and
scoped override semantics; CLI configuration remains available for explicit edits.

- `config::edit` owns the shared edit policy and a disposable validated Config draft.
  `sq config set KEY VALUE` accepts layout preset, flat split panes/direction/sizes,
  refresh, notes mode, hidden tracks, exact state colors, global tabs order/hide and the
  selected squad's reminder and idle-alert enable/threshold (booleans `true`/`false`,
  thresholds through the `Config::reminders` whole `s`/`m`/`h` validation, 1 m–24 h, or
  `Config::idle_alert`, 5 m–24 h); arrays use JSON. Structural edits of a nested split tree refuse rather than flatten a
  custom or factory tree. Partial flat edits keep the workflow preset and seed missing flat
  split keys from the resolved arrangement. The draft runs the existing area validators,
  then `Config::set_setting` calls only the `Config::write` compare-and-set path; a changed
  file is refused and comments, order and unrelated keys survive. CLI edits touch no roster
  or member metadata, install no provider hooks and grant no extension consent. A reminder
  preview reclassifies only known ages in memory from the board's newest
  `staleness::Snapshot` (disabled previews remove marks; unknown ages stay unknown) and does
  no observation, cache publication or reminder claim; confirmed edits reach the shared
  observer on the ordinary reload.
- `config::idle_alert` owns `[squad.<name>.idle_alert]` (`enabled`, default false in every
  layout; `after`, default 30 m, 5 m–24 h, digit-first like reminders) and
  `Config::idle_alert_squads`, which lists the squads that switched it on from the document alone
  so a squad with it off costs no Core read. `config show` reports both keys with the source
  `default:off`/`default:30m` or the key path. Detection and the notice are
  [`idle_alert`](board-cron.md#idle-alert-scan), not config.
- `board::help` projects navigation, effective bindings and meter explanations into
  `tmt-tui::components::KeyHelp` sections; Action owns binding wording: `description` preserves independent CLI/config
  prose, while `workspace_description` names workspace effects in Help and the
  host-action editor. Settings keep their literal JSON value and source apart from
  presentation prose. Meter input keeps observed roster names (including the lead and
  members omitted from displayed rows) for excluded labels.

## Actions and effects

- `action` parses explicit global, host and section bindings into events and template
  arguments; bad events, actions or field syntax are configuration errors. Config owns
  the workspace defaults and overlays compatible global, host, then section bindings.
  Dispatch, help and footer consume that same table in the current list or Reader scope.
  Member `rename-member` defaults to `r` only for a visible Agents list member, including
  Common pool; the staged form freezes its UUID and literal current name for Core's
  `identity.rename` CAS, with explicit review and no automatic retry.
  Default Enter talks for a member and inspects a squad or the Common pool, on every host;
  double-click inspects; list `j` jumps, while Reader `j/k` scroll. The host table
  uses only Core's published `location.host` (`tmux` or an external host); missing or
  unknown location supplies no host override. Retired actions produce located migration
  notices and preserve authored bytes. Arguments are fully resolved before execution;
  missing values refuse rather than constructing a partial action. A
  `[bind.<host>]` table takes the same events and actions as `[bind]`, is parsed apart from
  the flat entries (`Config::host_bindings`), and its name must be a host token
  (`[a-z][a-z0-9-]{0,31}`); an unknown host never matches. One parser/dispatcher owns
  `toggle <pane>...`.
- `effects` holds the row actions behind plain `jump`, `open` and `copy` and the board.
  `template` fills `{field}` placeholders into one value and refuses empty values. Built-in
  names win over a squad field of the same name; `{host}`, `{session}` and `{window}` come
  only from the pane `location` that `join_presence` copies from `ls --json` (never parsed
  from `target`), so a host that reports no session or window refuses the action.
  Programs run as argv, never through a shell: the top-level `opener` and `clipboard`
  arrays, or the system opener. An opener starts in its own process group with null stdio
  and a reaper thread. `run` fills one argv element per template (refusing a value that
  would start an argument with `-`) and starts it the same way. Reserved `{board_pane}` for Run comes only from public
  `whoami --json`: the effect lane reads it at dispatch, while the Host action
  opening reads a nullable sample for preview. Agent fields cannot supply it,
  and paint/input never read Core. `{{` and `}}` are literal braces; for example
  `{{left-of}}` stays a literal program argument, never a row placeholder.
- `swap <left|right|up|down>` is an Action beside Jump, resolved through the same
  effective binding table and described by Help/footer. Its effect calls Core
  `focus <selected UUID> --into <direction>`; Core owns the host, neighbour,
  server and pane validation. Ops admits the published exchange/already-present
  JSON shapes, preserves typed refusals and never claims a swap from Started.
  Standard exchange has no home/history store: the previous neighbour moves to
  the selected agent's old slot; selecting the previous neighbour agent swaps
  it back beside the board. No pane is killed and the board keeps focus;
  unsupported hosts and missing panes/neighbours refuse in Core. Custom Run
  remains detached.
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
  with operands after `--`; Common pool Talk omits the room rather than inventing one.
  Annotations require a squad and use a talk tagged `[<squad> · <row>]`; answers use one
  `tmt answer <member> --request <id>` (core selects and proves the request, no receipt
  passes through Squad); nothing acknowledges. Squad has no talk, reply, replies or annotate
  command: conversation is Core's, and configured annotation actions and response quotes
  use the board composer and in-process requests, never a command or hidden `__` entry.
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
calls `skills.install` as owner `ops` (its `OWNER` constant); `rm` calls `skills.remove` with the playbook's name,
so the lead skill and `tmt extension rm squad` are unaffected. Squad never writes a provider
directory and never executes a playbook. The lead skill source is embedded only in the squad
executable, never in the core skill bundle.

## Shared dispatch intents

`send::dispatch` owns the frozen intent, private journal and acceptance recovery
used by status announcements. Status drafts retain the opening actor
and room/member context; `membership::status_update` revalidates that authority
before constructing an intent. Public dispatch deduplicates recipient UUIDs. `send::new_operation` supplies a fresh UUID for explicit sends, shared with
manual cron sends. Before dispatch, a private 0600 intent under
`board-dispatches/<operation>.json` beside Squad configuration is synced. Confirmed
acceptance removes it; uncertain acceptance retains it for manual inspection.
One `dispatch.show` read can recover lost output. There is no automatic resend or wake retry. An explicit status notification retry first recovers uncertain acceptance; after the owned invocation has closed and a definitive missing receipt, it may create only the same frozen operation/intent again. Confirmed acceptance is never created again. Feedback distinguishes each recipient's queued or
unavailable acceptance without claiming delivery or processing.

The per-layer view-state controls store the list choice as `agentsFilter`
(`all`, `agents` or `pinned`). A file written with the removed `overviewView` or
`overviewFilter` keeps its committed query, sort and scroll controls and ignores those
keys. A present invalid choice is refused by the existing control decoder. Config remains unchanged.
