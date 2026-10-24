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
filters Agents; launch scope applies across destinations. The hidden band reports UUID-deduplicated waiting/blocked evidence from hidden
rooms in the existing state roles, cached when preferences configure the snapshot;
it omits absent attention and drops these counts after session reveal. The
acquired roster and global shell attention remain unchanged.
Attention ordering uses each squad's worst observed child state; its tree strip
shows the admitted members once each, with the observed lead first.
Refresh, sorting and pinning preserve the anchor; a hidden or removed
anchor disables actions rather than selecting its neighbour. A confirmed explicit
Hide of the currently visible squad intentionally selects the next visible row;
a failed save or unrelated preference change cannot transfer that authority. The session-local
Agents filter selects All or Pinned without saving preferences. All keeps Common
pool first; Pinned shows pinned squads with their members, plus individually pinned
agents under their squad or Common pool. Individually pinned children are shown
without changing saved expansion. UUID agent pins and squad pins share the existing
Config preference save lane; hidden/scope and search fences still apply. Retained `needs-you` view state
loads as All; an authored `agents-filter needs-you` binding gets a located
retirement notice without rewriting its text or falling through to a default key.
Waiting agents remain visible through their state marks. Search keeps matching
ancestors and temporarily reveals matching children; clearing expands the
selected occurrence's group when needed to keep that row visible. Group totals remain unfiltered room totals. Empty filtered results are
non-interactive; retained hidden targets grant no action authority.

The Agents list rule carries the FilterBar chips (Squads, Agents, Pinned) and the
always-visible search field; `f` cycles the chips from either pane for any selected
row (a selected squad hands over to its lead when the flat list opens);
authored bindings retain precedence. `p` pins or unpins the selected visible
agent or squad in either tab, and the effective footer reflects its saved state.
Pinned names carry a Muted `▪` mark in both lists. The flat list reserves its two-cell
name slot; the Agents tree uses its leading gutter. An unpinned row keeps the same
name and numeric tracks, and NO_COLOR keeps the glyph.
Selected chips keep their fill while search owns input; hover and activation
eligibility remain independent.
`/` edits the retained query; typing filters live, with bold-only matches. Up/Down
move through filtered rows while typing; opening search moves focus to the list.
Enter keeps the filter and returns list keys; Escape clears and closes from typing
or applied state in one press; empty Backspace ends typing.
The filled field keeps one 34-cell slot across idle, typing and applied states,
clipped only by available width. Each query edit selects the first visible match;
Up/Down then move among its matches. Clearing retains the current exact row and
squad occurrence, expands its group when needed, and scrolls it into view. A
subsequent Escape uses the normal list behavior. No matches leave the previous
pane visible as a read-only preview, without hidden-row actions, revealing hidden
rooms or bypassing scope or the Pinned filter.
While editing, match counts live on section rules. Committing moves the count
into the shared field (`/ foo  3 of 23  ×`); × clears and closes through the same
transition as Escape. The shared field reports distinct clipped clear geometry;
hosts supply admitted counts and current-frame hits. App owns the sole query and
exact selection, with only a tab marker retained for the guarded no-match preview.
Ordinary unfiltered totals remain on section rules. The Measure view applies the query to held
Timers before the three-row preview limit, matching displayed ID, squad, schedule
or purpose. Its rule counts all matching jobs against the unsearched observation;
stale and unavailable observations stay explicit. Transient footer feedback keeps
key hints and uses a right-hand neutral filled chip; only a failure mark is red.
App owns its message and failure/expiry metadata; the existing UI loop expires it
after four seconds, and the next key dismisses it. Notices resolve identity names
from held observations, never perform acquisition or display raw UUIDs. A lane job's
progress notice is held until `SPINNER_DELAY` (`App::set_progress`), so a job that ends
sooner paints nothing; a job the user did not ask to wait on (`Job::shows_progress`) has
none, and a result or failure is shown at once. Persistent
migration and acquisition failures retain their existing footer priority.

`board::unread` projects what answered the user's messages from one page of Core's
originator `answers` view (a submitted final, or a passthrough's captured turn), limited
to 50 with no continuation fetch just to count; it is read by the same Leads job that
reads the results page, which still feeds the reply list. A request is one item, so a
final and a turn never count twice. Request IDs deduplicate repeated room evidence;
squad/global counts deduplicate member UUIDs. Only the originator page supplies this
count, never blended recipient histories or pane output. Optional failed/malformed reads retain the previous page for the same
actor; actor changes discard it. A member's count is `N+ unread` only when
`nextBefore` is present and that pair's marker precedes the page's oldest
`answeredAtMs`. Aggregate `+` is the OR of eligible members' partial flags;
observed zero renders no unread state, even when older replies are possible.

StateCell supplies `● N unread` in place of the glyph and activity words
(working/idle/ended/stale/`?`); the member's task state still follows, as
`● N unread · <task>`, and clips first in a narrow cell. Only the activity form
leaves out a task that repeats its activity word; the unread form keeps it. One owner,
`columns::activity_spans`, paints it in the tree, roster and flat rows.
Only the glyph uses the canonical `unread` role, with neutral count/words and
unchanged text under NO_COLOR. TeamStrip retains lead-first UUID order and admits
whole labels or its existing overflow mark. A visible unread squad reserves a
14-cell TEAM column for a full two-digit lower-bound label and overflow mark;
the shared header and TOK/NOW columns move together. With no visible unread
squad, the existing ten-cell TEAM column remains. Unread members sort first in the flat list
and within Agents tree sibling groups without rewriting raw state or waiting
permission. Marker/page updates invalidate the existing row cache and reconcile
the retained selected UUID. TopBar sums the global workspace independently of
scope, hiding and the active tab, beside its waiting count. At narrow widths the
partial `≥` counts take precedence over the redundant totals-unknown suffix and
named layer caption. Conversation admission and shared marker publication belong
to [the reply owner](board-notebooks.md#replies-and-verification).

Member CRUD uses the existing frozen `workspace::manage` review and ordered
worker effects. Rename keeps Core's identity/name CAS; assignment keeps admitted
room revisions and destination-before-source ordering. `remove-member` is a
copy-only review until Core stop is available: it freezes the displayed name
(the public `rm` operand), offers no mutation or foreground handoff, and warns to
stop and recheck the name first. Displayed CLI equivalents quote only words needing shell protection;
Ops never executes a shell. CRUD modal bounds leave two body cells per edge,
wrap commands, and keep the existing clipped-control and default-Cancel rules.

## Shell, geometry and input

`board::shell` owns Agents and Digest selection, with one inactive destination
anchor kept only for the board session. First ordinary Digest entry selects `all`;
explicit named-view and exec restores retain their UUID. Agents uses one list with three
chips (`AgentsFilter`: Squads tree, Agents flat list, Pinned) and one right column
(`RightTab`: Talk, Measure). The shared FilterBar paints the chips and the
in-place search field on the left rule and `talk_measure` paints its control on the
right rule; the totals sit at the bottom right of the list. `f` and `t` switch them
through the effective binding table; active searches show matching squads or
agents out of that mode's admitted unsearched total. Agents counts deduplicate
identities across memberships and include collapsed matching children, without
building or sorting display rows. Binding resolution shares the existing immutable
row scope and releases it before any input or model change. Explicit authored keys
win; App retains the only search query and editing state.
While focus is in a conversation, the selected agent's row takes the
`conversation` tint and a bold name instead of the plain selection, with state
colours unchanged. Without background colours it is reverse video plus the bold
name (`Look::conversation`, `tree_row::Row::in_conversation`).
`overview::monitor` caches canonical rows and the scoped numeric summary; acquired
roster, preferences, label and meter/job updates invalidate it. Waiting membership
occurrences are admitted before UUID deduplication, so an unobserved or nonwaiting
room cannot mask observed waiting in another eligible room. Global attention never
supplies missing room evidence. Digest retains its own projection over the same
acquisition. Shell attention and provider quota remain global.

Squads use lead-first, UUID-deduplicated team strips, the existing HOUR readings
and per-room sum for TOK 1h. Task/model cells never disclose pending question bodies. Missing
readings remain unavailable and incomplete sums stay partial. The Agents view
contains all admitted identities, with unread agents first, followed by waiting agents: proven oldest request
first, unknown request times last, then name/UUID. Authored sort operates within
unread/waiting/other groups in Agents. Squads keeps TOK 1h ordering; its sort hints
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
Short viewports reveal the selected target. Only the
footer advertises effective keys; Help and Actions retain other admitted actions.
With nothing selected the default Timers key is the explicit `all-timers` action. The authored
`timers` action retains selected-scope admission; hidden or missing selected scopes
still refuse to widen.

Agents Usage is a fixed full-width band reserved once before body placement; it yields to its one-line summary, then to nothing, when the two panes would no longer fit below it.
Panels exposes Usage, its inline quota, the list, and the side column;
retired Summary, Recent exchanges and table controls are absent. Agents section
positions are fixed in both destinations. Digest retains chart/list/details
visibility; obsolete status/usage/report IDs and ordering get located notices
without rewriting authored values. Timers are
a fixed Measure module. Persisted Config IDs and compare-and-save semantics remain with
[config-and-effects](config-and-effects.md#views-pane-arrangements); unrelated
preferences are retained. Hidden surfaces have no stale hits or clocks. Only
explicit message focus reveals a configured-off Agents side column; Escape
keeps the UUID draft. Digest always exposes failed/loading/stale observations;
Exchanges remains reachable when Details is hidden.

The narrow lower band places shared timer rows beside key/value, joined by
divider. Timer rows show at most three soonest jobs from the acquired all-room cron
snapshot, deduplicated by room/job identity; the more control opens that same
all-room list. Its key hint resolves only an effective `all-timers` binding.
A scoped, searched or hidden roster labels these global facts in the Timers title.
Paused rows are dim. A failed read retains dim stale rows with unknown countdowns;
a missing clock does not become a promised fire time. Countdown formatting uses
the acquired read instant, with no new wall clock, scan or service read in paint.
Observed empty and unavailable timer sources use distinct empty-state sentences.

Key/value preserves whole labels, including their time windows, before fitting values;
the narrow cost row drops its redundant list-price note before shortening an amount.
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
all row is bold. Its fixed 1d estimate columns consume matching held supplier
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
send geometry. The flat list and reader defaults use Reply on Enter; authored
bindings merge afterward without a rewrite. Paint, action admission and host-key
provenance share the selected exact membership or the squad's observed current lead.
Short columns drop optional static
modules before the response and editor. The Talk matrix is absent until its data is shipped.

Shared `top_bar` and `tab_strip` own shell paint; the host supplies held counts
and publishes whole tab hits. Only nonzero waiting/blocked glyphs use state colour.
Attention keeps priority over a truncated named-layer caption.

`workspace::render` projects the Agents tree through `tree_row`; `agents_focus`
projects the selected member or squad through the shared board components.
Conversation, editor, drafts, reply reader and Send keep their existing owners. Digest projects the
Core usage/traffic and supplier-owned Digest status through `digest_dashboard`:
range summary, TOK-ranked table with read-only `all`, six-level 24-hour activity,
Exchanges and details. The existing Digest report/query/scope cache also holds the
full admitted exchange ranking: shared `exchange_matrix::by_traffic` orders by
sent plus received, then name and UUID before the shared painter fits at most five
and retains an off-list selection in the last slot. Cursor and viewport changes
reuse those facts. Known-empty traffic and unavailable traffic stay distinct.
Wide frames place Exchanges above details; narrower frames
stack the table and details, with Tab exposing Exchanges. Enter on a real member
opens its Agents Talk pane; `all` grants no recipient or Digest mutation.
The canonical roster owns membership and action admission.
Agents and Digest use the selected squad's appearance and contextual detail. Auto/Side/Stack is an Ops preference,
not a second editor or a revival of factory board views. An explicit message
focuses the selected agent's conversation: a compact Agents tab shows the conversation
pane alone, while a wide one retains both columns. A retained Reader moves to the
conversation pane when the columns can no longer fit; refresh/resize never converts its
reader arrows or paging into list/host actions. Logical placement survives clearing pointer
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

The selected member's focus contains identity, HOUR tokens/trend/squad share,
state, supplier Digest and owned-timer tiles, followed by its resolved link and
actor-only `Exchanges with you`. State-since labels wait for proven evidence.
The selected squad contains its observed current lead, task/pending/next fields,
per-member HOUR spark rows and shares, scoped Timers, and the same exchange/input
owner. A missing lead grants no substitute recipient. Common pool stays virtual.
`identity_line` and `stat_tiles` own their fitted inert elements.
The shared HOUR observations supply member values and room totals, deduplicated
once per member (~ when any member is missing/partial, — when none is observed).
Each mini-trend uses the same admitted meter observation and bounded buckets;
no new history is acquired. Common-pool group tokens remain unavailable.
The tree keeps STATE, TOK 1h, `◷` and the lead marker; Digest/model/runtime belong
in focus. Supplied savings keep their shared tokens/cost/percent choice: one
whole compact estimate in the member DIGEST detail, or in the squad Usage rule
note when it fits. An absent member value leaves its normal Digest detail;
the squad note omits an estimate that does not fit. No extra rows or placeholder
estimates are added; Digest Usage retains the full description and coverage.
`◷` means **timers owned** by that identity in that row's room, paused
included, as an integer. Known zero is muted `0`; unavailable is muted `—`.
Squad timer totals use the same acquired room/ID deduplication. Shell online/waiting/blocked aggregates are computed during
snapshot configuration from independent UUID sets; paint reads them on every tab,
qualifying partial/unknown evidence rather than publishing false zero totals.
Agents tables use the shared Scrolls table viewport, reveal and paging rule with
one non-interactive range/total/more line, including when every row fits.

Config projects one effective table per list/Reader scope for dispatch, help and
footer. In Agents, factory Enter replies to an observed question or messages the
selected member/current lead; factory `j` uses that recipient's guarded host path
from either pane. Reader arrows and paging retain response scrolling, and Tab/
BackTab retain pane movement. Generic Reader retains `j/k` scrolling. Explicit
bindings keep global/host/section precedence, including authored Reader keys;
hints name the action that will actually run. Retired notebook-pane actions are reported for explicit migration
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
timer impact for review. Agents uses the exact selected squad; the flat list, Digest
and Common-pool members resolve memberships only from a Complete roster. One
membership supplies context; several require the existing Menu's explicit room
choice; none reports no membership rather than incomplete data. Choice rechecks
the visible member UUID, current membership and Complete room before freezing
its current revision. Partial or failed evidence names the held cause and asks
for refresh. The Menu loses pointer authority with the frame and regains it on
paint; keys retain their selection. The shared modal starts on Cancel, fits its
content within the existing caps and scrolls overflow in its read region. Its
confirm label names the frozen operation; whole key hints fit after the controls
in Esc, Enter, Tab priority. Short terminals use a screen-sized sheet; read
position uses the shared modal's reserved position row. Confirm requires current geometry.
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

Peek windows light-dismiss (`board/peek.rs`): the Timers list, Help, the subscription and
agent details, and the Actions and mention menus close on a left press outside their painted
sheet, and the same press goes on to what is under it. Each window records its painted sheet
during paint (`PeekAreas`, cleared at the start of every paint), so a stale frame never
dismisses. A wheel or other button outside, a keyboard close, a press inside, and any press
while the composer holds a draft leave the window as it is. Dialogs (forms, Settings, Timer
review, pickers, Recovery, Checklist, the reply reader, Discovery, Usage controls) are not peeks
and still swallow outside presses.

## Usage and verification

`App::home_usage` and provider-limit estimates reuse the existing history owner:
missing differs from measured zero, uncovered evidence is partial, sampling off
has no token cells, and model metadata is best effort. See the
[meter reference](refresh-and-meter.md#token-window-meter) for evidence fences.
Agents places Usage across the frame above its body panes. One shared 7d/5h
choice in the quota heading selects the window for every provider. The selected
quota, rate and forecast are neutral; only the early-runout ◆ uses attention color.
Missing windows, stale balances or insufficient rate evidence remain explicit.
The Subscriptions panel ID controls these inline cells; Usage controls the section.
At 103/148 columns the quota track occupies 37/49 cells plus its three-cell gap.
Smaller widths admit complete fields or leave full values in the existing Details.
The Digest tab owns the Allowance block ([board reference](board.md)).

Agents quota pace uses the last two hours of the actual clock, including the
latest admitted observation and both horizon endpoints, independent of token
Each/Range.
Rate is the change in used percentage divided by actual source elapsed hours;
trend compares it with the preceding two hours. Reset crossings, changed windows
and percentage drops restart the admitted suffix. Fewer than two observations or
less than 30 minutes of spacing means unknown; an unknown prior rate has no arrow.
The worker acquires admitted observations from 48 closed five-minute buckets over
four hours through the existing retained reader, then includes current evidence. Account observations repeat across seats and are
never summed. Only history linked to the verified driver watermark is attributed.
Positive pace projects depletion from the latest source observation; ◆ appears
only when that time precedes reset. Zero or unknown pace invents no depletion.
Footer-only balances remain usable without claiming this API-backed pace. Details
uses the same projection and gives source timestamps and complete reset/depletion
values. Paint and input perform no acquisition.

`board::overview::chart::host` owns one session Each/Range selection for Agents
and Digest. `u` resolves through effective bindings in their list scopes; Agents,
Reader, search and composers retain their own input. Tab selects Range, Each,
quota and buckets in Agents (without quota in Digest). Enter opens a dropdown
or pins a bucket; arrows choose menu entries or select/scroll buckets. Esc closes
the dropdown or unpins and returns to the list. The quota choice is retained per
view layer; changing it does not change token controls or acquire source data.
The independent choices, Auto width rule and readout are defined under
[Shared board components](#shared-board-components). A range transition keeps an
admitted fixed interval or chooses the closest admitted interval; invalid direct
pairs preserve both controls. Control and bucket hits come from admitted,
clipped nodes; resize and source replacement revoke their authority.
Agents retains provider rows when counts are missing or measured zero. With no
observed token bucket, EmptyState paints one muted status in the trend track;
provider names and available quotas remain, totals/shares are em dashes, and no
axis, blank bar rows or invented unknown provider appears. Measured zero remains
zero. Digest opens this existing chart only while its controls have focus. If
placement would leave fewer than six target rows, the chart collapses to one
summary line. Its effective shortcut, summary click or Actions entry focuses the
same controls; Enter opens the anchored dropdown. When the Usage band is omitted
on a short Agents tab, the shortcut opens Range directly. Closing never replays input. A body with
no spare target row omits the summary to retain the last usable row. At eight
terminal rows, Agents reveals selected targets and omits the full Usage band; the
effective view, Details and usage-control keys remain reachable.

## Shared board components

Shared board modules own visual elements once. `section_rule` owns neutral rules
and right-side counts; `team_strip` owns lead-first glyphs; `state_cell` owns
attention glyph roles; `activity` owns the one typed projection of core's
`session.activity`, the STATE words (`Label`) and the agent `Tally`, and
`workspace::columns::activity_spans` owns every member STATE cell's words and
colours; `paint_activity` and `tree_row` report the separate activity/task areas
for their layouts; `filter_bar` composes `chip_group` selectors and the idle
search field; `key_hints` owns fitted effective-key hints. `spark_row` owns admitted
trends, numeric headings, totals, shares, optional quota tracks and drawn-width
time axes. `chip_group` owns option padding, selection and clipped option hits;
`empty_state` owns the honest unavailable/empty sentence. `timer_rows` owns bounded
job tracks and their more hint, `key_value` owns aligned metric values, and
`divider` owns joined band separators. Agents adapts acquired
observations and owns effects. These painters perform no acquisition or
view-specific projection. Agents focus reuses these same modules.
`top_bar` and `tab_strip` paint the neutral shared shell from held values.
The Agents tab reserves a constant two footer rows from 24 terminal rows up, so
focusing the composer never moves its box; panes end directly above them and
sparse exchanges still grow down.
`exchange_view` paints grouped exchange headers and body gutters, `input_field`
paints the filled editor surface, and `digest_matrix` paints observed metrics
across identity columns. The host retains data projection, recipient authority,
editor/session state, effective keys and clipped hits; these painters acquire no
data and perform no effects.
Core's idle alert reaches the board through the same acquisition: `tab_view::roster_documents_with`
runs `idle_alert::overs`/`annotate` on the squad document and its projected rows (Home passes the
identities list as `idle_source`, so no presence is read), `OpsMembershipState::idle_over` and
`OpsSquad::idle_alert` carry the result, and `activity::Label::idle_over` (only while the label
reads idle) makes `columns::activity_spans`, `team_strip::line` and the squad team strip paint
`state_cell::over` (`◷`, `!` under the ASCII glyph set) in the attention colour before the idle word.
The squad row's MODEL / ALERT cell shows `◷ 30m` or `○ off` from that observation;
`idle_alert_editor` owns its clipped hit, effective `i` entry, scoped chooser and disposable
form. It uses shared Modal/ChipGroup/Editor/input styles. The action lane loads Config and
validates both settings before writing changed keys through `set_setting` (After, then enabled).
A second-write refusal rereads the saved setting, keeps the draft and names the partial result.
Its threshold preview uses held activity and published over marks; paint/input acquire nothing
and never dispatch alerts. The catalog supplies Help, Settings and whole footer hints.
The per-squad settings overlay is reached from the Workspace settings menu (`Choice::SquadSettings`, last
after a `Choice::Rule`; absent without a selected squad): `Opening::SquadSettings` opens it in that squad's
context with `Overlay::from_menu`, which keeps a refresh from closing it, returns Esc to the menu, titles it
`Squad settings: <squad>` and scrolls the `idle_alert` group into view once (`reveal_group`). A STATE suffix
that does not fit its cell whole is dropped, never cut (`Label::fit`, `activity_spans`).

Only ◆ and ✗ carry attention color; activity words and other marks remain neutral,
and a member's secondary task state is muted unless it is `blocked` (✗, red). Agent working/idle
counts tally core activity once per identity and count a stale read as unknown;
Squad TEAM glyphs use the same observed activity label, never a task state or
a pending request as a substitute for activity.
Sent-message feedback shares that acquired activity; the [conversation owner](board-notebooks.md#replies-and-verification) defines its progress and visible clocks.

One identity-total series uses the existing Rate owner. Its header reads
`Usage · last <range> · <total> tok`, with independent right-aligned Range and Each
dropdowns. Range offers 1h, 6h, 1d, 7d and 30d; Each offers Auto, 5m, 15m, 30m,
1h, 2h, 6h, 12h and 1d, omitting intervals longer than the range. Choices exceeding
720 buckets remain visible but disabled, with their minimum interval in the menu
footer. Auto selects the finest interval fitting both the plot width and 720;
fixed choices survive resize. Authored controls belong to the selected view hint.
Each provider has one neutral-grey spark row, then its observed total and share.
The fixed tracks are name (8), a one-cell gap, spark, two spaces, total (7,
right-aligned), two spaces and share (8, right-aligned, muted). Unknown provenance
has its own muted row. TOK and % of all label the numeric tracks; quota controls
keep their independent track. When all buckets fit, each repeats one
eighth-height glyph over its share of the spark track, starting right after the
name track; the remainder cells go to buckets spread evenly along the track, so
runs differ by at most one cell and the plot meets the TOK column at every width.
Otherwise one-cell bars scroll over the loaded series, initially at the newest end. Arrow keys,
wheel and Shift-wheel move the window; Home/End reach oldest/newest. Axis labels
follow that window, with muted `◂`/`▸` at ends containing more history.

Hover or keyboard selection replaces the title with the interval span and total,
and TOK/share show that bucket's provider values. Partial numbers have `~`. The
selected column has a neutral background across all provider rows and an `▲`
axis marker. Click pins/unpins; Esc unpins, and leaving an unpinned chart restores
the full-range summary. There is no Usage Details window or floating card.

Provider shares divide by all observed tokens, including unknown provenance.
Live continuous increments use their verified counter driver. History uses the
drivers Core recorded per bucket (`byDriver`); a bucket without that split is
attributed only through the complete gap-free, non-discontinuous suffix linked to
its included latest driver watermark. Earlier or unproven evidence remains unknown;
current model or identity metadata never relabels history. A mixed receipt slot keeps its proven
portions beside that slot in the same bounded ring owner, removed on replacement.
Driver IDs come from admitted counters, not a built-in name list. Compact local
IDs keep each ring cell at 64 bytes; a driver name can be recycled only after its
last owning fine or retained cell expires. The interval projection merges keyed portions only after
its existing whole-member UUID de-duplication. Held account IDs add quota-only
rows without claiming token provenance. Numeric totals and shares retain the full
source denominator even when a bucket exceeds the spark quantizer's range.
Fine and retained resolutions share that owner and the refresh worker; their
overlapping totals are never added. Typed projection tests retain exact half-open
bounds, observed tokens, missing identities and source durations. Different
identities' durations are agent-time, not inferred joint elapsed coverage. An
unreported, disabled or absent common-pool source stays unknown in the canonical
roster scope. The chart retains at most 720 interval totals and one admitted
scene; the existing Frame retains the terminal diff. Paint and input acquire no
data; the board paints before the existing asynchronous history read fills it.

`board::app::workspace::tests` covers authority, modal lifecycle, pointer geometry,
reload and scoped appearance. `board::workspace::tests` covers pure membership,
counts, selection and sorting. Native service tests exercise public Core and Ops
ports. Capture packets include cells, styles, exact targets and hits across width,
height and colour depth; they are evidence, not regenerated fixtures.

Frozen HOME and TUI parity baselines retain their provenance until a decoded
cells/styles/hits/oracle diff is approved by the UX primary reviewer. Regeneration follows the
[TUI parity gate](../../tmt-tui/references/development.md#board-parity-baseline).
[development.md](development.md#checks) owns commands and isolated captures.

## Conversation todo projection

`conversation_notes::State` owns held per-conversation counts and the global
room/member admission set. The existing quiet Notes lane admits one read at a
time, selected first at five seconds and other keys at most once per minute.
Background deliveries contain counts only; full previews remain with the selected
Notes overlay. The existing write epoch fences late reads. Scope publication and
Notes delivery update the cached header total, so paint does not scan every
conversation. `columns::activity_spans` adds the whole todo suffix to the observed
activity (or the existing unread override), only for a known positive count.
The shared track solver keeps header, row and hit geometry aligned and admits
MODEL / ALERT only when its whole header fits. Pending/unavailable counts stay
distinct from known zero in the global header; none adds a row suffix.
