# Readability audit: extensions

Source: `site/src/chapters/extensions.mdx` at blob `573180706bac968a9ebb6f0924b5796d2c3b47f5` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L4: The opening couples PATH discovery, a gcloud analogy, command precedence and integration APIs in three dense sentences; separate discovery from authority.
- L24: Updates combine per-extension channels/pins, one consent, partial failures and unpinning; make each outcome independently scannable.

## Terms before explanation

- L4: PATH and hooks need links or brief explanations; gcloud is an unexplained comparison that may add more vocabulary than it removes.
- L24: channel and pin are used before this chapter defines them; link directly to updating settings.
- L26: Distinguish agent instruction skills from command registration.

## Repetition and ownership

- L4 and L26 repeat that any PATH executable works. Keep discovery in one place and installation as a separate official-package convenience.
- L24 overlaps settings.mdx L39–48; let the settings chapter own update policy and keep extension-specific behavior here.
- L32 mentions members in their own worktrees, while squad.mdx L41–44 explicitly says Squad does not create them. Preserve the architectural distinction in the proposed wording.

## Where a table or diagram helps

- L17–21: A command/action/data-retention table would complement the examples.
- L30–35: The existing known-extension table is the right format; preserve status marks and distinguish Meet as part of Squad.
- L24: A compact update outcome table would show successful, consent-required and failed components without suggesting rollback.

## Section order

- Lead with the known extensions and status table so readers can choose a capability.
- Explain discovery/precedence, then installation and separate upgrade behavior.

## Proposed outline

1. Known extensions and shipped/planned status
2. How command discovery works; built-ins win
3. Install and optional agent skills
4. List, update and remove
5. Per-extension channels, pins and partial failure
6. Build your own extension
