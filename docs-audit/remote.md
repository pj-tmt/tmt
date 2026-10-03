# Readability audit: remote

Source: `site/src/chapters/remote.mdx` at blob `fb2be5df056de1865d6335a235649da068570931` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L4: The remote-talk sentence combines destination types and three delivery invariants; separate the capability from the safety rules.
- L29: The final sentence contrasts absent listener, next start/action and absent extension; a case table would be clearer.

## Terms before explanation

- L4, L8, L16: backend, pair, listener and end-to-end encryption need brief roles before they become architecture claims.
- L25: durable log and subscription cursor are introduced after the backend list but before any flow model.
- L35–38: Distinguish a machine key, a sender identity and a per-agent grant/approval; they are different decisions.

## Repetition and ownership

- L4 and L25 repeat transport-independent behavior. Keep the user-facing invariant once and use the later section for the backend contract.
- L33 and L37 repeat the authority distinction; retain the rule but tie it to one explicit sender/recipient model.

## Where a table or diagram helps

- L18–23: Keep the backend comparison table, adding proposal/readiness context rather than implying all are usable.
- L29: A listener on/off × agent active/idle outcome table would clarify queueing and wake-up.
- L35–39: A machine pairing → permitted sender → agent policy → delivery flow would make the trust boundaries visible.

## Section order

- The route badge marks Designing and the command caption says Proposal, but L4, L16 and L29 use present-tense capability claims. Put the proposal scope before the example and carry it into the backend/security sections. This is an editorial status-consistency finding, not a runtime verification.
- Define the two-machine flow and listener before asking readers to choose a backend.

## Proposed outline

1. Designing/proposal scope and non-final commands
2. What remote talk preserves
3. Machines, identities and the local listener
4. Proposed pairing and request/reply flow
5. Backend comparison and readiness
6. Permission and per-agent delivery decisions
7. Offline, uncertainty and recovery cases
