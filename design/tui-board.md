# Ops board presentation specification

This specification defines the presentation of `tmt ops ui`. Ops owns runtime
behavior, data acquisition and terminal geometry; UX owns visual acceptance.
Use flat neutral surfaces, square edges, soft grey hierarchy and small semantic
marks. Keep state and keyboard actions understandable without color.

The [full-screen interaction rules](cli-style.md#full-screen-interaction),
[Ops board reference](../.agents/skills/tmt-ops-dev/references/board.md) and
[shipped controls](../extensions/tmt-ops/skills/tmt-ops/SKILL.md#board-appearance)
own behavior. This spec describes their presentation; it adds no renderer,
configuration key, action, permission or persistence contract. Effective bindings
supply shortcut hints; user bindings and host-specific actions take precedence
above examples.

## Surfaces and hierarchy

Use the resolved terminal theme and its neutral text, dim, muted and selection
roles. Group the workspace tree and conversation/detail with horizontal rules and
blank side margins, one rule per boundary. Keep measured titles, inner areas and
clipped hit regions together. Do not stack an extra separator above a group
heading or reserve an invisible wall as an independent hit target. Do not add decorative boxes
around each field, shadows, gradients or a colored background for each state.

Inline input and read bands use the same flat horizontal treatment and an opaque
mask. Content beneath the band must not show through or receive covered mouse
hits. Management review/results, settings, pickers and Timers overlays retain square frames.

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
reported state it appends `· offline`, and the offline name is dim. Conversation exchange marks (`◆`, `…`, `✓`) describe the latest request/reply,
not member activity.

On a real selection background, `Look::selected_words` keeps ordinary selected
words readable in the text role while eligible single semantic marks retain
their signal color. Preserve its one final-frame policy and user overrides;
selection must not turn every word into an attention signal.

### Selection, focus and terminal depth

Selection styling follows admitted selected content, not scroll/reveal envelopes.
Grid rows include selected `Part.row` cells and their wrapped continuations;
feedback/annotations with `row=None` are excluded. Workspace selection includes the admitted tree row and its padding, preserving
readable state words. Conversation selection follows its own source owner. Section labels, rules, borders, unselected previews
and composer/read reservations are not selected content. Repeated UUIDs retain
occurrence identity; only the selected occurrence expands or composes.

Selecting a row does not transfer keyboard focus to it. Keep current title,
input, modal and effective-hint routing. Borderless workspace rows do not acquire
new `Focus: rows` footer labels or an extra focus row. Footer input, search,
notice, selected link and error take precedence over ordinary hints. Drop
whole low-priority hints; preserve `? more` and `q quit`, or `? more` alone when
only it fits.

True-color terminals use the resolved neutral theme and small semantic accents.
Terminal16 respects the terminal's palette; words, marks, rules and spacing carry
meaning when fills are indistinguishable. `Depth::None` removes theme colors and
effects, while application selection retains its no-background reverse/bold
fallback. Do not claim escape-free live terminal output: cursor, erase,
alternate-screen and mouse control remain protocol. No decorative blink or
pulse is needed; loading and observed-meter animation retain their own lifecycle.

### Supplied row labels and digest chooser

A supplied label group replaces the Core policy chip, never accompanies it. Its
muted source name leads the labels (`digest Auto · 3 held`); separate labels with
`  ·  ` and never show the source alone. Keep the name and the whole state word
first. Then show the source with its first label and as many further labels as
fit, dropping trailing labels whole in the supplier's order; in grid headings the
row-end age and `cron next` drop before any label. Show no chip when the source
plus first label does not fit. The
[Digest label contract](../extensions/tmt-digest/contracts/digest-v1.md#labels)
owns wording, units, states and supplier option order; Ops renders them rather
than recalculating them.

`▾` after the mode identifies its clickable choice. Click it or use the digest key
(`D` in the workspace) to open the centered `<member> digest` menu.
Mark the current option with `(current)`, retain supplier order and put `Custom…`
last. Custom opens `<member> digest · Enter sets, Esc cancels` with the hint
`A duration such as 20s, 10m or 1h; empty goes back to the default.`

Keep outcome notices explicit: success is `Digest for <member>: <value>.`, rejection
is `Digest for <member> was not changed: <value> was not accepted.`, and when the
answer is lost, show its error followed by `Digest for <member> may or may not have
changed.` Invalid input says `That is not a value digest can take; nothing
changed.` Escape in the Custom input says `Digest not changed.`; Escape in the
choice menu only dismisses it, without that notice. None of these notices implies
an automatic retry.

## Frame and view composition

One `OPS / UI` shell contains `1 Overview`, `2 Agents`, and `3 Digest`. The same
acquired roster feeds all three sections. Overview owns global attention, usage
and provider limits. Agents owns the squad workspace and selected conversation;
Digest keeps the same exact identity context and Core policy controls. A section
label never creates membership or a send audience. Agents/Digest use the selected
squad's appearance; Overview uses the shared board appearance.

The dense tree keeps virtual Common pool first, then pinned squads and current
sort order. Common pool means zero Ops squad memberships, proven by complete
observations; failed/partial reads do not prove absence. A lead is marked and
included once in its squad count. The same identity may have different states
in different squads. A collapsed squad keeps its worst observed child state,
with attention priority. Display names preserve authored text, including `tmt-`
prefixes, and fit through the shared text policy.

| Section  | Composition                                        | Placement                                                  |
| -------- | -------------------------------------------------- | ---------------------------------------------------------- |
| Overview | Global attention/usage/limits and squad/pool tree  | Global facts appear once; partial/stale totals say unknown |
| Agents   | Tree plus exact agent conversation or squad detail | Auto/Side/Stack uses actual minimum usable areas           |
| Digest   | Same tree/context and existing policy chooser      | No new Digest store, permission or parallel editor         |

Auto/Side/Stack, pin and sort are per-user Ops preferences. The new default keys
replace old squad tabs, Left/Right tab switching, tab reorder/pick/hide and factory
views; no legacy mode applies those arrangements. Located migration notices
preserve authored configuration and identify replacements. Independent public
`ls --tab`/layout projections remain separate contracts.

Tree arrows move/expand/collapse, `s` sorts, `S` reverses, `n` creates, `m` manages
or assigns, `x` removes only the selected membership, `D` opens Digest, and `c`
opens Timers. Member rename is deferred and has no `r` hint/default. `w` is reserved
for snapshots. Text/search/completion/modal/conversation input claims its keys
before section digits. Dispatch, help and footer use one effective binding table;
compatible user/host/section bindings retain precedence.

### HOME overview and sections

Overview names known attention and working/idle/blocked totals, retaining
partial/stale/unknown distinctions. A complete empty attention read may say
`Nothing needs you`; a failed read never becomes a fabricated zero. The global
longest-window usage line uses actual configured labels, `~` for partial evidence
and `0` for measured zero. Deduplicate identities using the better-covered
observation; missing readings remain missing. No rate or money estimate replaces
completed-request token totals.

Provider bars use existing approximately 18-cell labels and shared width steps:
at 160 cells two fit side by side; narrower widths give each its own row.
Keep percentage inside and depletion warning outside. Reverse-fill the existing
working role above 40% remaining, waiting from 15% through 40%, and blocked below
15%; Text over neutral Track marks the unfilled part. NO_COLOR reverses fill and
underlines track. Only depletion before reset adds reset time, positive measured
pace `~N%/h` and eligible trend; insufficient/stale history omits the trend.
Warning `! ~duration` becomes `✗ ~duration` at or within 12 hours, independently
of remaining-balance colour. Preserve the meter owner's bounded observation,
coverage and disclosure contracts; paint/input acquire nothing.

Room/identity UUIDs anchor selection across search, collapse, refresh, sort, pin
and reload. Hidden or removed selection remains explicit and cannot retarget an
action to its neighbour. Scroll/reveal and clipped hits consume the same painted
geometry. Narrow/low-height layouts keep context and an explicit next action;
clipping never creates authority.

## Rows, reading and composing

Keep the selected recipient, task context and shared WP1 conversation together.
There is one editor and one session draft/reading position per sender/recipient
UUID, independent of display names, membership changes and section/placement.
The opaque composer/read region masks covered cells and hits. Reading sends and
acknowledges nothing. Composing shows the admitted recipient/mode, quoted request,
draft and effective hints. Available answer/note/talk/status modes retain drafts;
several requests require an explicit choice. Notes require a real squad context;
Common pool Talk never invents a squad. Opening/cancelling/empty input sends nothing.

Refresh preserves exact selection and retained drafts. Loading, unavailable and
stale values differ from a successful empty result. The existing worker owns
reads and refresh; display labels never replace sender/recipient/source authority.

## Settings, actions and overlays

Use shared opaque picker/Modal slots, FocusStack and current clipped hit maps.
Settings offers Auto/Side/Stack, Theme, Token window, then Actions. Theme preview
stays in memory until explicit save; cancelling restores appearance with refreshed
data. Config owns format-preserving CAS writes; failure/conflict publishes no new
choice. Masking squad settings are named rather than silently removed.

Management review freezes exact operations, UUIDs/revisions and observed timer
impact. Cancel has initial focus; Tab reaches the readable scroll region and
Apply. Short terminals use the available screen and the shared position row.
Apply requires current visible geometry; pending blocks repeated submission.
Results retain per-item partial evidence and explicit repair, never automatic
retry. Removing membership pauses source jobs before leave and never stops or
retires the identity. Re-add/reassign never resumes pause; explicit Resume does.

Checklist is an action reached through Settings → Actions… → Checklist, not a
new default board section or assigned shortcut. Its list distinguishes completion,
assignment and archive filters; selection does not assign or complete an item.
Use its existing details, preview/Confirm, initially focused Cancel for Delete,
retained drafts and explicit conflict review. Completion changes neither agent
state nor request attention. The [shipped Checklist controls](../extensions/tmt-ops/skills/tmt-ops/SKILL.md#use-the-board-checklist)
own exact actions and authority.

Cron remains separate from member focus. Use the selected room UUID for the job list and owner prefill; keep
paused/active state, owner and next slot, empty versus failed-read distinction,
and draft/validation/refusal feedback. A selected paused job is not running or
sending. Modal keys and covered mouse hits never activate the board beneath it.

## Verification criteria and reuse

Use existing `Look`, Outline/Modal/Strip/Scrolls, WP1 editor/conversation and
picker schemas. Selection paint,
scroll/reveal and hits remain distinct responsibilities. No raw-widget exception,
parallel layout pass, new renderer or frozen-fixture regeneration follows from
this specification.

Review identical inputs at 80/100/160/actual 180+ cells and ordinary/low heights,
in light/dark/terminal16/NO_COLOR. Include wrapped selected rows, selected but
unfocused panes, duplicate occurrences, a middle-row composer, covered hits,
collapsed squads, scroll/list end, disabled/error states and retained drafts.
Include Overview usage omission, measured zero, partial data at 60/80/100/160
cells and their width boundaries, plus settings preview/cancel/conflict and
Checklist/cron authority.

The reader must be able to identify the shown scope, item needing a response,
next key's destination, selected occurrence, draft recipient and usage absence
versus zero. Captures and runtime checks establish implementation behavior;
schematics and source inspection alone do not certify measured geometry, palette
contrast or usability timings. Keep acceptance evidence and unresolved findings
in the owning issues/PRs, rather than a delivery diary in this spec.
