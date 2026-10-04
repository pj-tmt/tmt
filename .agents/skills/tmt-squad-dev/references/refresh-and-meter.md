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

## Token-rate meter

The meter shows completed-request tokens per second for the visited squad. State lives in
`board::rate` (evidence), `board::meter` (presentation) and `App`.

- Input and sampling: `board::rate::Input` captures the observed roster UUIDs and public `resume`
  values before section shaping. A normal named-squad load carries that input; while idle, the
  worker reads only `ls --room --json` for those UUIDs, without providers, notes/history or
  staleness publication, on separate 5–10 second deadlines on the same worker. Usage events share
  the generation cancellation and shutdown owner. `App` accepts counter receipts at the configured
  cadence and never treats cached tabs as fresh evidence. It keeps one meter per visited named
  squad (pruned against visible/hidden tabs) and owns the runtime selected window; leaving a tab
  closes sampling continuity. Meter state is separate from pane/fold settings.
- Evidence (`board::rate`): validates cumulative input/output/cache-subset, session/driver/epoch
  and sequence/time order. Complete deltas enter a fixed ring of `SLOTS` 720 buckets of
  `SLOT_MS` 5 s by receipt time, so buckets hold observed batches, never reconstructed completion
  times. Missing, invalid, gap, decrease, new-session or recovery evidence establishes a baseline
  without invented tokens. Prior-epoch tokens and gap evidence expire by the selected window.
  Never-reporting members are excluded and named in help; previously reporting members with lost
  evidence make the known sum a lower bound. Failed reads become partial after two sampling
  periods. Provider `observedAt` is order evidence, not a heartbeat. Input plus output counts
  cached input once; mixed providers sum reported token units, not cost or text volume.
- Configuration: `config::TokenRate` layers team preset, global `[board.token_rate]` and per-squad
  keys; only Team defaults on, and built-in all/leads tabs omit the meter. The bindable
  `token-window` action (`w` in both host presets, outside text inputs) cycles the available
  windows (5 s, 1 min, 30 min, 1 h). Default 1 min; 5 s is offered only at exactly 5 s sampling.
  Window cycling is runtime state, never a config write, and a switch advances the worker's
  generation, cancelling superseded core reads.
- Window math: longer windows divide known deltas by the covered span until full, and the label
  discloses that span. Windows with no usable interval hide, including warm-up (10 s for windows
  other than 5 s). Measured zero renders `0`, or `≥0` when reporting coverage is missing.
- Presentation (`board::meter`): cubic counting digits (600 ms, 250 ms frame spacing, exact final
  frame), smooth retargeting and immediate window switches/reduced motion. Eight trend bars derive
  from the ring with slices rounded up to 5 s, so trend spans are 40 s/80 s/30 min/1 h. No evidence
  is blank, measured zero is ▁ and nonzero bars use ▂ through █. One right-aligned
  number/unit/label/trend group uses a seven-cell maximum number region with no padding between
  parts, and the trend keeps its eight slots including empty slices.
- Step-aside order when width is short: drop the trend, then only a full default-1m label, shorten
  `tok/s` to `/s`, then hide, before cutting lead/attention text; covered-span and non-default
  labels persist. The normal cached render and ratatui diff own output (backend-cell tests prove
  meter-only ticks emit inside the meter band), so there is no parallel paint path.
