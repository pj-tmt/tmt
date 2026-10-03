# Readability audit: dev-extension

Source: `site/src/chapters/dev-extension.mdx` at blob `4bc2a059aed3b33fd0c2eb90c1ed543de73b0103` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L38: Completion call shape, literal candidates, time/output budgets and fallback behavior form a dense paragraph; use an input/output/limits table.
- L96: Hook approval, file ownership, post-commit timing, event shape, isolation and drop/repeat behavior need separate subsections.
- L98: The first sentence lists three injection destinations before the content limit and trust label; shorten the conceptual chain.

## Terms before explanation

- L4: PATH/executable are reasonable developer prerequisites but should be explicit.
- L26: stdin/stdout/stderr/TTY and process replacement need a compact glossary or prerequisite note.
- L57–59: operationId, receipt and revision need their local API meanings before the operations table.
- L96–98: observe event, provider hook and informational context contribution are distinct hooks; name the distinction.

## Repetition and ownership

- L42 and L55 repeat the no-database boundary; keep the rule at the API introduction and cross-reference it from storage.root.
- Arguments and direct execution are described in both the introduction and receives section; one concrete invocation diagram can own the model.
- The consent and no-authority rules are necessary; consolidate their repeated explanation without losing the explicit consent requirement.

## Where a table or diagram helps

- L18–34: An argv/environment/terminal/exit-status contract table would connect the receives terms.
- L53–60: Keep the operations table; add a dispatch create/recover state flow for idempotency.
- L88–98: Two flows should distinguish post-commit observe events from informational provider-context contributions.

## Section order

- Move the complete runnable example before optional completion so the core extension path is uninterrupted.
- Keep API contracts and explicit originator selection together; optional hooks and context contribution come last.

## Proposed outline

1. Prerequisites and executable naming/discovery
2. Invocation: arguments, terminal and TMT_EXECUTABLE
3. Complete minimal example
4. Optional completion contract and budgets
5. CLI JSON versus versioned API
6. Storage ownership, dispatch recovery and originator selection
7. Optional consented observe hooks
8. Informational context contribution and event limits
