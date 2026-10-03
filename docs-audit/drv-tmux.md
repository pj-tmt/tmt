# Readability audit: drv-tmux

Source: `site/src/chapters/drv-tmux.mdx` at blob `bce18dac6f51c59c018cbae56d5003216618c559` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L37: The uncertainty sentence embeds the paste-versus-Enter example inside the result; separate the outcome from the example.
- L55: The sandbox paragraph presents refusal, diagnosis and two fixes together; make the observed error and next action scannable.

## Terms before explanation

- L10, L23: pane metadata and pane-border-format assume tmux familiarity; link to a pane/border primer or explain their role.
- L15–16: Distinguish the shell-mode protection character substitution from byte-for-byte delivery.
- L51: “recorded session state” should point to the state meanings and their activity limits.

## Repetition and ownership

- L23 and L51 repeat unchanged titles/layout/theme; retain the boundary near the badge configuration.
- L37 repeats working.mdx delivery uncertainty and L55 repeats settings troubleshooting; summarize and link to the canonical recovery owner.

## Where a table or diagram helps

- L37: A validated → pasted → Enter-confirmed diagram with the uncertain edge would make partial delivery concrete.
- L43: Keep BadgeBorders; align each state with the concepts state table.
- L47–51: A before/after border fragment with the insertion point beats explaining several tmux options in prose.

## Section order

- Lead with the existing driver card and delivery limits, then badge setup, then troubleshooting links.
- Keep the badge state warning adjacent to the badge pictures.

## Proposed outline

1. Host-driver capabilities and limits
2. Verified pane delivery and uncertainty
3. Optional badge: states and appearance
4. Enable the badge and insert the format fragment
5. Recorded state versus activity
6. Socket access refusal and the settings troubleshooting link
