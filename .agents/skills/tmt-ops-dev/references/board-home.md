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
one snapshot. Pool remains first; pinned squads precede the selected sort order.
A collapsed squad keeps its worst observed child state, prioritizing attention.
Refresh, sorting, pinning and search preserve the anchor; a hidden or removed
anchor disables actions rather than selecting its neighbour. The session-local
Agents filter selects All, Needs you or Pinned without saving
preferences. All keeps Common pool first; Needs you retains qualifying groups and
incomplete evidence; Pinned selects real pinned squads. Search keeps matching
ancestors and temporarily reveals matching children without changing saved
expansion. Group totals remain unfiltered room totals. Empty filtered results are
non-interactive; retained hidden targets grant no action authority.

## Shell, geometry and input

`board::shell` owns Overview, Agents and Digest selection. `overview::monitor`
projects one identity row through the admitted workspace roster. Its single cache
also holds roster-intersected typed Digest counts and admitted recent exchanges;
acquisition, labels, meter/job updates and admitted exchange reads invalidate it.
`workspace::columns` owns shared full values and TUI-solved tracks; Overview
captions Digest as MODE, while `sort` orders known values with missing observations
last in both directions. Config owns persisted monitor order; absent order retains
needs-you priority. Saved agent UUID pins form the first group with a decorative
divider; the current sort applies within each group. Hidden or vanished pins do
not retarget selection. Header clicks sort; Overview has no supplied sort keys,
but authored `sort` / `reverse-sort` bindings remain admitted. The session-local All/Needs you filter preserves a hidden
selected UUID. Only viewport rows build display text and hits.
`overview::composition` places summary, Digest, quota, chart, monitor and exchange
panels independently. At 140 columns and 12 body rows, Overview reserves 60% for
these panels, one separator and the remainder for the existing conversation;
Agents has its own 47% list placement (75 cells at 160 columns), with Auto stacking
below 120 columns. Its shared title, filter and snapshots bands precede both panes;
optional bands yield on short screens to preserve the conversation's usable budget.
Digest retains its existing placement. Recent exchanges are bounded latest lead exchanges
plus own-result metadata, not an event log: source times sort newest first,
untimed rows last, and failed or unacquired pages say unavailable. Successful empty
pages say No observed recent exchanges. Summaries label incomplete totals and
unknown modes/counts instead of inferring Off or zero.
The two-line Overview legend omits Digest, Manage, Create, Remove and Details;
Help and the Actions menu retain these effective bindings.
`overview::detail` discloses full values and observed membership scope without
adding command authority. The Summary timer control opens the existing all-rooms
list without retargeting selection. With no selected target, the Overview Timers
binding does the same; hidden/missing selected scopes still refuse to widen.
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
legend; a draft-close notice keeps its footer priority until the next key. Tab moves between the tree and conversation; arrows expand/collapse
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
and Digest share the same admitted per-window quota bars: remaining percent,
labelled reset and measured weekly pace. Quota width >= 90 pairs providers;
narrower areas stack them. Paired rows omit reset/pace when they do not fit
(terminal 160/100); stacked rows retain them at 80/60, while Quota Details always
retain full values. Complete source timestamps and unknown
evidence stay in on-demand details; paint and input perform no acquisition.

`board::overview::chart::host` owns one session Each/Range selection for Overview
and Digest. `u` resolves through effective bindings in their list scopes; Agents,
Reader, search and composers retain their own input. Arrows draft a choice, Enter
applies or inspects, Tab selects Each/Range/buckets and Esc returns to the list.
Invalid pairs preserve both controls. Direct control and bucket hits come from
admitted, clipped nodes; resize and source replacement revoke their authority.
If there is no positive observed count, or placement would leave fewer than six
target rows, the chart collapses to one summary line. Its effective shortcut, summary click or Actions entry opens the
same controls in an opaque body modal; closing never replays input. A body with
no spare target row omits the summary to retain the last usable row. At eight
terminal rows, Overview keeps the table and omits summary/quota context; the
effective filter, Details and usage-control keys remain reachable.

One identity-total series uses the existing Rate rings. Details preserve exact
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
cells/styles/hits/oracle diff is approved by UX and Core. Regeneration follows the
[TUI parity gate](../../tmt-tui/references/development.md#board-parity-baseline).
[development.md](development.md#checks) owns commands and isolated captures.
