# Readability audit: drivers

Source: `site/src/chapters/drivers.mdx` at blob `58ca656bb374eede2d12b69d3b2abb68d71d016c` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L14: The last setup sentence packs a settings preview, approval, per-provider selection and removal into one sentence; separate reversible actions from setup benefits.
- L43–48: Consumption, context, unavailable coverage and rate resets form a dense block even where sentences are short.
- L52: The package/protocol paragraph combines future distribution, community drivers, overriding and discovery; break the conceptual jobs apart.

## Terms before explanation

- L14: Explain a hook before saying setup adds provider evidence.
- L20: “shared server” is an exception in the comparison table before its meaning is introduced; link or briefly explain it.
- L48: epoch and gap are named as raw fields before their rate-calculation meaning is explained.
- L37, L52: Consent and replacement rules differ between shipped Herdr and planned runtime packages; keep their status explicit.

## Repetition and ownership

- L4 and L24–26 repeat the kind definitions; use the comparison table as the main overview and links as navigation.
- L14 overlaps working.mdx L150–160 and the two provider connection sections. Keep this page focused on choosing a driver and setup benefits.
- L41–48 is the useful canonical usage explanation; provider pages should retain only their differences and link here.

## Where a table or diagram helps

- L16–20: Keep the existing before/after setup table.
- L37: An approved → changed → re-approved driver diagram would explain why upgrades can retain consent or require it again.
- L43–48: A context-versus-completed-consumption table and a small rate-baseline timeline would clarify missing counters and resets.

## Section order

- Put all shipped drivers, including Herdr with its separate approval, together before optional usage details.
- Keep future packages and planned runtime drivers at the end.

## Proposed outline

1. What host and runtime drivers do
2. Built-in drivers and the setup comparison
3. Provider guides
4. Herdr approval and changed-driver handling
5. Optional usage: context versus consumption
6. Reading unavailable counters and resetting rates
7. Planned driver packages; link to development
