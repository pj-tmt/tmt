# Readability audit: squad

Source: `site/src/chapters/squad.mdx` at blob `afbff616289d151b01427b1454d67ff764af1dca` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L59: The board paragraph switches among layout, filtering, attention sorting, background lookups, live reload, manual refresh and nonterminal output; split it by reader task.
- L79 and L95: Token-window meanings and tab overflow/state marks are dense independent topics; each deserves a small legend.
- L195–197: Layout presets, width adaptation, legacy config preservation and key actions are packed together; separate view choice from custom configuration.
- L250–255: The column/line/value/percentage/wrap bullets are reference-sized paragraphs, especially L252. Separate field source, format, absence and coloring.
- L315, L343, L359, L443: Theme fallback, auto detection, observed age and binding grammar each mix several cases; use tables or named reference subsections.

## Terms before explanation

- L35–39: identity, selected lead, role and lead field have different meanings; explain the distinction before the first configuration detail.
- L77–89: token rate assumes optional usage hooks; link the prerequisite before the meter example.
- L138–160 versus L184–206: view, layout preset and hand-written board.layout are competing configuration concepts; define their distinct responsibilities and precedence before showing choices.
- L250–284: grow, span, from, field provider and overflow need a small vocabulary/annotated diagram before the exhaustive keys.
- L355–367: playbook, observed age and informational hooks need explicit “who runs/installs what” context.

## Repetition and ownership

- L59, L195 and L162–182 repeatedly describe the team board, detail/replies and narrow-width folds. Keep the default model near the first board sketch and the folding rules in one owner section.
- L95–106 and L426–439 repeat tab attention/order/hiding; keep daily interaction first and configuration reference later.
- L197, L282 and L443–462 repeat binding actions and precedence. Preserve the trust boundary and consolidate action grammar in the reference.
- L311–347 overlaps design.mdx; let design own token meanings and keep Squad-specific theme inheritance and auto behavior here.

## Where a table or diagram helps

- L59–75: Annotate BoardSketch with numbered rows/detail/replies/notes and a short everyday controls table.
- L79: Add a token-meter legend covering measured zero, missing coverage ≥, unreported members ? and blank trend; keep context versus completed usage separate.
- L156: A precedence diagram should distinguish custom geometry → per-squad view → board view → workflow layout, separate from theme precedence.
- L250–280: One row/grid/spanning-cell diagram and one nested-pane diagram would beat the largest reference bullets.
- L315–345: A theme-scope table plus auto light/dark/no-response decision flow would make the rules inspectable.
- L359–369: A first-observation → unchanged age → informational nudge timeline should preserve the prerequisite and no-earlier-history limits.

## Section order

- Use a progressive path: team roles/ownership, create a squad, see the board, daily controls and host-specific jump limits before customization.
- Keep presets/views together before hand-written tabs/sections/columns/fields.
- Move optional token-rate, observed-age and deep theme/reference material after the core board workflow.
- Keep all existing trust, explicit consent, fold lifetime, partial-result and no-authority constraints in the proposed reorganization.

## Proposed outline

1. What Squad owns: roles, human attention and no agent/worktree creation
2. Create/select a squad and its lead; get the board
3. Read the team board and mark legend
4. Everyday actions: talk, answer, annotate, search and refresh
5. Jump/back and host capability matrix; optional hotkeys
6. Notes, member detail and complete replies
7. Squad tabs and personal filtered tabs
8. Views versus workflow layout presets; precedence
9. Advanced geometry: panes, columns, row lines and wrapping
10. Field sources/providers, safe bindings and complete JSON/text values
11. States, token meanings and theme inheritance
12. Optional usage-rate meter and observed-age reminders
13. Lead skill and optional host playbooks
14. Reference: config scopes, tab pin/hide/order, refresh and action grammar
