# Extension surface: command dispatch, API, MCP and hooks

Code-side rules for what core exposes to extensions. Wire shapes and limits are in
[extension-api](../../../../contracts/extension-api.md) (including "Lifecycle hooks") and
[mcp-v1](../../../../contracts/mcp-v1.md); the ownership summary is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#public-command-boundary).

## Command dispatch and help

- `tmt-cli/src/grammar.rs` owns the ordered core registrations, shared spec/option helpers
  and help projection; private `grammar/` modules own builders by group and do not import each
  other. Each visible core command registers from a `CommandSpec` through
  `tmt_cli_style::apply`; hidden commands have no help page. Every public core option has a
  nonblank single-line description, checked recursively. Core help never enters runtime
  dispatch or skill-drift inspection.
- The help scan shares option-value boundaries with error-mode recovery so `-h` bypasses
  required operands without reading payload data as flags. `OutputMode` holds only JSON;
  unsupported `--verbose`/`--debug` fail with `USAGE_ERROR` before effects.
- Core names and aliases are reserved before external PATH dispatch; the recursive listing
  guard in `tmt-cli-style::audit` covers core, Office and Squad.
- `skill_reminder` prints at most one best-effort stderr line after a successful result and
  never changes JSON or raw stdout; `TMT_HINTS=off` disables transition hints, not error
  recovery or drift reports.
- `output::table` is the one plain human-table renderer (escaping, Unicode width, no
  truncation or color); JSON and exact message bodies bypass it.

## External commands (v1)

- PATH enumeration happens only for root help, root completion and unknown-command
  suggestions; exact dispatch probes one filename. `tmt help <extension>` runs
  `tmt-<extension> --help`.
- Completion v1 (`__complete`) runs through the bounded process owner with a one-second
  deadline and 64 KiB bound; shells quote candidates and never evaluate them.

## Local API and MCP

- `tmt-adapters::api` owns envelope admission and composition; the facade `api.rs` keeps the
  dispatcher, bounds, settings selection and storage lifetime, and private `api/` modules own
  operation families. Capabilities and unsupported-version discovery never open storage.
  Explicit identity selects write attribution, not privilege.
- `rooms.roster` reads membership, metadata and status from one deferred SQLite snapshot and
  excludes presence (host observation, `ls --room --json`). `identity_projection` gives CLI
  and API identical identity bytes. `identityHooks.*` are scoped to the named consumer;
  another consumer's hook is `HOOK_NOT_FOUND`.
- History reads never acknowledge or renew retention; dispatch operation IDs recover
  immutable acceptance and replay never wakes again.
- `tmt-cli::mcp_command` pins one saved identity UUID and data root and composes the existing
  command owners in process with the same JSON encoders. API dispatch distinguishes local name
  lookup from an exact saved UUID so retirement cannot retarget a pinned writer.

## Extension hooks

- `tmt extension hooks enable <name>` records path, SHA-256, stat fingerprint and capabilities
  in `<global>/extension-hooks.json` (0600, atomic replace); ownership and fingerprint are
  re-checked before every delivery and a change skips the extension until re-enabled.
- Capture installs temporary triggers in `Storage::open` only for the `tmt` CLI with an
  enabled `lifecycle_observations_v1` observer; the temporary table follows the transaction,
  so a rollback leaves no evidence. Delivery runs after the command under one aggregate
  deadline, its output is discarded and no failure changes the result. `TMT_HOOK_DELIVERY=1`
  stops nested capture.
- Context contributions (`context_v1`, via `context_command::verified_document`) are asked
  only for a verified bound identity, are untrusted and are dropped before role or notes when
  the 4 KiB context bound bites. Provider `UserPromptSubmit` hooks reuse the same budget and
  need a running, verified binding that matches the caller.
- Without a consent file a command reads it at most once, on first storage open, and spawns
  nothing.
