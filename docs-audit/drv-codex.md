# Readability audit: drv-codex

Source: `site/src/chapters/drv-codex.mdx` at blob `fc0f8b7b6a08b2b55775302b0aa147fb1c986ec6` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L41: The delivery paragraph adds queueing, typing behavior, process ownership and cleanup to the opt-in explanation; separate the choice from ownership.
- L50: Version compatibility, the current fresh-attachment regression and no-fallback behavior are tightly packed; present requirements and limits separately.
- L56: The sandbox paragraph combines configuration examples, escalation and approval policy; separate diagnosis from alternatives.

## Terms before explanation

- L10–16: thread, shared app server and exact match need an early relationship diagram; distinguish a thread from a pane and process.
- L41: private app-server/supervisor are implementation terms; explain only why they matter to launch lifetime and cleanup.
- L56: workspace-write and writable_roots need a link to the provider configuration rather than an assumed vocabulary.

## Repetition and ownership

- The card and setup sample repeat exact resume; the card can remain a concise capability summary.
- L50 and L52 both contain safety limits; group channel readiness, trust prompts and no-paste fallback by outcome.
- The data-directory recovery duplicates settings.mdx L104–111; keep the Codex-specific permission setting here and link to the common errors.

## Where a table or diagram helps

- L10–16: pane ↔ identity ↔ exact thread, with multiple threads on a shared server, explains the caller ambiguity.
- L41–52: A plain/channel launch-and-resume comparison would keep the current version limitation visible before execution examples.
- L56: A storage error → unchanged data → approved access/escalation flow would clarify retry conditions.

## Section order

- Introduce standalone versus shared-server caller selection before setup or examples requiring identity.
- Move the known channel regression before the opt-in command block, retaining its explicit version.

## Proposed outline

1. Thread and identity overview; standalone/shared server
2. Connect/remove lifecycle hooks
3. Launch and exact resume with plain delivery
4. Optional channel: requirements and current limits
5. Readiness, trust prompt and cleanup behavior
6. Sandboxed storage access
7. Codex usage-counter differences
