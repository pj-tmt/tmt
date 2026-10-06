# Squad board redesign proposal

**Status: design draft for #1829 under #1825.** The accepted direction is flat
opaque neutral blocks, a black/white foundation, soft gray hierarchy and fully
saturated semantic signals with no shadows. This document proposes
the complete presentation system; it does not describe a delivered redesign.
Squad owns runtime behavior and checklist data. UX owns this specification and
visual acceptance. The browser package (#1797) is a separate route.

## Design outcome

A reader should recognize the current squad, keyboard destination and item that
needs a response without scanning every field. Keep task and reply content
readable; spend saturated color on small meaningful cues. Neutral surfaces group
content, while text, marks and selected geometry remain useful without color.

The cross-product direction accepted in #1874 applies to visual roles, not a
shared web/terminal renderer. Terminal palette resolution, cell geometry, user
overrides and keyboard behavior remain owned by Squad. Do not copy browser RGB
candidates into painters or treat an illustrative web mock as runtime evidence.

The existing [full-screen interaction rules](cli-style.md#full-screen-interaction)
and [board behavior owner](../.agents/skills/tmt-squad-dev/references/board.md)
remain authoritative. This proposal changes presentation, not send, request,
configuration, membership or persistence authority. Effective bindings supply all
displayed shortcut hints. Specific current composer and host behavior takes
precedence over the generic key examples in the style guide.

## Flat surfaces and semantic roles

Use one quiet canvas, one neutral content surface, an opaque inline band and the
selected background with a distinct non-color indicator. Preserve existing
selection geometry and behavior while reviewing its appearance through #1830.
Content surfaces have square edges and no offset
shadow cells. Avoid a colored card for every state or separate frames around
each field. A focused pane is recognizable from its title and focus marker;
selecting a row does not imply that pane currently receives keys.

| Meaning             | Presentation target                                          | Non-color cue                                                   | Existing owner / next boundary                                     |
| ------------------- | ------------------------------------------------------------ | --------------------------------------------------------------- | ------------------------------------------------------------------ |
| Canvas              | Quiet background outside content blocks                      | Spacing and reading order                                       | Theme/screen adapter; proposed new surface roles need #1830 review |
| Content surface     | Neutral block, shared by rows and reading panes              | Section title; one boundary when adjacent panes need separation | `Outline` / scene composition                                      |
| Selected item       | One continuous background over the complete wrapped item     | Explicit selected-occurrence mark across the wrapped item       | `Look::row_span`, `selected_words`                                 |
| Keyboard focus      | Distinct pane-title focus label; optional supported emphasis | Focused title with a distinct focus label                       | App focus, pane title / modal focus stack                          |
| Waiting on the user | Warm attention mark; ordinary readable words                 | `◆` plus `waits on you`                                         | Acquired attention predicate; never checklist-derived              |
| Working             | Green mark, quiet body text                                  | `●` plus `working`                                              | Admitted state; not a heartbeat                                    |
| In review           | Review mark / role                                           | `◐` plus `in review`                                            | Admitted state; not waiting on Ben                                 |
| Blocked or failed   | Error mark plus one actionable sentence                      | `✗` plus reason                                                 | Existing failure owner                                             |
| Checklist complete  | Small success cue, retained item title                       | `[✓]` plus `done`                                               | Proposed presentation; allowed state from #1826                    |
| Muted / unavailable | Secondary labels; readable inherited contrast                | `–`, empty phrase or explicit source label                      | Existing `muted` / `dim` roles                                     |
| Disabled action     | Muted label with explicit unavailable reason                 | Disabled marker; activation refused                             | Existing admission/controller                                      |
| Link                | Link role and underline where supported                      | Link label and effective open hint                              | Existing link validation / effects                                 |

Do not place literal palette values in Squad painters or create a second palette
in tmt-tui. Existing `Role` does not have canvas/content/band backgrounds: those
are proposed design distinctions, not admitted token names or config keys.
#1830 must map them through the existing theme/screen owner and bring any actual
core style/token seam to its owner. Preserve user theme overrides and auto theme.

In a selected row, ordinary text uses the existing selection contrast treatment;
single semantic marks retain the current mark policy. Test words, marks, links,
disabled controls and errors on the selected background together. Do not paint
attention over every word or make a disabled item look actionable because it is
selected. Completion styling must leave the title readable.

`Look::selected_words` is the existing final-frame policy owner: selected
muted/dim/accent/link/state words use the text role, while eligible single
semantic marks retain their color. It matches resolved role colors rather than
measuring contrast dynamically. Preserve this single owner and user overrides;
browser candidate hex values are not terminal painter constants.

### Terminal degradation

- True color: neutral canvas/content separation, small semantic accents and
  selection contrast. The accepted private prototype is direction, not a token
  implementation or a contrast certification.
- Terminal 16 colors: respect the user's palette. Default foreground/background,
  bold titles, boundaries and marks must carry grouping when two neutral fills
  cannot be distinguished. No exact RGB contrast claim for arbitrary palettes.
- NO_COLOR target: `Depth::None` removes theme-supplied colors and effects,
  but current `Look::selection` adds reverse without a background and
  `row_span` may add bold. Application-owned emphasis is a separate source.
  Explicit cues must make selection understandable without relying on those
  effects. Removing the existing fallback is not a prerequisite for the first
  role-mapping slice; any removal needs affected selection evidence first. Explicit
  glyphs and text identify the selected occurrence, receiving
  focus and checked state independently; spacing and boundaries retain grouping.
  An error keeps both its mark and its explanation. Interactive cursor, erase,
  alternate-screen and mouse-control sequences remain terminal protocol, not
  theme styling. Escape-free output applies to retained plain-text schematics
  and exports, not to the live board's terminal protocol.
- No decorative blink, pulse, gradient or shadow. Existing meaningful loading and
  observed meter updates retain their own lifecycle.

### Reuse boundaries for #1830

The following maps presentation onto existing owners. It proposes bounded
changes, not a replacement renderer or new behavior controller. Actual theme
tokens and overrides retain their canonical owner; browser RGB values are not
literal constants for Squad painters.

| Presentation                       | Existing reuse owner                                                                           | Required preservation / design gap                                                                                                                                                                                                                                                                                                    |
| ---------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Role resolution and selected words | Squad `look.rs::Look::{role,selection,row_span,selected_words}` and `tmt-cli-style::Theme`     | Preserve user theme resolution and semantic mark treatment. `Look::selection` explicitly adds reverse without a selection background; `row_span` adds bold in that fallback. Theme `Depth::None` alone does not establish the proposed non-effect selection target. Resolve that mapping in #1830 with explicit selection/focus cues. |
| Pane grouping and focus            | `board/view/panes.rs`, `board/composition`, `tmt-tui::components::Outline`                     | Keep factory/authored composition, folded pane slots and receiving focus. `Outline` paints a boundary; it does not fill its contents. Any neutral fill belongs to the existing pane paint boundary, not a second layout pass.                                                                                                         |
| Wrapped rows, scroll and hits      | `board/view/rows.rs`, `board/view/row_paint.rs`, member-list painter and existing scroll owner | Preserve measured complete row ranges, occurrence selection, clipped hits and follow-versus-wheel behavior. Apply selected styling to those same measured cells.                                                                                                                                                                      |
| Composer and expanded read band    | `board/view/waiting` and `board/view.rs`                                                       | Reuse existing reserved row lines and input placement; retain full ordinary task preview, shifted later rows, opaque split-pane masking and admitted recipient/mode. No independent bottom composer.                                                                                                                                  |
| Header/footer and compact text     | Existing board view/home owners and `tmt-tui::components::strip`                               | Keep effective hints, displayed/requested scope and observed-total admission. Style cannot change tab acquisition or attention predicates.                                                                                                                                                                                            |
| Pickers, settings and actions      | Existing picker surface schemas and `tmt-tui::components::{surface,collection,Modal}`          | Reuse measured title/query/body/status/footer slots and hit maps. Keep draft, current values, save/conflict and modal key ownership in their existing controllers.                                                                                                                                                                    |

Shared primitive code is justified only by actual consumers. #1830 may deliver
an accepted reuse map without new abstractions; #1833 depends on a new primitive
only when it actually consumes it. Acquisition, caching, request/attention,
checklist data and loading lifecycle are outside this presentation batch.

## Frame and reading hierarchy

1. **Tabs:** HOME (`@all`), leads and named/custom tabs retain the current keys and
   ordering. A selected tab is distinct from a requested uncached tab. Pinned
   groups and the overflow switcher remain discoverable at every width.
2. **Summary:** current scope, lead/member counts and response attention. Named
   sampling squads retain their selected window and no-data line. HOME retains
   its different usage admission rules. Usage is completed-request token totals,
   never a per-second rate or cost.
3. **Body:** neutral content groups follow the configured composition. The
   selected occurrence and complete inline band are one reading unit. Cron and
   checklist sections have their own labels and focus; no overloaded attention
   count combines their items with unanswered requests.
4. **Footer:** current effective actions, notices and overflow state. Drop whole
   low-priority hints rather than truncate keys. Help remains reachable.

Headers for HOME (`@all`), leads and named/custom squads must state the displayed scope.
An authored tab's label is not proof of membership or a new request audience.
Duplicate appearances of one agent retain occurrence identity; selection,
expansion and composing affect only the chosen occurrence. Shared observed usage
continues to deduplicate by UUID through the existing meter owner.

## View composition

These targets preserve the factory arrangements in `ViewName::settings` and
authored layouts. Color blocks do not introduce a competing Todo-only layout or
silently replace the user's pane tree. Pane ratios below identify existing
arrangements, not new breakpoint policy.

| Surface       | Target grouping                                                          | Narrow / folded behavior                                                  | Required interaction specimen                                       |
| ------------- | ------------------------------------------------------------------------ | ------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| HOME (`@all`) | Attention, boxed leads, audience, cron and squad table in existing order | One stream; preserve section positions and admitted observed-total header | Needs-you, no reading, measured zero, deferred leads, sent feedback |
| leads         | Boxed lead rows with preview / expanded band                             | Same row stream and clipped hits                                          | Replies off/on, expanded lead, multiple requests                    |
| members       | Boxed lead/member list, then lead notes                                  | Existing 60/40 layout; task preview remains inside the box                | Middle-row expansion and answer/note/talk composer                  |
| team          | Rows with detail/replies beside them, notes below                        | Existing detail/replies fold below 100                                    | Inline opaque band masks covered side panes; later rows shift       |
| focus         | Rows, with detail/replies/notes title lines                              | Preserve configured collapsed panes                                       | All folded, expand focused pane, end of list                        |
| notes         | Rows beside lead notes, detail/replies folded                            | Existing tree; readable notes and focused links                           | Notes scroll / links, retained row selection                        |
| detail        | Rows above detail/replies; notes folded                                  | Replies fold below 100                                                    | Long fields, missing data, reply scroll                             |
| wide          | Three columns: rows, detail/replies, notes                               | Preserve actual 180+ arrangement and below-180 folds                      | Full 180+ screenshot plus resize across threshold                   |
| custom        | User-authored split/tab composition and row cells                        | Existing tree/fold rules; no implicit rewrite                             | Duplicate agent occurrences, reordered sections and pinned tabs     |

The built-in `@all` board tab is HOME, not a second aggregate board surface.
HOME keeps its admitted observed-total usage header and omits the named-squad
summary. Authored aggregate sections belong to custom/user tabs. Public
`ls --tab all` is a separate listing projection, not another board surface.

### Schematic: member row and inline composer

```text
MEMBERS · product                         [focused pane]
  ● Sol              lead · working
    Review board controls
  members · 3
> ◆ auth-fix         waits on you · 14m     [selected occurrence]
  Fix login and preserve user bindings     [collapsed task preview]
  ● docs             working
    Write the guide
```

On expansion, replace the task preview with the shared read band at that same
occurrence. On composing, reserve the opaque band beneath the **complete** target
row. Header, quoted request, draft and mode hints belong to the band. Shift later
rows and mask covered split-pane hits; never put the composer after the whole
notes grid. A narrow screen scrolls the band rather than hiding the recipient.
Ordinary answer/note/talk composing retains the selected task preview. The boxed
member painter suppresses that preview for `ReadRow`, not ordinary composing.

```text
> ◆ auth-fix         waits on you · 14m
  Fix login and preserve user bindings     [retained task preview]
  → auth-fix (product) · answer
  Request: Which login behavior should be retained?
  Draft: Keep the current saved bindings.
  Tab modes · Enter send · Esc cancel       [effective hints]
  ● docs             working               [later row shifted]
```

Mode cycling retains each available answer/note/talk/status draft and its
admitted recipient or status subject through the existing controller. Status
updates, request answers and pending-only notes remain separate effects. With
several requests, show the existing explicit request selection. Read-only `t/e`
states never imply a send. Pending-only writing is a note; it does not clear
pending or finalize an inbox request. Plain-host Enter exposes the action menu;
tmux jump uses the existing host-aware action. Custom bindings override printed
defaults. This proposal does not assign a checklist shortcut.

## Checklist presentation boundary

#1826 owns the item/source/assignment/transition contract. The proposed board
section is titled **CHECKLIST · <squad>**; its count explicitly says `unfinished`
or `done`, never `waiting on you`. Keep squad-wide/unassigned and selected-agent
scopes visible if admitted by that contract. A filtered empty list differs from
a squad with no items. Completing a visible item must not change request or
manual-pending attention.

```text
CHECKLIST · product
2 unfinished · 1 done   All items           [proposed filter label]
> [◐] Fix login and preserve user bindings
      in progress · result not yet confirmed
  [ ] Write the installation guide
      not started
  [✓] Verify cache-hit scenarios
      done
```

The specimen illustrates presentation only: `[ ]`, `[◐]` and `[✓]` are proposed
labels contingent on #1826, not a frozen schema. Create, assign, progress,
complete and reopen appear only when the owning contract admits them. A mutation
shows its target, action and revision-conflict/failure feedback; retain draft and
selection after refusal. Do not add automatic parsing of task prose, GitHub
synchronization, dispatch or request acknowledgment. Checklist placement in the
user's composition remains a contract/design decision for #1834, not a new
default layout from this document.

## Overlays, settings and cron

Use existing opaque square `Modal`, picker/list surface and focus routing. Keep
title, body, status, position and effective hints in their measured slots.
Neutral fills group fields; labels and errors remain explicit. Unhandled modal
keys cannot activate the board beneath it, including covered mouse hits.

| Surface             | Specimen states and visible result                                                                                     |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| View / theme picker | Current value, query/no match, selected/disabled item, preview, confirm/cancel and resize                              |
| Tab switcher        | Pinned/current tab, overflow, hidden/unpicked inventory, custom binding, failed refresh retaining selection            |
| Settings quick rows | Theme/View/current Token window; selection preserved after successful Config save                                      |
| Config editor       | Editable versus read-only, draft, validation failure, stale conflict, durable save; failed save publishes no new value |
| Help / action menu  | Context and effective keys, long description, narrow stacked labels, disabled action reason                            |
| Notice              | Success, caution, partial/stale and failure; next-key lifetime remains owner-controlled                                |
| Cron list           | Scoped job selection, paused/active status, owner, next slot, no jobs, retained rows plus failed read                  |
| Cron forms          | Create/edit/reassign/delete confirmation, exact message draft, validation/refusal/conflict; no automatic retry         |

Keep cron separate from member focus. Its jobs half retains the existing content
cap and low-height rule-only behavior; forms reuse the existing draft/controller.
Do not imply a selected paused job is currently sending or running.

## Coverage and evaluation

Use the same bounded fixture for old/new comparisons: one lead, one real needs-you
row, one working row, one unavailable value, a repeated agent occurrence, long
Unicode content and a middle-row composer. Compare identical viewport, tab,
selection, request set and meter evidence. Do not generate expected behavior from
the new painter or regenerate frozen parity without the owning approval.

| Axis             | Required specimens                                                                                                                          |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Width / height   | 80, 100, 160, actual wide 180+; ordinary and low height; threshold resize                                                                   |
| Palette          | Dark, light, terminal 16 colors and NO_COLOR; selected/focused/disabled/error together                                                      |
| Position         | First/middle/last item, scroll/list end, wrapped row larger than viewport, all panes folded                                                 |
| Authority / data | Read-only, pending-only, several unanswered requests, stale/partial/failure, retained draft                                                 |
| Usage            | Named no-data active window; HOME (`@all`) warmup omission, known zero, partial observed-total header; no named-squad summary on HOME/leads |
| Navigation       | Authored duplicate rows, pinned/overflow tabs, cached/uncached switch, search no matches, keyboard and clipped hits                         |
| Checklist        | Contract-admitted scopes/transitions, no items versus filtered empty, conflict/refusal and completed item with request still open           |

Evaluation tasks: identify the current squad; identify which item actually needs
Ben; predict the next key's destination; locate the selected row after refresh;
read the draft recipient/mode; distinguish no usage from measured zero; complete
a checklist item while retaining the open request. Record wrong answers,
ambiguous cues and any added scrolling for the same fixture. These are evaluation
tasks, not results or invented usability timings.

Complete per-surface mockups remain #1829 acceptance work. Native implementation
evidence belongs to the separately owned runtime children. The schematics above are not captures, measured terminal geometry, a full
mockup inventory or product acceptance. #1829 stays open until that coverage and
Squad current-behavior co-review support the specification. Runtime children
#1830–#1833 and checklist integration #1834 keep their separately owned gates.

## References and interpretation

- [Carbon color layers](https://carbondesignsystem.com/guidelines/color/overview/)
  motivates a small set of neutral surface levels; this terminal proposal adapts
  that idea rather than importing web shadows or CSS geometry.
- [Nielsen's usability heuristics](https://www.nngroup.com/articles/ten-usability-heuristics/)
  motivate visible scope/state, familiar wording, retained user control and
  discoverable actions. They do not prescribe this exact palette or layout.
- [W3C use of color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html)
  motivates marks and labels alongside color. This is a design principle, not a
  claim of WCAG certification for terminal output.

Current runtime owners and component mechanics stay in the linked area
references; acceptance history, per-run evidence and unresolved findings belong
to #1829 and its PR rather than this living specification.

### First implementation handoff: selected occurrence and role mapping

The first #1830 slice maps the existing selected-item, selected-word, semantic
mark and receiving-focus roles. Preserve the measured occurrence and viewport
intersection as the selected range; never extend across a pane boundary or
covered input band. A visible occurrence marker belongs in the existing row
prefix allocation, repeats on visible continuation lines, and does not replace
state/attention marks. If that prefix cannot accommodate it, return the exact
geometry conflict before changing row width or hits. Focus is identified at the
receiving pane title or input header, not inferred from the selected fill.

Keep the existing real-background selected-word policy and no-background
reverse/bold fallback in this first slice; explicit marks and labels must remain
sufficient when effects are ignored. Preserve user overrides and distinguish
true-color, terminal16 and Depth::None results. New canvas/content/band tokens
are not required merely to complete this mapping. Additional opaque surfaces
remain part of the following chrome/row/pane adoption, with concrete owners and
reviewed values rather than a second palette.

Use one pinned fixture definition for before/after comparison: same shown tab,
requested tab, stable member UUIDs and occurrence IDs, pending/request/state,
task bytes, bindings, meter evidence and viewport. Include a wrapped selected
row, selected-but-unfocused row, repeated occurrence and composer coverage at
80/100/160, plus an actual 180+ composition and a low-height clipped case.
Record the expected selection/focus answer for each specimen. Source-only or
illustrative review is labeled separately from rendered runtime evidence; no
participant timings are inferred. Zero/partial usage, refresh transitions,
checklist operations and other surfaces remain separately tracked coverage,
not claims made by this first role slice.

Squad is the accountable implementation owner; #1830 is currently unassigned.
Squad supplies the immutable runtime base, existing-seat assignment and exclusive
changed-file allocation before implementing. It owns hit/scroll/input regressions and scoped frozen
fixture disposition; UX reviews the corresponding rendered before/after
selection and focus. No acquisition, checklist, request, loading-lifecycle or
browser-package work is included. Whole-board #1825 acceptance remains open
through its existing chrome, views, overlays, guidance and combined-delivery
children.
