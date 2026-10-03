# Readability audit: working

Source: `site/src/chapters/working.mdx` at blob `d567c04eda787132fcce70e98d9282c7eb2ecb96` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L9: The run introduction packs option ordering, remembered data, job control and exit status into one block; separate invocation grammar from lifecycle.
- L60: The talk/reply paragraph includes completion, correlation, retention, deadlines, result retrieval and detached notices; split these jobs into named subsections.
- L127, L129, L131: Delivery bullets are paragraphs carrying multiple conditions and next actions; convert the outcomes to a table and keep hooks as an advanced note.

## Terms before explanation

- L16–18: channel and temporary identity appear before this chapter gives the identity/lifetime overview.
- L103–113: X, revision and acknowledgement need a short model before their command examples.
- L135–139: notes, role and preamble are defined well but come after long examples that already use identity/context.

## Repetition and ownership

- Run/name examples overlap start.mdx L75–94. Keep the home walkthrough brief and let this chapter own invocation and recovery cases.
- Resume guarantees repeat at L22–33 and provider pages; retain common lifetime/retry semantics here and provider differences there.
- Setup and delivery uncertainty appear again in driver/settings chapters; link to the owning section rather than repeating full rules.

## Where a table or diagram helps

- Keep MessageTravel as the overview, followed by a request → delivery → stored reply → result/ack diagram.
- L22–33: A resume state table for available/missing/stale/forget/retry beats a sequence of paragraphs.
- L60–85, L103–131: Distinguish sender result recovery, recipient inbox answering and transport uncertainty in separate case tables.
- L135–139: A notes/role/preamble storage/injection table would clarify which text is added to messages.

## Section order

- After MessageTravel, establish name/lifetime/caller basics and a minimal talk/reply loop before advanced launch flags.
- Move channel flags, detailed notice batching and hook nudges after the core request/recovery story. Keep destructive uninstall at the end.

## Proposed outline

1. Request/reply overview
2. Identity, saved lifetime and explicit caller selection
3. Launch, name and exact resume
4. Send work and submit the correlated reply
5. Sender result recovery and X acknowledgement
6. Recipient inbox and answer
7. Delivery outcomes: paste, channel, queue-only, uncertainty, approval
8. Notes, roles and preambles
9. Setup and optional usage/hooks
10. Uninstall and retained data; settings links
