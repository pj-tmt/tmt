# Readability audit: design

Source: `site/src/chapters/design.mdx` at blob `08968bf4a346ebc33e61b4bee74b700aaf3cb504` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L5: The file/source explanation spans the product claim, guideline scope, source token path and generated consumers; separate the user principle from implementation provenance.
- L20: Token aliases, driver-color mapping and parity tests are packed together; replace the alias clause with a mapping table.
- L32: Typeface roles and command prompt/comment rules can be separate entries.

## Terms before explanation

- L10–11: NO_COLOR, truecolor, token and ANSI/16-color rendering need first-use definitions or a compact legend.
- L32: display/body/mono are font roles rather than necessarily literal font families; the TypePreview can label both.
- L44: Release status should remain distinct from whether an illustrative scene appears runnable.

## Repetition and ownership

- “One vocabulary” L12 repeats the opening uniformity claim; retain the principle and shorten the lead.
- The surface table repeats token values by design through one source; do not turn this into hand-written duplicated reference data.
- Color and mark principles overlap Squad’s theme introduction; this page should stay the canonical vocabulary owner.

## Where a table or diagram helps

- Keep TokenPreview, TokenTable, TypePreview and MarkList; these already beat prose.
- L20: A CLI alias → semantic token mapping table would make the synonym rules scannable.
- L44: A shipped/planned tag example beside a design sketch would illustrate status disclosure.

## Section order

- Keep principles first, then place state marks next to semantic colors before moving to surfaces/type.
- Close with status disclosure and token-source provenance.

## Proposed outline

1. Shared design principles
2. Semantic colors and mark vocabulary
3. Light/dark/terminal previews and token tables
4. CLI aliases and driver-color mapping
5. Web surfaces
6. Typeface roles and command notation
7. Shipped/planned disclosure
8. Single token source and verification links
