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
- HOME exchanges: after publishing HOME, `board::home_leads` reads one bounded
  `requests.list` originator results page and one bounded recipient-history page per
  distinct current lead UUID (50 items each). Other tabs schedule no exchange reads.
  Inbox questions reuse the acquired `waitingOnYou` projection; acknowledged finals
  remain results. Ordering uses submission time for replies and preparation time for
  asks. The selected `Kind::Question` is also HOME's ◆ mark source: those questions
  come first, oldest first. Other exchanges follow, newest first. Undated exchanges
  follow dated ones within each group; both use stable lead UUID/squad ties.
  Leads without any exchange follow both groups
  in name/squad/UUID order. Read, reconcile and replacement share this ordering.
  Truncated pages and read failures remain
  explicit evidence; no room scan or unbounded pagination fills gaps. Observations
  belong to the user's UUID and current lead occurrence, and share the refresh
  generation's cancellation. Preview text is sanitized separately from retained bodies.
  The same results page supplies HOME member reply metadata. Expanded row replies
  and the full-reply reader schedule `requests.show` on that same worker through
  the selected-read queue. The user/recipient/request/kind key and selection
  revision fence delivery; detail verifies both participants before displaying
  the retained body. `board::row_detail` drops late bodies for collapsed or removed
  targets and prunes expansion/cache entries when their rows disappear.
- Priorities: full loads outrank selection jobs (detail notebook or row reply) and usage-only reads.

## Token window meter

The meter shows completed-request token totals for the visited squad. Input plus
output counts cached input once. Mixed providers sum reported token units, not
cost or text volume. `board::rate` owns evidence, `board::meter` presentation and
`App` the effective configured window.

- Acquisition: `board::rate::Input` captures roster UUIDs and public `resume`
  values before section shaping. On named/HOME entry, the existing cancellable
  worker publishes the usable roster before acquiring history. It batches
  `consumption.history` for at most 32 UUIDs per call, requesting one longest
  window capped at the API's 1 h limit. The worker may reuse validated seeds
  only with the same successful core change cursor, closed five-second range
  and requested window; missing UUIDs are fetched, failures are not cached, and
  any unknown/changed cursor or time range discards reuse. This bounded worker
  cache does not replace interval freshness for panes or notebooks.
  A new generation reacquires history even when queued tab switches coalesce
  back to the previous name; ordinary same-generation reloads do not acquire it.
- Publication: the same worker serializes roster, history and a fresh public
  usage observation. History never replays the opening snapshot's older counters.
  One global `ls` supplies the post-history observation, replacing the first
  scheduled usage poll. Delivery checks cancellation, displayed owner, room UUID,
  settings and exact member UUIDs before seeding the existing Rate rings. A failed
  observation preserves historical evidence with a gap; it cannot imply zero or
  continuous coverage. The usable roster remains interactive while usage updates.
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
- HOME projection: global `board.tok` windows live in the retained HOME model;
  tiles and member grids keep squad overrides. `App::home_header_usage` joins only
  shown sampling squads from existing roster names and meter UUIDs, with no reads.
  It queries actual durations, so a shorter retained ring remains partial in a
  longer global window. Duplicate UUIDs count once per window: prefer more verified
  bucket evidence, then complete readings and longer spans; equal evidence keeps
  displayed squad order. The global longest-window total is the shared denominator
  for the top member and current-model shares. Missing readings contribute no
  tokens, increment unreported once, and make totals/shares partial; a measured
  zero stays numeric but has no share denominator. Current model attribution is
  best effort. The header painter consumes this typed projection only.
  `home::paint::usage` formats the global windows and shares, using shared model
  family names and grapheme-safe fitting. `view::render_frame` reserves one row
  below counts at the shared MD breakpoint (100) only when observations admit it,
  painted through Strip. The MD row keeps w2/w3 and the top member's share,
  labeled `share <window>:`. LG (140) adds w1, a comma-separated `models` group
  and `N member(s) without data`. The share window is stated once and the top
  member's model is not repeated. No readings means no usage row, including
  warmup; measured zero admits it without a share denominator. Narrow widths,
  disabled sampling and search with no shown sampling squads keep their height.
- Coverage: covered readings stay numeric, including measured zero. Known nonzero
  history deltas also remain numeric lower bounds without continuous coverage;
  zero without coverage stays unavailable. Partial coverage, windows longer than available evidence and unreported members prefix
  the known total with `~`. Unreported identities contribute no tokens. A baseline
  alone is not measured zero. Without usable interval evidence, the meter retains its active
  window label and `–`, plus one dim `no usage reported yet` line; member cells
  also show `–`. Built-in all/leads tabs omit the named-squad summary meter.
- Window selection: comma settings > Token window cycles configured windows;
  `token-window` remains bindable with no default key. Both paths post the label in
  the existing board notice, including without data or when the summary cannot fit. Whole hours use
  `h`, so 60m displays as 1h. These paths and clicks on the painted meter group
  persist the shared `board.token_rate.window` through the existing Config writer.
  Explicit squad overrides still win; failed writes leave the displayed window
  unchanged. Reload/reopen resolves config rather than a session precedence flag.
- Pointer presentation: the existing frame hit collection includes the meter group
  and individual bars. Moved events repaint only on target changes; leaving,
  keyboard operation, resize and modal input clear hover. The whole group uses
  selection background. A hovered bar uses text colour and replaces the fixed
  number/label slot with its total and bucket-end age (`now` below 1m, minutes below 1h, then hours).
  Readouts reuse the token formatter and already retained Rate evidence, with no
  acquisition in paint/input; absent evidence stays distinct from measured zero.
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
  up to 8 cells; paint has no meter-specific fitting branch.
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
  number/unit/window/trend group reserves a fixed 24-cell readout slot when bars
  fit, accommodating both live totals and slice total/age without shifting. It
  drops the trend, then shortens the unit. It preserves the selected label by
  clipping lead/attention text when necessary; it hides only when the terminal
  cannot fit the compact meter itself. Its status row stays reserved while enabled
  so coverage transitions do not move member rows. The new unavailable-state
  text uses the admitted `tmt-tui` strip painter. Help explains completed-request
  window totals, best-effort/excluded/partial semantics, effective switch keys and
  cycle order, and the global/per-squad `tok` setting. The normal cached renderer and ratatui
  diff own output; meter-only ticks stay in the meter band. No parallel paint,
  worker, core dependency, provider-file read or Squad persistence is introduced.
