# Provider hook payload fixtures

These fixtures pin the provider fields the first-party drivers read. Each driver
keeps only the fields it needs, and a `model` field is optional input. Update a
fixture only from the provider's documentation or a recorded payload, and keep
its source listed here.

- `claude-session-start.json`: the SessionStart input example from the Claude
  Code hooks reference (<https://code.claude.com/docs/en/hooks>), verbatim,
  retrieved 2026-09-29. The reference documents `model` as the active model
  identifier. It can be omitted, for example after `/clear` or after conversation
  recovery.
- `codex-session-start.json`: assembled from the field tables in the Codex hooks
  documentation (<https://learn.chatgpt.com/docs/hooks>), retrieved 2026-09-29.
  The common fields `session_id`, `transcript_path` (`string | null`), `cwd`,
  `hook_event_name` and `model` ("Codex-specific extension. Active model slug")
  appear together with SessionStart's `source` and `permission_mode`. The IDs and
  values are placeholders, because the documentation gives no example payload.
