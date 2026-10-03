# Readability audit: settings

Source: `site/src/chapters/settings.mdx` at blob `c896a4de0f5eddb19ca233300d9d55d40fa1d949` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L31: The file/source paragraph combines global/folder scope, former naming, path discovery and opaque fields; separate scope precedence from file locations.
- L48: CLI versus extension pins, consent, noninteractive behavior and partial failures belong to distinct decisions.
- L111: Permissions, no mutation workaround and identical-reply retry are several separate recovery rules.

## Terms before explanation

- L20–21: Preamble is used before being explained here; link to notes/roles/preambles in working.
- L31, L48: Override, channel and pin need concise meanings before file/update examples.
- L57, L67: compinit and push-default are specialist shell/tmux terms; label them as advanced prerequisites.

## Repetition and ownership

- Config show/source behavior is stated in both L8 and L31; keep one scope explanation next to the settings table.
- Update behavior repeats extensions.mdx; this should own the common CLI/channel/pin policy and link extension-specific details.
- Badge extras overlap drv-tmux; keep the advanced rules here with the existing basics link.
- Sandbox guidance repeats drv-codex; retain common errors/retry rules here and provider-specific configuration there.

## Where a table or diagram helps

- L18–29: Keep the setting/default/scope table; replace ambiguous “either way” with explicit local/global terminology at the table level.
- L31: A command/local/global/default precedence diagram plus file-path table would make source selection clearer.
- L48 and L115–118: Separate update outcomes and request-error next actions into two case tables.

## Section order

- Separate everyday configuration/updates from advanced shell and pane styling.
- Place troubleshooting error cases under a clearly scannable common entry before advanced customization, or provide direct navigation.

## Proposed outline

1. Inspect settings and source precedence
2. Set/remove overrides; value/scope table
3. Update: CLI and extension channels/pins
4. Troubleshooting: missing binary, skill conflicts, blocked storage/socket, request outcomes
5. Shell completion
6. Advanced pane badge styling
7. Theme selection links
