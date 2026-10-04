# Refresh worker and token meter

## Refresh worker

- One refresh thread loads snapshots off the input loop, collapsing queued requests, so keys act
  on painted data. Results carry their generation and cannot replace a newer view. The worker
  owns one never-reset stop flag per generation; preemption and shutdown set it while the
  generation counter still fences events.
- Cancellation kills and reaps the child process group in the shared `tmt-invoke` bounded process
  owner, without changing ordinary command deadlines or output bounds. Shutdown cancels the
  in-flight core read, disconnects requests and joins after terminal restoration.
- Timer: the input loop requests a reload at the shown squad's `refresh` interval, which each
  snapshot carries, so a squad that failed to load retries at the default. The timer always runs.
- Change detection: between requests the worker checks `board::changes` every `CHECK_EVERY` (1 s)
  using core's `changes.cursor` (public extension API method) and `squad.toml`'s modification time
  and length. When either moved since the stamp taken just before the last load, it reloads that
  squad early unless its `refresh` is off. A failed read is never a change, and
  `API_INPUT_INVALID` (a core without the method) stops cursor reads for the session, leaving the
  file check and the interval. A field-provider save moves the cache directory's stamp, which
  `board::changes` also watches.
- Providers: the board hands each load's members to one fetcher thread that runs due provider
  work off the paint path and again at the shortest `every`.
- Tab attention: the refresh computes attention for the shown squad from its document and
  publishes that view first. The same worker then computes every other squad's attention from a
  roster-only document (one `rooms.roster` read each plus one `inbox` read shared by all, no `ls`).
  Previous tab attention stays visible until that generation's update arrives, and a newer switch
  preempts this lower-priority work. The cross-squad leads/all views read the rosters needed for
  their own rows before publication.
- Priorities: full loads outrank selection jobs (detail notebook) and usage-only reads.

## Token window meter

The meter shows completed-request token totals for the visited squad. Input plus
output counts cached input once. Mixed providers sum reported token units, not
cost or text volume. `board::rate` owns evidence, `board::meter` presentation and
`App` the runtime selected window.

- Acquisition: `board::rate::Input` captures roster UUIDs and public `resume`
  values before section shaping. On named/HOME entry, the existing cancellable
  worker batches `consumption.history` for at most 32 UUIDs per call, requesting
  one longest window capped at the API's 1 h limit. The transient response is
  validated and consumed once into the existing Rate rings; it is not retained
  as a second history store. Ordinary reloads do not acquire history.
- Seeding: closed deltas replace the authoritative recent region on re-entry,
  preserving older board observations in the same owner. The included `latest`
  driver/session/epoch/sequence and cumulative counters, rather than `throughMs`,
  anchor live subtraction. Shorter windows overlap and are never added again.
  Whole rollups crossing a shorter window boundary are excluded rather than
  prorated. Missing or failed acquisition means unavailable evidence, not zero,
  and cannot bridge current cumulative counters.
- Live sampling: a named-squad load carries public counters; idle reads use only
  `ls --room --json` for those UUIDs on separate 5–10 second deadlines, without
  providers, notes or staleness publication. Full loads take priority; usage
  shares generation cancellation and shutdown ownership. `App` accepts counter
  receipts at the configured cadence and never treats cached tabs as fresh
  evidence. It retains meters for visited squads, pruned against visible/hidden
  tabs; leaving a tab closes continuity. Meter state is separate from folds/panes.
- Evidence: each reporting UUID owns a bounded ring of 5 s buckets sized by the
  longest configured window, up to 24 h. Receipt time advances monotonically from
  a UTC anchor. `Rate` validates cumulative input/output/cache-subset,
  driver/session/epoch and sequence/time order. Missing, invalid, gap, decrease,
  new-session or recovery evidence establishes a baseline without invented
  tokens. Removing a roster UUID drops its history. Failed reads close continuity
  and mark gaps after two sampling periods. Provider `observedAtMs` is order
  evidence, not a heartbeat. Retained model attribution uses the current observed
  session model and is best effort.
- Configuration: team preset, global `[board.token_rate]` and per-squad keys layer
  through `config::TokenRate`; Team alone defaults on. `[board] tok` and per-squad
  `board.tok` select three distinct ascending whole m/h durations from 1m through
  24h, default 1m/5m/60m. Both layers validate even when masked. The config reader
  reports the winning setting path and preserves explicit column titles.
- Coverage: any covered reading stays numeric, including measured zero. Partial
  coverage, windows longer than available evidence and unreported members prefix
  the known total with `~`. Unreported identities contribute no tokens. A baseline
  alone is not measured zero. Without coverage, the meter retains its active
  window label and `–`, plus one dim `no usage reported yet` line; member cells
  also show `–`. Built-in all/leads tabs omit the named-squad summary meter.
- Window selection: bindable `token-window` (`w` in both host presets, outside text
  inputs) cycles configured windows and posts the label in the existing board
  notice, including without data or when the summary cannot fit. Whole hours use
  `h`, so 60m displays as 1h. Selection is runtime state, never a config write.
- Row projection: `App::project_usage` derives a board-only document from immutable
  public status using the accepted meters for model and three totals. Repeated
  section rows share one UUID history; changed values invalidate only existing
  grid/cell derivations. Retained views keep their owning values while another
  squad loads. TEAM and crew preset TOML own column placement/priorities;
  `ColumnSource` resolves `usage.w1`–`usage.w3` by window index. One-shot
  `ls` has no history: JSON keeps descriptors without usage values and text skips
  board-only columns. Policy changes start fresh observations. The default grid
  reserves a 20-cell TASK minimum. Sampling off hides usage tracks through
  `Rows.hidden_columns` while retaining MODEL. With sampling on, authored
  window-column priority ranks keep the active window longest: inactive windows
  step aside before PR, then model, then the active window. `Rows.select_window`
  reorders those ranks; App applies it and invalidates the grid immediately on
  selection, even when usage values are unchanged. Model width follows content
  up to 14 cells; paint has no meter-specific fitting branch.
- HOME: separate typed `View.home_rate` templates supply each squad roster. While
  HOME is open and any squad enables observation, one global public `ls --json`
  per sampling cycle is indexed once and joined into the same retained meters.
  Other tabs never schedule that read. Switching cancels the existing generation
  and closes continuity. `App::home_usage` exposes the current lead model,
  configured windows, raw lead/squad readings and the lead's share of the longest
  window (normally 1h). Missing stays absent; zero stays zero; partial flags
  propagate to share; a zero denominator has no share. Consumers format this
  projection without sampling or recomputing totals.
- Presentation: cubic digits count for at most 600 ms at 250 ms frame spacing;
  retargeting starts from displayed digits, and window switches/reduced motion
  settle immediately. Eight bucket-aligned bars derive from the same rings: blank
  means no evidence, ▁ measured zero, and ▂–█ nonzero. A right-aligned
  number/unit/window/trend group uses a seven-cell maximum number region. It
  drops the trend, then shortens the unit. It preserves the selected label by
  clipping lead/attention text when necessary; it hides only when the terminal
  cannot fit the compact meter itself. Its status row stays reserved while enabled
  so coverage transitions do not move member rows. The new unavailable-state
  text uses the admitted `tmt-tui` strip painter. Help explains completed-request
  window totals, best-effort/excluded/partial semantics, effective switch keys and
  cycle order, and the global/per-squad `tok` setting. The normal cached renderer and ratatui
  diff own output; meter-only ticks stay in the meter band. No parallel paint,
  worker, core dependency, provider-file read or Squad persistence is introduced.
