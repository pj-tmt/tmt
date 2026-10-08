# HOME board surface

[board.md](board.md) owns frame routing, row composers and shared scroll integration.
The shipped Squad skill owns [HOME controls and appearance](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#home-dashboard).
Acquisition/selection fences belong to [refresh-and-meter.md](refresh-and-meter.md#refresh-worker),
usage evidence to its [meter reference](refresh-and-meter.md#token-window-meter),
and audience effects to [config-and-effects.md](config-and-effects.md#home-lead-sends).

## Model and scene owners

- `board::home::Home` is board-only (`View.home`); acquisition reuses `tab_view` and
  the user-section pipeline. It observes each squad's task/state age under the
  existing staleness lock/policy before/after its roster read; request age comes
  from inbox timestamps, while pending alone provides none. Public aggregate
  documents and `ls --tab all` retain their separate projection.
- HOME bypasses ordinary pane composition. The neutral `view::scene` admits literal templates,
  binds display-ready data, computes TUI geometry and lifts a scratch buffer into
  lines. Section owners are `attention`, `leads`, `rows` (audience/cron), `tiles`
  (squads) and `bar` (summary/usage/keys). Builders report local entry/start/end/x/width
  placements; `home::paint` translates them into one stream, cursor, reveal and
  viewport-clipped hit map. Scene IDs are local ordinals, not names or cursor identity.
- Width steps stay in `tmt-switch`/`hide-below` markup; content fitting stays Rust
  measurement feeding bound branches. `tiles` supplies one full-width squad table
  at every width, in filtered reading order. Padding belongs to row selection;
  headings/gaps have no hit. Member counts align inside the table rather than at
  the terminal edge. Marks exclude leads and assign each member one urgency class;
  unknown/custom states get an unmarked `other` count. Waiting precedence comes
  from `attention::waits_on_you`.

## Runtime projections and identity

- `App::home_usage` supplies observed totals, model attribution and longest-window
  share; `tiles` only formats them. Missing differs from measured zero, partial
  evidence propagates to shares, and disabled sampling supplies no token cells.
  Observations admit token columns/legend; mixed windows label actual durations.
  A sampling lead without totals has one missing mark in the first token column.
- Session-model observations are independent of sampling: a bounded public `ls`
  on refresh seeds retained inputs, and meter receipts can update them. Failed
  model reads remain HOME failed-read evidence without changing aggregate JSON.
  `App::home_lead_model` can supply a model while sampling is disabled; without
  observation the cell is omitted. `source::model_name` shares best-effort family
  formatting with session columns/meter; acquisition preserves the original name.
- `App.selected` reconciles by section/squad/member identity across refresh/search.
  Attention precedes deferred leads, their audience footer, cron and squads. Inline
  input and sent feedback join the stream beneath the selected row, shifting later
  regions together. Tiles expose no member names, task/PR fields or question text;
  composing uses the acquired target and ordinary send revalidation.
  `App::shown_changed` places it when HOME comes on screen: `home_left` (the target
  `go` saved on leaving) if that row still exists, else `place_home_start` (needs-you,
  blocked, leads, squads; never the cron line). A start on a lead also sets
  `home_start`, so the first deferred lead read, which reorders the leads, places the
  cursor once more; `select` and the composer clear it. Squad tabs start at row 0.
- `home::leads` projects deferred lead headings into the shared `view::member_list`
  `Outline` box. Leads and attention members start collapsed. `board::row_detail`
  owns their explicit in-place detail and reply blocks; section builders reserve
  those lines and include their presentation data in cache keys. Every block line
  maps to its parent cursor target, while selection styles only the heading.
  The audience footer remains outside the box. Acquisition and audience effects
  remain with the linked owners.

## Cache invariant and verification

Each section and summary/usage/key strip has a `view::scene::Kept` in the immutable
view: HOME-specific caches live in `Derived.home` (`home::Scenes`), and the shared
boxed-list cache lives in `Derived.member_list`. `scene::Key` contains bound data, width,
look and selected block. Ages, cron text, input reservation, sent feedback, search
and usage must reach bound data before key comparison; decoration may read only
look and selected block. Local placement lets a shifted section reuse its block.
Selection repaints the departed/entered sections; width/look changes rebuild affected
scenes, and a new snapshot starts empty. The key strip decorates keys and labels through the shared footer painter;
its bound strings include the conditional reply-reader hint.

`home/tests/cache.rs` compares retained frames with fresh frames. `home/tests/oracle.rs`
records whole-frame cells/styles/hits around `sm`, `md` and `lg` boundaries;
regeneration follows the [parity approval gate](../../tmt-tui/references/development.md#board-parity-baseline).
`home/tests` and `home/tiles/tests` cover projection, selection, reveal and clipped
hits; [development.md](development.md#checks) owns commands and isolated captures.
