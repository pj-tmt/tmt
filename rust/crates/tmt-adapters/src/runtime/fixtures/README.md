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

## Turn-end usage (#519)

- `claude-assistant-usage.jsonl` and `codex-token-count.jsonl`: **real, minimized
  by tmt-lead** on 2026-09-30 from this project's own sessions. Every field except
  the usage structure and its envelope was dropped, and text content was replaced
  with `<redacted>`. The Claude line comes from Claude Code with model
  claude-opus-5-5; the Codex line is a rollout `event_msg` of type `token_count`.
- Every other transcript line in the tests is **assembled** from these two, by
  editing one field (for example a sidechain flag, a diverging `iterations[]`, a
  null `info`, or reasoning tokens).
- Claude usage is the top-level `message.usage`: `input_tokens +
  cache_read_input_tokens + cache_creation_input_tokens`. The line also carries
  `iterations[]`, whose single entry equals the top level here; a test pins that
  the top level is read when they differ. Sidechain and `<synthetic>` messages are
  skipped. The window is not in the transcript, so none is stored.
- Codex usage is `last_token_usage.total_tokens`, with `model_context_window` as
  the window, as Codex's own status display reads it (openai/codex
  `codex-rs/tui/src/token_usage.rs` at b1e7296, 2026-09-27: "the latest active
  context size"). Reasoning output is not subtracted, and `cached_input_tokens`
  is part of `input_tokens` and is not added.
- Both formats are unofficial. The Claude hooks reference says the transcript is
  written asynchronously and may lag the current turn, so a recorded value can be
  one turn old. Codex says its transcript "isn't a stable interface for hooks".

## Resume argv placement

The resume commands follow each CLI's usage line. These were observed read-only
with `--help` under a disposable `HOME` (and `CODEX_HOME`) on 2026-09-29:

- codex-cli 0.158.0, `codex resume --help`: `Usage: codex resume [OPTIONS]
  [SESSION_ID] [PROMPT]`, with `-m, --model <MODEL>` among the resume options.
  The driver emits `codex resume -m <model> <session>`.
- Claude Code 2.1.284, `claude --help`: `Usage: claude [options] [command]
  [prompt]`, with the `--model <model>` and `-r, --resume [value]` options. The
  driver emits `claude --resume <session> --model <model>`.
