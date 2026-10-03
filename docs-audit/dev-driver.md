# Readability audit: dev-driver

Source: `site/src/chapters/dev-driver.mdx` at blob `df52e69eff63def1e94f755a164446f1b4498e22` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L31: The invocation sentence combines executable naming, protocol arguments, request input and result output; split it into the wire-call parts.
- L48: Planned override rules include priorities, installation disclosure, discovery and name/claim conflicts in one dense paragraph.
- L56: Persisted-state invalidation lists four triggers after the storage rule; a trigger table would read better.

## Terms before explanation

- L9, L31–33: protocol, wire format and capabilities need concise definitions before the command envelope.
- L41: binding evidence should link to the concepts definition; avoid treating reported evidence as verified caller identity.
- L54: hooks here are provider lifecycle integration, while driver operations are separate invocations; make the distinction explicit.

## Repetition and ownership

- L6 says every driver is compiled in, then L8 says Herdr already runs externally; clarify the current built-in/external exception in the opening status summary.
- L39–44 and L52–56 overlap trust/evidence/storage rules. Group the invariant with its operational consequence once.
- L35 repeats the same-interface explanation from drivers.mdx; link the user overview and retain developer detail here.

## Where a table or diagram helps

- L31–33: A capabilities → selected protocol → bounded operation call diagram beats the compact wire-format paragraph.
- L39–44: A driver/tmt ownership table would separate untrusted evidence from verification/storage/consent.
- L55–56: Delivery outcomes and invalidation triggers belong in separate small tables.

## Section order

- Keep the planned scope first and explicitly separate the shipped Herdr path.
- Explain the two kinds and ownership before wire invocation; finish with planned overrides and publishing.

## Proposed outline

1. Current availability and planned package scope
2. Host versus runtime operations
3. tmt versus driver ownership and trust
4. Capabilities and protocol negotiation
5. Bounded operation request/result contract
6. Delivery outcomes and state invalidation
7. Planned built-in override rules
8. Publishing and conformance status
