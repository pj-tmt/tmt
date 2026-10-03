# Readability audit: meet

Source: `site/src/chapters/meet.mdx` at blob `c2f5628db5baaaf69d4b055feef494cee7184d97` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L18: The floor rule combines explicit grants, raised hands and agent mentions; split the human action from the agent action.
- L24: The second sentence maps floor, talk, reply, wake-up and quota into one explanation; separate the protocol model from its consequence.

## Terms before explanation

- L15–20: Define “floor” and “hand” once before describing controls and modes.
- L24: room and transcript appear late; link the core room concept when first describing the meeting.
- L19–20: “turn” is potentially a speaking turn rather than a provider/model turn; make its meaning clear.

## Repetition and ownership

- The callout and L30–32 both discuss availability; keep the callout as the gate and the last section as future scope.
- “You decide who speaks” at L13 is restated in L17–18; a compact rule introduction can lead to the detailed controls.

## Where a table or diagram helps

- L17–20: A raised hand → grant → speaking → return-of-floor state diagram would beat four disconnected rules.
- A controls/action/authority table can separate g, @name and m while keeping agent mentions distinct.
- L24–28: A request/reply/transcript/confirmed-follow-up diagram would show why a recorded decision does not execute by itself.

## Section order

- Keep the planned callout first, then explain membership and the floor vocabulary before the controls.
- Explain ending/confirmed follow-up before the underlying room/talk implementation.

## Proposed outline

1. Planned scope; part of Squad
2. Meeting topic and participants
3. Floor and raised hands
4. Host-granted and first-come modes
5. End the meeting: transcript and confirmed decisions
6. Room/talk/reply foundation
7. Later web view and voice
