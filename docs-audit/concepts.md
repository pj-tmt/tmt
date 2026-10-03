# Readability audit: concepts

Source: `site/src/chapters/concepts.mdx` at blob `a388f87fd84d2b8665dc35288cba29bd7eee3a75` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L8–9: The identity definition is short but puts “durable” beside temporary retirement; separate the common definition from the saved/temporary contrast.
- L18–19: The driver definition combines hosts, runtimes and hooks in one explanation; keep the two kinds visually distinct. No unusually long prose sentence dominates this short chapter.

## Terms before explanation

- L8–9: Saved versus temporary is introduced inside a parenthesis; make the lifetime distinction explicit before using “durable.”
- L13–14, L19: “caller,” “hooks,” and provider conversations need a short explanation or links at first use.
- L50–54: “X,” originated/incoming, and unacknowledged appear in output without an explanation of the next action.

## Repetition and ownership

- L16–19 repeats the opening definition in drivers.mdx L4. Keep this as the brief concept definition and link to the driver chapter for the full host/runtime model.
- L30–45 overlaps provider driver pages; retain the state meanings here and link from those pages.

## Where a table or diagram helps

- L5–26: A name → binding → host/runtime diagram would connect the four otherwise separate definitions.
- L32–45: The existing state rows are effective; add a conceptual separation between recorded session state and readiness/activity, rather than treating the badge as activity.
- L50–54: An annotated context sample should identify identity/lifetime and the two attention directions, with a link to recovery.

## Section order

- Define identity, lifetime and binding together before driver and extension.
- Follow the state table with a short context-output reading guide.

## Proposed outline

1. Identity and saved/temporary lifetime
2. Binding and caller selection
3. Drivers: host and runtime
4. Extensions
5. Recorded states and their limits
6. Read the session context; links to working and recovery
