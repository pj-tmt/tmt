# Provider hook payload fixtures

## Ordinary caller session discovery (#2131)

- `claude-caller-session.json`: real-session environment shape supplied by
  tmt-lead in request `req_53a06c6f`, 2026-10-09, Claude Code 2.1.288
  (`CLAUDE_CODE_EXECPATH` ends in `claude/versions/2.1.288`). The UUID is the
  supplied session ID; the positive decimal PID is normalized. The documented
  tool variables locate the main resumable conversation. Native PID equality
  and ancestry authorize recording; this path reads no Claude transcript files.
- `codex-caller-session.json`: captured on 2026-10-09 from core-1's natively
  verified running codex-cli 0.160.0 session. Version is corroborated by both
  that exact index row and its rollout header `cli_version`. The installed
  standalone package is 0.161.0; installation alone does not prove the version
  of an already running process. No 0.161.0 session was captured. Only
  `CODEX_THREAD_ID`, `PRAGMA table_info(threads)`, one parameterized exact-thread
  index projection, and the first bounded rollout metadata header were inspected.
  UUIDs are replaced
  consistently by a UUID of the same version; the private path is normalized.
  Prompt, instruction, user-path and transcript fields are omitted. These are
  implementation formats, not documented compatibility promises. Unknown
  index/header/env shapes silently leave the session unrecorded; mutation tests
  derive incompatible shapes from these recorded fixtures.

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

## Prompt-submit context (#652)

`claude-prompt-submit.json` and `codex-prompt-submit.json` are assembled from
[Claude's UserPromptSubmit contract](https://code.claude.com/docs/en/hooks#userpromptsubmit)
and [Codex's hook contract](https://learn.chatgpt.com/docs/hooks#userpromptsubmit),
retrieved 2026-10-01. IDs, paths and prompt text are placeholders. Local versions
were Claude Code 2.1.285 and codex-cli 0.159.2; Codex's generated first-party
schema also lists `userPromptSubmit`. These are documented wire fixtures, not
recordings from a model run. `runtime::prompt_tests` pins event/session decoding,
provider-specific context output, bounds and rejection of other event shapes.
They establish context delivery only; activity ordering is owned by #656.

### Main-turn activity source contract (#656)

Source review on 2026-10-01: Claude Code 2.1.286 and codex-cli 0.159.2 installed
version commands; official [Claude hooks](https://code.claude.com/docs/en/hooks)
and [Codex hooks](https://learn.chatgpt.com/docs/hooks) contracts. These are
versioned contract fixtures, not captured model sessions. No model was invoked.

Both providers report UserPromptSubmit before prompt processing and Stop when
the main agent finishes responding. SubagentStop and other events are not mapped.
Codex supplies the active `turn_id` on both events; its Stop continuation creates
a new prompt. Claude supplies no documented turn ID here: ordering relies on
synchronous execution of TMT's setup-written command hooks and committing before
return. Tests model that contract; they do not prove the provider implementation.

Claude documents that async hooks receive identical input. TMT setup never writes
async hooks; changing an owned entry to async is reported as an edited hook by
setup inspection. Such a modification violates this source contract. TMT does not
infer execution mode from payload fields or timing and does not read all effective
settings on every event. Expired calls cannot defer activity work beyond return.

## Completed-request consumption (#872)

`claude-usage-sequence.jsonl` is a real 24-record Claude Code sequence supplied
and minimized by tmt-lead on 2026-10-02. Content was removed (only content block
types remain); uuid/`requestId`/`message.id` were replaced by consistent synthetic
IDs preserving equality. Usage objects are verbatim. There are 16 distinct
message IDs, with identical contiguous duplicates for content blocks.
No provider files were read by the implementer. The first message is the
baseline; independently summed totals for the remaining 15 IDs are input
5,415,987, output 5,609 and cache-read 5,405,674. Cached input is part of input.
The existing real Codex fixture supplies `total_token_usage`: input
2,674,657,871, output 6,910,968, cached input 2,624,251,008 and total
2,681,568,839; reasoning is already included in output.

Failure/partial-line/decrease/replacement and noncontiguous-repeat variants
are assembled from these structures. Last-ID-only deduplication assumes
append-only, contiguous content-block groups as observed in this sequence,
not arbitrary historical deduplication. Its known limitation is documented in
ARCHITECTURE. Both provider formats remain unofficial.

Consumption attribution tests reuse these minimized usage objects. Codex turn
contexts and cache-write variants are assembled from the official **0.160.0**
source at commit [`a956835d020762cb2b570053af06f643a11c0ecc`](https://github.com/openai/codex/blob/a956835d020762cb2b570053af06f643a11c0ecc/codex-rs/protocol/src/protocol.rs):
`TurnContextItem.model`, `TurnStartedEvent.turn_id` (`task_started` /
`turn_started`) and `TokenUsage.cache_write_input_tokens` (with the existing
`event_msg` / `token_count` and `turn_context` rollout envelopes). They are synthetic
wire variants, not additional live-session captures. Rate-limit model aliases are
never used for request attribution. Claude variants retain the recorded assistant
`message.model` and `cache_creation_input_tokens` shapes; no user transcript, model
call or credential is needed. Missing or malformed model fields and missing cache-write attribution stay
unknown. Existing legacy counter validation remains unchanged.
