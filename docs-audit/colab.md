# Readability audit: colab

Source: `site/src/chapters/colab.mdx` at blob `13028ff75c1b853b6f215ca53f7c65dded3526dd` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L14: The middle sentence assigns page maintenance, CLI publishing, browser reading, comments and decisions in one clause chain; separate the actor responsibilities.
- L18: The browser-to-listener sentence follows four sequential actions without visual stops; this is better as a flow.

## Terms before explanation

- L18: listener and local talk need a definition/link before the transport explanation.
- L28: pairing, grants and no-resend rules arrive together in the last line; explain the terms at the referenced boundary, without presenting a released feature.
- L25–26: Firestore, Workers and Durable Objects are backend names; readers need their role, not additional platform detail.

## Repetition and ownership

- The opening callout and L28 both say no backend has released; keep the prominent status gate and use the backend list for precise readiness.
- L14 overlaps the home Colab scene; the chapter can add actor/flow detail rather than repeating the pitch.

## Where a table or diagram helps

- L18: Browser → outward-connected local listener → talk → agent → reply → page is the strongest diagram opportunity.
- L22–28: A backend/status/audience table beats an ordered list when none is released; ordering can remain a roadmap column.
- L14: Keep the existing design sketch, but label who writes, reads and decides.

## Section order

- Retain the in-progress callout before any feature claims.
- Explain what the page is and its actors before the design sketch and transport.

## Proposed outline

1. In-progress scope and current availability
2. What the shared page is; who owns decisions
3. Design sketch
4. Browser-to-agent request/reply flow
5. Planned backend order and status
6. Security relationships: pairing, grants and uncertain delivery
