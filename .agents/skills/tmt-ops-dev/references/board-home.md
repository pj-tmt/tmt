# Ops workspace surface

[board.md](board.md) owns frame routing, the WP1 composer and shared scroll integration.
The shipped Squad skill owns [workspace controls](../../../../extensions/tmt-ops/skills/tmt-ops/SKILL.md#home-dashboard).
Acquisition fences and usage evidence belong to [refresh-and-meter.md](refresh-and-meter.md);
validated send effects belong to [config-and-effects.md](config-and-effects.md).

## Acquisition and model ownership

`board::home::Home` retains the existing attention, sampled-history and provider
limit projections. Its `workspace::observe::OpsWorkspaceSnapshot` is built from the
same `tab_view::Acquired` documents, room records and global identity inventory;
painting and input perform no Core reads. Public aggregate documents and
`ls --tab all` keep their independent projection.

The snapshot contains admitted source rows, room UUIDs, identity UUIDs, per-squad
states and effective Config tables. A malformed, failed or incomplete roster is
not proof of zero memberships. Common pool includes only identities positively
known to have no Ops squad membership; other Core rooms do not count as squads.
A lead remains one identity in its squad count. Repeated memberships do not
inflate the unique identity total. Unknown, stale, partial and measured zero stay
distinct. Per-squad rank/state never becomes a global identity state.

`workspace::OpsWorkspace` owns expansion, filtering and the exact selected
`OpsTarget`: pool, room UUID, or room/identity UUID pair. It derives the tree from
one snapshot. Pool remains first outside a launch scope; pinned squads precede the
selected sort order. Session visibility holds resolved launch room UUIDs and the
explicit hidden-room reveal. Scope wins saved hiding. Hidden memberships never
become Common pool members, and search never reveals hidden squads. Saved hiding
filters Agents and Overview; launch scope applies across destinations. The hidden band reports UUID-deduplicated waiting/blocked evidence from hidden
rooms in the existing state roles, cached when preferences configure the snapshot;
it omits absent attention and drops these counts after session reveal. The
acquired roster and global shell attention remain unchanged.
A collapsed squad keeps its worst observed child state, prioritizing attention.
Refresh, sorting and pinning preserve the anchor; a hidden or removed
anchor disables actions rather than selecting its neighbour. A confirmed explicit
Hide of the currently visible squad intentionally selects the next visible row;
a failed save or unrelated preference change cannot transfer that authority. The session-local
Agents filter selects All or Pinned without saving preferences. All keeps Common
pool first; Pinned selects real pinned squads. Retained `needs-you` view state
loads as All; an authored `agents-filter needs-you` binding gets a located
retirement notice without rewriting its text or falling through to a default key.
Waiting agents remain visible through their state marks. Search keeps matching
ancestors and temporarily reveals matching children without changing saved
expansion. Group totals remain unfiltered room totals. Empty filtered results are
non-interactive; retained hidden targets grant no action authority.

Overview and Agents share the always-visible search field after FilterBar chips.
`/` edits the retained query; typing filters live, with bold-only matches. Up/Down
move through filtered rows while typing; Enter keeps the filter and returns list
keys; Escape clears from typing or applied state; empty Backspace ends typing.
The filled field keeps one 34-cell slot across idle, typing and applied states,
clipped only by available width. Each query edit selects the first visible match;
Up/Down then move among its matches. Clearing restores the pre-filter selection
when it still exists. No matches leave the previous pane visible as a read-only
preview, without hidden-row actions, revealing hidden rooms or bypassing scope
or the Pinned filter.
Counts belong to section rules, not FilterBar. Overview applies the query to held
Timers before the three-row preview limit, matching displayed ID, squad, schedule
or purpose. Its rule counts all matching jobs against the unsearched observation;
stale and unavailable observations stay explicit. Transient footer feedback keeps
key hints and uses a right-hand neutral filled chip; only a failure mark is red.
App owns its message and failure/expiry metadata; the existing UI loop expires it
after four seconds, and the next key dismisses it. Notices resolve identity names
from held observations, never perform acquisition or display raw UUIDs. Persistent
migration and acquisition failures retain their existing footer priority.

## Shell, geometry and input

`board::shell` owns Overview, Agents and Digest selection. Overview uses one
switchable Squads/Agents list. The shared FilterBar paints mode chips and the
in-place search field; section rules own right-aligned counts. `v` switches the
view through the effective binding table; active searches show matching squads or
agents out of that mode's admitted unsearched total. Agents counts deduplicate
identities across memberships and include collapsed matching children. Explicit
authored keys win; App retains the only search query and editing state.
`overview::monitor` caches canonical rows and the scoped numeric summary; acquired
roster, preferences, label and meter/job updates invalidate it. Waiting membership
occurrences are admitted before UUID deduplication, so an unobserved or nonwaiting
room cannot mask observed waiting in another eligible room. Global attention never
supplies missing room evidence. Digest retains its own projection over the same
acquisition. Shell attention and provider quota remain global.

Squads use lead-first, UUID-deduplicated team strips, the existing HOUR readings
and per-room sum for TOK 1h, and NOW from pending questions before tasks. Missing
readings remain unavailable and incomplete sums stay partial. The Agents view
contains all admitted identities, with waiting agents first: proven oldest request
first, unknown request times last, then name/UUID. Authored sort operates within
waiting/nonwaiting groups in Agents. Squads keeps TOK 1h ordering; its sort hints
are hidden, and an authored sort key explains how to switch to Agents without
changing that view's retained order. No waiting duration is rendered, and blocked state alone
does not manufacture waiting. Pins retain their existing owners.

Each painted row grants only its current target and clipped hit. Selected agents
retain their acquired membership; a selected squad can address only its currently
observed lead in that room. Missing lead or row evidence refuses a conversation;
there is no neighboring-target fallback. A deliberate view switch selects its first
admitted row and starts at the top; source refresh or filtering keeps hidden/missing
anchors without retargeting. The primary list has one scroll owner and a neutral
`↓ N more` overflow caption only when rows exceed the space above the footer.
The list uses that full space; later lower sections reduce it through placement.
The waiting-first count note appears only for a positive count. Short viewports
reveal the selected target. Only the
footer advertises effective keys; Help and Actions retain other admitted actions.
Overview's default Timers key is the explicit `all-timers` action. The authored
`timers` action retains selected-scope admission; hidden or missing selected scopes
still refuse to widen.

Overview Usage is a fixed full-width band reserved once before body placement.
Panels exposes Usage, its inline quota, Squads / Agents, and Conversation;
retired Summary, Recent exchanges and table controls are absent. Overview section
positions are fixed, while Digest keeps its independent ordered panels and Details
switch. Overview also exposes its retained Digest toggle. Timers are
a fixed overview module. Persisted Config IDs and compare-and-save semantics remain with
[config-and-effects](config-and-effects.md#views-pane-arrangements); unrelated
preferences are retained. Hidden surfaces have no stale hits or clocks. Only
explicit message focus reveals a configured-off Overview conversation; Escape
keeps the UUID draft. Hidden Digest Status still exposes failed/loading/stale
observations.

The narrow lower band places shared timer rows beside key/value, joined by
divider. Timer rows show at most three soonest jobs from the acquired all-room cron
snapshot, deduplicated by room/job identity; the more control opens that same
all-room list. Its key hint resolves only an effective `all-timers` binding.
A scoped, searched or hidden roster labels these global facts in the Timers title.
Paused rows are dim. A failed read retains dim stale rows with unknown countdowns;
a missing clock does not become a promised fire time. Countdown formatting uses
the acquired read instant, with no new wall clock, scan or service read in paint.
Observed empty and unavailable timer sources use distinct empty-state sentences.

Key/value uses the existing scoped numeric Summary and supplier estimate/price-table
projection. Held and mode counts retain unknown/partial distinctions; values align
right. Context and list-price quote rows use the existing unit/coverage formatter,
independent of the shared savings-unit preference. An unreported cost is a dash;
reported quotes retain their distinct coverage and list-price assumption. Stale
supplier data does not become current zero. Narrow layouts stack these modules
when their named minimum widths cannot fit. Placement reserves their space below
the primary list. Filter bar, section rule and column header remain fixed above the
scrolling data rows; there is still only one list scroll/selection owner.
The wide right column uses `digest_matrix` directly: all scoped identities,
then highest observed HOUR-usage identities, one row per agent limited by section
height. Muted metric headers and right-aligned whole values share one table; the
all row is bold. Its fixed 24h estimate columns consume matching held supplier
evidence; another acquired window
stays unavailable under those captions. Ratios and price-version conflicts keep
the supplier's existing aggregation policy. There is no wide key/value copy.

At wide widths, the divider joins the left Squads/Agents column to the selected
row's `exchange_view`, three-row filled `input_field`, Timers and Digest matrix.
The exchange keeps sender/recipient metadata separate from the grouped body gutter and reply
name. Its measured wrapped content grows top-down without a fixed minimum or cap.
The filled input follows the final observed group with one blank row above; only
overflow pins it to the bottom, with an earlier-content count and the effective
reader paging key. That hint yields while the editor owns typing. Read-only or
unadmitted inputs reserve no empty editor band.
The existing exact-request reader supplies the selected incoming question;
a preview is not a response attachment. The same exact-question helper supplies
answers after a compact editor moves to Agents. Confirmed answers join the
existing bounded session feedback through one `tmt answer` worker call; failed
answers keep their draft. A new question cannot discard an unsettled send, and a
note to a different UUID cannot borrow the selected agent's question. The fill keeps a one-cell column gutter on both sides; text is inset two cells
and the right hint ends one cell before the fill edge. The same geometry owns
editor cursors and send hits. The input has no section rule or frame, uses the
existing UUID draft/editor/quote owners, and publishes only its clipped input and
send geometry. Overview list and reader defaults use Reply on Enter; authored
bindings merge afterward without a rewrite. Paint, action admission and host-key
provenance share the selected exact membership or the squad's observed current lead.
Short columns drop optional static
modules before the response and editor. The Talk matrix is absent until its data is shipped.

Shared `top_bar` and `tab_strip` own shell paint; the host supplies held counts
and publishes whole tab hits. Only nonzero waiting/blocked glyphs use state colour.
Attention keeps priority over a truncated named-layer caption.

`workspace::render` retains the Agents tree and squad detail. Conversation, editor,
drafts, reply reader and Send keep their existing owners. Digest projects the
supplier-owned status snapshot and UUID-deduplicated member list through
`digest_dashboard`; the canonical roster owns membership and action admission.
Agents and Digest use the selected squad's appearance and contextual detail. Auto/Side/Stack is an Ops preference,
not a second editor or a revival of factory board views. An explicit message
focuses the selected agent's conversation: compact Overview opens Agents, while
wide Overview retains its shared pane. A retained Reader moves to Agents when
Overview can no longer fit its conversation; refresh/resize never converts its
scroll keys into list/Host actions. Logical placement survives clearing pointer
rectangles. With fewer than three content rows, the table omits its header and
keeps usable target rows.

The retired HOME attention/audience/cron/tile body painters and their caches are
removed, including their cursor targets and held-row feedback payload.
`home::paint` retains the shared summary, usage and quota strips. Aggregate usage
and counter keys derive stable source order directly from acquired squads;
search admission and UUID evidence precedence do not depend on tree indices. Cache equality tests render
actual workspace frames, including pointer hits and composer geometry, rather
than maintaining a second HOME renderer. The single decoded HOME oracle and
private workspace capture packet share literal fixtures and the actual frame
entry point; source changes do not authorize frozen regeneration.

Selected-squad detail presents a fixed-width roster preview with lead-first UUID
members, scoped state and supplied task/next text. Common pool remains virtual.
The detail retains supplied usage windows, independent lead model/readings/share,
and their missing/partial markers. It does not expose private lead summary fields.
The shared HOUR observations supply Agents room token totals, deduplicated once per
member (~ when any member is missing/partial, — when none is observed); Common pool
group tokens remain unavailable. Overview and Agents share the truthful TOK 1h and
spaced ◷ CRON captions. Shell online/waiting/blocked aggregates are computed during
snapshot configuration from independent UUID sets; paint reads them on every tab,
qualifying partial/unknown evidence rather than publishing false zero totals.
Agents tables use the shared Scrolls table viewport, reveal and paging rule with
one non-interactive range/total/more line, including when every row fits.

Config projects one effective table per list/Reader scope for dispatch, help and
footer. Reader defaults scroll responses with `j/k`; compatible explicit
bindings keep global/host/section precedence, so hints name the action that will
actually run. Retired notebook-pane actions are reported for explicit migration
without rewriting the authored file or redirecting to a different recipient.
Before a snapshot arrives, the same Config-owned defaults drive the three shell
tabs; no named-squad tab or old-key fallback is admitted. Text, search, completion, overlays and conversation take their keys before shell
shortcuts. A composer with an in-pane key legend suppresses the base list
legend; draft-close feedback uses the same transient Notice chip. Tab moves between the tree and conversation; arrows expand/collapse
squads in the tree. Exact painted rectangles own pointer actions and are cleared
on resize/model replacement and shell changes by `App::invalidate_frame_geometry`.
Offsets and drafts survive; only repaint grants fresh pointer geometry. Single-line
labels escape sanitized display text without promoting names to target identities.
Clipped controls publish no action authority.

`workspace::manage::Draft` freezes typed operations, UUIDs/revisions and observed
timer impact for review. Agents uses the exact selected squad; Overview, Digest
and Common-pool members resolve memberships only from a Complete roster. One
membership supplies context; several require the existing Menu's explicit room
choice; none reports no membership rather than incomplete data. Choice rechecks
the visible member UUID, current membership and Complete room before freezing
its current revision. Partial or failed evidence names the held cause and asks
for refresh. The Menu loses pointer authority with the frame and regains it on
paint; keys retain their selection. The shared modal starts on Cancel and offers a scrollable
read region before Apply. Short terminals use a screen-sized sheet; read position
uses the shared modal's reserved position row. Apply requires current geometry.
Pending blocks replay; partial results remain readable without automatic retry.
The service rechecks the frozen identities and revisions before writes.

In-place reload carries shell choice, expanded room UUIDs, pool expansion and the
exact selected target in resume version 3. A removed target stays missing.
A retired HOME cursor or detail invalidates the reload hint and is consumed
without opening a surface; bounded private-file and symlink checks remain.
Workspace forms and the read-only subscription details modal block reload until
dismissed. Quota row clicks and the Actions menu open the same modal; the
configurable `quota-details` action has no default key. The modal retains the
selected UUID and consumes close before base dispatch. Shared ScrollState owns
its current viewport, wheel/page scrolling and resize clamping. The old
HOME display cache is not admitted as a new shell frame.

## Usage and verification

`App::home_usage` and provider-limit estimates reuse the existing history owner:
missing differs from measured zero, uncovered evidence is partial, sampling off
has no token cells, and model metadata is best effort. Provider pace/trend remains
a derived current-cycle observation, not a persisted field or new acquisition.
See the meter reference for coverage, time fences and trend thresholds. Overview
places Usage across the frame above its body panes. Its provider rows include the
held account quota: neutral 7d/5h bars and remaining percentages; a missing short
window uses the weekly reset countdown. Stale, reset or unreported windows show
an em dash. The Subscriptions panel ID controls these inline quota cells, without
a standalone block; Usage visibility controls the whole section. The quota track
is 36 cells plus a three-cell gap, reduces to percentages below 79 available cells,
and yields to the trend below 59. Quota hits retain the existing full Details.
Digest retains its existing per-window account meters and width rules. Complete
source timestamps and unknown evidence stay in Details; paint and input perform
no acquisition.

`board::overview::chart::host` owns one session Each/Range selection for Overview
and Digest. `u` resolves through effective bindings in their list scopes; Agents,
Reader, search and composers retain their own input. Arrows draft a choice, Enter
applies or inspects, Tab selects Each/Range/buckets and Esc returns to the list.
Invalid pairs preserve both controls. Direct control and bucket hits come from
admitted, clipped nodes; resize and source replacement revoke their authority.
Overview retains provider rows when counts are missing or measured zero. With no
observed token bucket, EmptyState paints one muted status in the trend track;
provider names and available quotas remain, totals/shares are em dashes, and no
axis, blank bar rows or invented unknown provider appears. Measured zero remains
zero. Digest retains its existing summary collapse for nonpositive evidence. If
placement would leave fewer than six target rows, the chart collapses to one
summary line. Its effective shortcut, summary click or Actions entry opens the
same controls in an opaque body modal; closing never replays input. A body with
no spare target row omits the summary to retain the last usable row. At eight
terminal rows, Overview reveals selected targets and omits the full Usage band; the
effective view, Details and usage-control keys remain reachable.

## Shared board components

Shared board modules own visual elements once. `section_rule` owns neutral rules
and right-side counts; `team_strip` owns lead-first glyphs; `state_cell` owns
attention glyph roles; `filter_bar` composes `chip_group` selectors and the idle
search field; `key_hints` owns fitted effective-key hints. `spark_row` owns admitted
trends, numeric headings, totals, shares, optional quota tracks and drawn-width
time axes. `chip_group` owns option padding, selection and clipped option hits;
`empty_state` owns the honest unavailable/empty sentence. `timer_rows` owns bounded
job tracks and their more hint, `key_value` owns aligned metric values, and
`divider` owns joined band separators. Overview adapts acquired
observations and owns effects. These painters perform no acquisition or
view-specific projection, and later Agents placements reuse the same modules.
Only ◆ and ✗ carry attention color; state words and other marks remain neutral.

One identity-total series uses the existing Rate rings. Its header reads
`Usage · last <range> · <total> tok`; two right-aligned selector groups expose the
shipped Each 5/15/30/60m and Range 15/30/60m choices, separated by a muted dot.
Each chip has one space of padding on both sides, with a one-cell gap between
chips and `·` between groups. Selected, hover and disabled states use the
[shared chip styles](../../../../design/cli-style.md#themes); disabled choices
publish no hit. Invalid pairs retain the existing keyboard refusal. No longer
window is implied. Each provider has one neutral-grey spark row, then its observed
total and share. The fixed tracks are name (8), a one-cell gap, spark, two spaces,
total (7, right-aligned), two spaces and share (8, right-aligned, muted). The unknown
row is muted. There is no y-axis or vertical plot. The row below the heading labels TOK, % of all and the two quota percentage
tracks (left) in the muted role. Start/midpoint/now ticks occupy only the drawn
spark width. Received hover or keyboard bucket selection temporarily replaces the
axis row with the same exact-evidence text used by Details; the idle layout keeps
its axis and gains no extra row. Each exact bucket repeats one eighth-height glyph over
`floor(spark_width / bucket_count)` cells, starting immediately after the name
track; leftover cells pad the right. Narrowing
changes only the spark track and never manufactures finer buckets.

Provider shares divide by all observed tokens, including unknown provenance.
Live continuous increments use their verified counter driver. History attributes
only the complete gap-free, non-discontinuous suffix linked to its included latest
driver watermark. Earlier or unproven evidence remains unknown; current model or
identity metadata never relabels history. A mixed receipt slot keeps its proven
portions beside that slot in the same bounded ring owner, removed on replacement.
Driver IDs come from admitted counters, not a built-in name list. Compact local
IDs keep each ring cell at 64 bytes; a driver name can be recycled only after its
last owning cell expires. The interval projection merges keyed portions only after
its existing whole-member UUID de-duplication. Held account IDs add quota-only
rows without claiming token provenance. Numeric totals and shares retain the full
source denominator even when a bucket exceeds the spark quantizer's range.
This adds no separate history ledger or acquisition. Details preserve exact
half-open bounds, observed tokens, missing identities and source durations;
click, keyboard and optional received hover share that formatter. Different
identities' durations are agent-time, not inferred joint elapsed coverage. An
unreported, disabled or absent common-pool source stays unknown in the canonical
roster scope. The chart caches at most twelve interval totals and one admitted
scene; the existing Frame retains the terminal diff. A read-only details modal
captures the inspected evidence, scrolls through the shared component, blocks
reload while open and closes without replaying input.

`board::app::workspace::tests` covers authority, modal lifecycle, pointer geometry,
reload and scoped appearance. `board::workspace::tests` covers pure membership,
counts, selection and sorting. Native service tests exercise public Core and Ops
ports. Capture packets include cells, styles, exact targets and hits across width,
height and colour depth; they are evidence, not regenerated fixtures.

Frozen HOME and TUI parity baselines retain their provenance until a decoded
cells/styles/hits/oracle diff is approved by the UX primary reviewer. Regeneration follows the
[TUI parity gate](../../tmt-tui/references/development.md#board-parity-baseline).
[development.md](development.md#checks) owns commands and isolated captures.

## Shared board components

`top_bar` and `tab_strip` paint the neutral shared shell from held values.
`exchange_view` paints grouped exchange headers and body gutters, `input_field`
paints the filled editor surface, and `digest_matrix` paints observed metrics
across identity columns. The host retains data projection, recipient authority,
editor/session state, effective keys and clipped hits; these painters acquire no
data and perform no effects.
