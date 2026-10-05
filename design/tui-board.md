# Squad board redesign proposal

**Status: design draft for #1829 under #1825.** The accepted direction is flat
color blocks with purposeful highlights and no shadows. This document proposes
the complete presentation system; it does not describe a delivered redesign.
Squad owns runtime behavior and checklist data. UX owns this specification and
visual acceptance. The browser package (#1797) is a separate route.

## Design outcome

A reader should recognize the current squad, keyboard destination and item that
needs a response without scanning every field. Keep task and reply content
readable; spend saturated color on small meaningful cues. Neutral surfaces group
content, while text, marks and selected geometry remain useful without color.

The existing [full-screen interaction rules](cli-style.md#full-screen-interaction)
and [board behavior owner](../.agents/skills/tmt-squad-dev/references/board.md)
remain authoritative. This proposal changes presentation, not send, request,
configuration, membership or persistence authority. Effective bindings supply all
displayed shortcut hints. Specific current composer and host behavior takes
precedence over the generic key examples in the style guide.

## Flat surfaces and semantic roles

Use one quiet canvas, one neutral content surface, an opaque inline band and the
existing selection treatment. Content surfaces have square edges and no offset
shadow cells. Avoid a colored card for every state or separate frames around
each field. A focused pane is recognizable from its title and focus marker;
selecting a row does not imply that pane currently receives keys.

| Meaning             | Presentation target                                       | Non-color cue                                                   | Existing owner / next boundary                                     |
| ------------------- | --------------------------------------------------------- | --------------------------------------------------------------- | ------------------------------------------------------------------ |
| Canvas              | Quiet background outside content blocks                   | Spacing and reading order                                       | Theme/screen adapter; proposed new surface roles need #1830 review |
| Content surface     | Neutral block, shared by rows and reading panes           | Section title; one boundary when adjacent panes need separation | `Outline` / scene composition                                      |
| Selected item       | One continuous background over the complete wrapped item  | Existing cursor/selection fallback                              | `Look::row_span`, `selected_words`                                 |
| Keyboard focus      | Accent and bold pane title; explicit focus cue in mockups | Focused title with a distinct focus label                       | App focus, pane title / modal focus stack                          |
| Waiting on the user | Warm attention mark; ordinary readable words              | `◆` plus `waits on you`                                         | Acquired attention predicate; never checklist-derived              |
| Working             | Green mark, quiet body text                               | `●` plus `working`                                              | Admitted state; not a heartbeat                                    |
| In review           | Review mark / role                                        | `◐` plus `in review`                                            | Admitted state; not waiting on Ben                                 |
| Blocked or failed   | Error mark plus one actionable sentence                   | `✗` plus reason                                                 | Existing failure owner                                             |
| Checklist complete  | Small success cue, retained item title                    | `[✓]` plus `done`                                               | Proposed presentation; allowed state from #1826                    |
| Muted / unavailable | Secondary labels; readable inherited contrast             | `–`, empty phrase or explicit source label                      | Existing `muted` / `dim` roles                                     |
| Disabled action     | Muted label with explicit unavailable reason              | Disabled marker; activation refused                             | Existing admission/controller                                      |
| Link                | Link role and underline where supported                   | Link label and effective open hint                              | Existing link validation / effects                                 |

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

### Terminal degradation

- True color: neutral canvas/content separation, small semantic accents and
  selection contrast. The accepted private prototype is direction, not a token
  implementation or a contrast certification.
- Terminal 16 colors: respect the user's palette. Default foreground/background,
  bold titles, boundaries and marks must carry grouping when two neutral fills
  cannot be distinguished. No exact RGB contrast claim for arbitrary palettes.
- NO_COLOR: no colored fills or semantic color. Preserve labels, marks, spacing
  and the existing reverse/bold selection fallback; focus remains distinct from
  selection. An error retains both its mark and its explanation.
- No decorative blink, pulse, gradient or shadow. Existing meaningful loading and
  observed meter updates retain their own lifecycle.

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

Mode cycling retains answer/note/talk drafts and the admitted recipient. With
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
