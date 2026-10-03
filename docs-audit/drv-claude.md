# Readability audit: drv-claude

Source: `site/src/chapters/drv-claude.mdx` at blob `a9b20d9dd287fd5dd0da493c0642d1a70df12d69` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L48: Several short sentences still form a dense compatibility/status block; separate version requirements, unfamiliar-build notice and launch-local changes.
- L52: The recovery bullet combines record ownership, process checking, inspect/recover actions and repeat safety; make it a bounded sequence.

## Terms before explanation

- L10, L32: hooks and lifecycle event names precede a description of what each event records.
- L50: “enrolled” needs a definition when channel enrollment first appears.
- L51: “No receipt” is easily confused with tmt reply receipts used everywhere else. Distinguish missing transport acknowledgement from the required request/reply correlation receipt.

## Repetition and ownership

- Driver card L15–20 and “What you get” L36–38 repeat resume/context benefits; one overview can link to the concrete setup.
- L50–52 repeat no paste/no resend across multiple failure cases; retain the invariant once and place cases in a table.
- Usage field definitions are correctly owned by drivers.mdx; keep only the Claude-specific differences here.

## Where a table or diagram helps

- L48–52: A channel compatibility/enrollment/delivery outcome table would help readers choose safe next actions.
- L52: A crash-record inspection flow should keep conclusive process absence as the recover condition.
- L32: A three-event hook table can connect session start/end/prompt-submit to their effects.

## Section order

- Setup and default delivery should precede optional channel details.
- Put requirements before the channel launch command, and failure/recovery immediately after it.

## Proposed outline

1. Claude runtime driver overview
2. Connect/remove hooks and normal launch/resume
3. Default paste delivery
4. Optional channel: compatibility and enrollment
5. Transport acknowledgement versus reply receipt
6. Failure cases and crash-record recovery
7. Claude usage-counter differences
