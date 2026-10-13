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
anchor disables actions rather than selecting its neighbour. A filter with no
observed matches keeps Common pool and adds a non-interactive message in the
tree viewport; it does not replace the selection or claim a complete roster.

## Shell, geometry and input

`board::shell` owns Overview, Agents and Digest selection. `workspace::render`
uses shared TUI Outline, Strip and Scrolls for the dense tree and squad detail;
only rows admitted to the scroll viewport build text spans. The full stable tree
retains scroll length and row indices for selection and hit mapping. WP1 retains
conversation, editor, drafts, reply reader and send authority.
Overview owns global attention/usage/limits. Agents and Digest use the selected
squad's appearance and contextual detail. Auto/Side/Stack is an Ops preference,
not a second editor or a revival of factory board views. An explicit message
focuses the selected agent's conversation: compact Overview opens Agents, while
wide Overview retains its shared pane. A retained Reader moves to Agents when
Overview can no longer fit its conversation; refresh/resize never converts its
scroll keys into list/Host actions. Logical placement survives clearing pointer
rectangles. With fewer than three content rows, the tree omits its outline and
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

Selected-squad detail formats supplied usage windows, lead readings/share and
independent model observations without the old tile width rules. Missing totals
do not hide independently supplied lead readings or share; absent observations
remain not reported, measured zero stays zero, and partial evidence keeps its
marker. Squad summary does not expose private lead task or link fields.

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
timer impact for review. The shared modal starts on Cancel and offers a scrollable
read region before Apply. Short terminals use a screen-sized sheet; read position
uses the shared modal's reserved position row. Apply requires current geometry.
Pending blocks replay; partial results remain readable without automatic retry.
The service rechecks the frozen identities and revisions before writes.

In-place reload carries shell choice, expanded room UUIDs, pool expansion and the
exact selected target in resume version 3. A removed target stays missing.
A retired HOME cursor or detail invalidates the reload hint and is consumed
without opening a surface; bounded private-file and symlink checks remain.
Workspace forms block reload until their operation/result is dismissed. The old
HOME display cache is not admitted as a new shell frame.

## Usage and verification

`App::home_usage` and provider-limit estimates reuse the existing history owner:
missing differs from measured zero, uncovered evidence is partial, sampling off
has no token cells, and model metadata is best effort. Provider pace/trend remains
a derived current-cycle observation, not a persisted field or new acquisition.
See the meter reference for coverage, time fences and trend thresholds.

`board::app::workspace::tests` covers authority, modal lifecycle, pointer geometry,
reload and scoped appearance. `board::workspace::tests` covers pure membership,
counts, selection and sorting. Native service tests exercise public Core and Ops
ports. Capture packets include cells, styles, exact targets and hits across width,
height and colour depth; they are evidence, not regenerated fixtures.

Frozen HOME and TUI parity baselines retain their provenance until a decoded
cells/styles/hits/oracle diff is approved by UX and Core. Regeneration follows the
[TUI parity gate](../../tmt-tui/references/development.md#board-parity-baseline).
[development.md](development.md#checks) owns commands and isolated captures.
