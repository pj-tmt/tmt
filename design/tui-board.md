# Ops board presentation specification

This specification defines the presentation of `tmt ops ui` for #1829 under
#1825. Ops owns runtime behavior, data acquisition and terminal geometry; UX owns
visual acceptance. Use flat neutral surfaces, square edges, soft grey hierarchy
and small semantic marks. Keep state and keyboard actions understandable without
color.

The [full-screen interaction rules](cli-style.md#full-screen-interaction),
[Ops board reference](../.agents/skills/tmt-ops-dev/references/board.md) and
[shipped controls](../extensions/tmt-ops/skills/tmt-ops/SKILL.md#board-appearance)
own behavior. This spec describes their presentation; it adds no renderer,
configuration key, action, permission or persistence contract. Effective bindings
supply shortcut hints; user bindings and host-specific actions take precedence
above examples.

## Surfaces and hierarchy

Use the resolved terminal theme and its neutral text, dim, muted and selection
roles. Group member rows, HOME leads and body panes with horizontal rules and
blank side margins, one rule per boundary: a heading's rule replaces the box top
under it, and a pane above another pane's heading or the cron heading ends on that rule. Keep the existing measured title, fold controls, inner area
and hit regions when a side wall is visually absent. Do not add decorative boxes
around each field, shadows, gradients or a colored background for each state.

Inline input and read bands use the same flat horizontal treatment and an opaque
mask. Content beneath the band must not show through or receive covered mouse
hits. Ask-lead, settings, pickers and cron overlays retain square frames.

Use `tmt-cli-style` theme resolution through Ops `Look`; tmt-tui paints the admitted
styles and geometry. Preserve global, board and squad overrides and automatic
light/dark detection. Browser RGB values and a second terminal palette do not
belong in painters. Surface distinctions do not require invented token names.

| Meaning           | Presentation                                                  | Non-color information                                          |
| ----------------- | ------------------------------------------------------------- | -------------------------------------------------------------- |
| Ordinary content  | Neutral readable text, quiet background                       | Titles, spacing and horizontal group rules                     |
| Selected item     | Existing selection background or reverse fallback             | Its stable occurrence and selection position                   |
| Keyboard focus    | Existing receiving pane title, input or modal focus treatment | Effective actions and cursor destination                       |
| Waits on the user | Small attention mark, ordinary body text                      | `◆` and the waiting state/request                              |
| Working           | Small working mark                                            | `●` and reported state                                         |
| Review/testing    | Small review mark                                             | `◐` and reported state                                         |
| Blocked/failed    | Small failure mark and reason                                 | `✗` plus actionable explanation                                |
| Idle              | Quiet state mark                                              | `◌` and reported idle state                                    |
| Offline           | Muted observed presence/name                                  | `○` in presence-based CLI lists; `offline` in board state text |
| Missing value     | Quiet explicit absence                                        | `–` or a named unavailable state, never fabricated zero        |
| Disabled action   | Muted control with its admission reason                       | Activation refused; selection grants no authority              |
| Link              | Existing link role and supported underline                    | Label and effective open action                                |

Reported state and observed presence are separate. Board member marks describe
reported state, so an offline member does not acquire a `○` state mark. With no
reported state, the state cell shows observed `online` or `offline`; with a
reported state it appends `· offline`, and the offline name is dim. HOME lead
marks describe the latest exchange (`◆`, `…`, `✓`), not member activity.

On a real selection background, `Look::selected_words` keeps ordinary selected
words readable in the text role while eligible single semantic marks retain
their signal color. Preserve its one final-frame policy and user overrides;
selection must not turn every word into an attention signal.

### Selection, focus and terminal depth

Selection styling follows admitted selected content, not scroll/reveal envelopes.
Grid rows include selected `Part.row` cells and their wrapped continuations;
feedback/annotations with `row=None` are excluded. Member groups select the
heading and task; HOME leads select the heading. HOME squad table selection
includes its row padding. Section labels, rules, borders, unselected previews
and composer/read reservations are not selected content. Repeated UUIDs retain
occurrence identity; only the selected occurrence expands or composes.

Selecting a row does not transfer keyboard focus to it. Keep current title,
input, modal and effective-hint routing. Borderless rows and HOME do not acquire
new `Focus: rows` footer labels or an extra focus row. Footer input, search,
notice, selected Notes link and error take precedence over ordinary hints. Drop
whole low-priority hints; preserve `? more` and `q quit`, or `? more` alone when
only it fits.

True-color terminals use the resolved neutral theme and small semantic accents.
Terminal16 respects the terminal's palette; words, marks, rules and spacing carry
meaning when fills are indistinguishable. `Depth::None` removes theme colors and
effects, while application selection retains its no-background reverse/bold
fallback. Do not claim escape-free live terminal output: cursor, erase,
alternate-screen and mouse control remain protocol. No decorative blink or
pulse is needed; loading and observed-meter animation retain their own lifecycle.

## Frame and view composition

Tabs identify HOME (`@all`), leads or the shown named/custom squad. A requested
uncached tab remains distinct from the displayed retained view. Pinned tabs and
the overflow switcher remain discoverable at narrow widths. An authored label
creates neither membership nor a new send audience.

Keep the factory views and user-authored pane trees. Their fold thresholds are
layout behavior, not new styling breakpoints.

| View    | Composition                                               | Width/fold behavior                                    |
| ------- | --------------------------------------------------------- | ------------------------------------------------------ |
| HOME    | Counts, blocked, grouped leads, audience, cron, squads    | One stream; full-width squad table at every width      |
| leads   | Grouped lead headings and admitted preview/read bands     | Existing stream and clipped row hits                   |
| members | Lead, members rule and member/task list; lead notes below | Default grouped list; notes can be hidden/shown        |
| team    | Rows with detail/replies beside them and notes below      | Detail/replies fold below 100 cells                    |
| focus   | Rows with initially folded detail/replies/notes           | Manual folds retain session state                      |
| notes   | Rows beside lead notes; detail/replies folded             | Existing pane tree and note/link focus                 |
| detail  | Rows above detail/replies; notes folded                   | Replies fold below 100 cells                           |
| wide    | Rows, detail/replies and notes in three columns           | Detail/replies fold below 180 cells                    |
| custom  | Authored sections, cells, splits and tabs                 | Preserve configured geometry and duplicate occurrences |

HOME is the built-in `@all` board, not another aggregate member grid. Public
`ls --tab all` is a separate listing projection. Named squad tabs start at the
lead row; the members rule is not selectable. HOME chooses an admitted blocked,
lead or squad row; the cron heading is not an initial member selection.

### HOME usage row

Put observed usage below HOME's counts, separate from body selection. At 100
cells and wider it includes the last two global `[board] tok` windows and the
longest-window top member/share, for example:

```text
tok 5m ~N · 1h ~N · share 1h: member P%
```

At 140 cells it also includes the first window, model shares and the number of
members without data. Deduplicate UUIDs across shown sampling squads using the
better-covered observation; search limits the row to currently shown squads.
Use `~` for incomplete totals/shares, `0` for measured zero and no share for a zero
total. Missing readings are excluded and counted once; model attribution is
best effort. Hide the row below 100 cells or when no shown sampling squad has an
observed reading. Do not show a named-squad summary on HOME or replace observed
completed-request totals with a token rate or money estimate.

Squad table token columns and their legend appear only when observations admit
them. Sampling-off hides token cells; observed model names can remain. Missing
and measured zero stay distinct; mixed window settings label actual durations.

## Rows, reading and composing

Keep the complete task and selected recipient together. Expanding a member
replaces its task preview with the shared read band at that occurrence. Ordinary
answer/note/talk composing retains the task preview and reserves its opaque band
beneath the complete target row; later rows shift together. Mask covered side
panes and hits. Do not place a second composer beneath the whole notes grid.

HOME leads start collapsed. With no exchange, show the existing blank mark and
heading-age dash, not an invented preview row. Reading sends and acknowledges
nothing. Composing shows the admitted recipient/mode, quoted request, draft and
effective hints. Mode cycling retains available answer/note/talk/status drafts.
Several requests require the existing explicit choice. A note changes neither
manual pending nor an inbox request's finalization; plain-host Enter and tmux
jump retain their own action routing.

Refresh preserves occurrence selection, retained drafts and existing cell
stability. Loading, unavailable and stale values remain distinguishable from a
successful empty result. Paint/input do not acquire data; the existing worker
owns reads and refresh.

## Settings, actions and overlays

Use the existing opaque picker/Modal slots, focus stack and hit map. View,
Theme, action menu, tab switcher and cron list show `›` on the selected choice's
first line when it fits. The saved-view `●` and picked-tab `[x]` remain separate
from the moving cursor. Opening a picker or preview changes no authority.

Settings expose Actions…, Theme, View and Token window. Show the current value,
source/scope, editable/read-only state, retained draft and save/conflict feedback.
View/theme previews remain in memory until explicit save; cancel restores the
opening arrangement with refreshed data. Writes use the existing format-preserving
Config owner; failure or conflict publishes no new value. Keep labels, validation
and the next action readable at narrow widths.

Checklist is an action reached through Settings → Actions… → Checklist, not a
new default board section or assigned shortcut. Its list distinguishes completion,
assignment and archive filters; selection does not assign or complete an item.
Use its existing details, preview/Confirm, initially focused Cancel for Delete,
retained drafts and explicit conflict review. Completion changes neither agent
state nor request attention. The [shipped Checklist controls](../extensions/tmt-ops/skills/tmt-ops/SKILL.md#use-the-board-checklist)
own exact actions and authority.

Cron remains separate from member focus. Keep existing jobs/list geometry,
paused/active state, owner and next slot, empty versus failed-read distinction,
and draft/validation/refusal feedback. A selected paused job is not running or
sending. Modal keys and covered mouse hits never activate the board beneath it.

## Verification criteria and reuse

Use existing `Look`, `Outline::paint_flat`, `Modal::paint_flat`, row scenes,
shared detail band, HOME section painters and picker schemas. Selection paint,
scroll/reveal and hits remain distinct responsibilities. No raw-widget exception,
parallel layout pass, new renderer or frozen-fixture regeneration follows from
this specification.

Review identical inputs at 80/100/160/actual 180+ cells and ordinary/low heights,
in light/dark/terminal16/NO_COLOR. Include wrapped selected rows, selected but
unfocused panes, duplicate occurrences, a middle-row composer, covered hits,
all panes folded, scroll/list end, disabled/error states and retained drafts.
Include HOME usage omission, measured zero, partial data and the 100/140-cell
thresholds, plus settings preview/cancel/conflict and Checklist/cron authority.

The reader must be able to identify the shown scope, item needing a response,
next key's destination, selected occurrence, draft recipient and usage absence
versus zero. Captures and runtime checks establish implementation behavior;
schematics and source inspection alone do not certify measured geometry, palette
contrast or usability timings. Keep acceptance evidence and unresolved findings
in #1829/#1825 and their PRs, rather than a delivery diary in this spec.
