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
  dispatch or skill-drift inspection. Unknown help topics name the nearest resolved public
  parent's help command; error code, presentation mode and JSON fields stay unchanged.
- Unknown-argument diagnostics reuse that scan's nearest canonical public path for a
  human-only help hint through the existing suggestion renderer. Hidden commands,
  values, `--` data and launch tails never select the hint topic; JSON keeps the
  original parser message and fields. Clap's typed subcommand suggestions omit hidden
  registrations and their aliases; when all candidates are hidden, human errors use
  the same public-scope hint without changing the offending token or JSON fields.
- The help scan shares option-value boundaries with error-mode recovery so `-h` bypasses
  required operands without reading payload data as flags. Help with `--json` reaches
  `finish_parse` and returns `JSON_UNSUPPORTED` before operand validation. `OutputMode` holds only JSON;
  unsupported `--verbose`/`--debug` fail with `USAGE_ERROR` before effects.
- Core names and aliases are reserved before external PATH dispatch; the recursive listing
  guard in `tmt-cli-style::audit` covers core and Ops.
- `skill_reminder` prints at most one best-effort stderr line after a successful result and
  never changes JSON or raw stdout; `TMT_HINTS=off` disables transition hints, not error
  recovery or drift reports.
- `output::table` is the one plain human-table renderer (escaping, Unicode width, no
  truncation or color); JSON and exact message bodies bypass it.

## External commands (v1)

- PATH enumeration happens only for root help, root completion and unknown-command
  suggestions; exact dispatch probes one filename. `tmt help <extension>` runs
  `tmt-<extension> --help`; `tmt help <extension> <tail...>` forwards `<tail...> --help`.
  Tail flags, including `--json`, belong to the extension; Core names and aliases stay reserved.
- Installed-extension prefix refusals recover presentation mode only from the stored
  Core prefix using the diagnostic scanner, and never inspect or execute the extension tail.
- Completion v1 (`__complete`) runs through the bounded process owner with a one-second
  deadline and 64 KiB bound; shells quote candidates and never evaluate them.

## Local API and MCP

- `tmt-adapters::api` owns envelope admission and composition; the facade `api.rs` keeps the
  dispatcher, bounds, settings selection and storage lifetime, and private `api/` modules own
  operation families. Capabilities and unsupported-version discovery never open storage.
  Explicit identity selects write attribution, not privilege.
- `identity.rename` admits an exact UUID/name CAS through the binding owner, without host
  observation; CLI name-based rename shares its transaction-local policy.
- `rooms.roster` reads membership, metadata and status from one deferred SQLite snapshot and
  excludes presence (host observation, `ls --room --json`). `identity_projection` gives CLI
  and API identical identity bytes. `identityHooks.*` are scoped to the named consumer;
  another consumer's hook is `HOOK_NOT_FOUND`.
- `extensions.uses` (`api/extensions.rs`) resolves the default install prefix and reads only receipts through
  `native_install::uses`; it never installs, runs an extension or opens storage. The declaration is the optional
  release file `TMT-USES.json`, validated at acquisition and inspected like the other release files.
- History reads never acknowledge or renew retention; dispatch operation IDs recover
  immutable acceptance and replay never wakes again.
- `tmt-cli::mcp_command` pins one saved identity UUID and data root and composes the existing
  command owners in process with the same JSON encoders. API dispatch distinguishes local name
  lookup from an exact saved UUID so retirement cannot retarget a pinned writer.

## Extension hooks

- `tmt extension hooks enable <name>` records path, SHA-256, stat fingerprint and capabilities
  in `<global>/extension-hooks.json` (0600, atomic replace); ownership and fingerprint are
  re-checked before every delivery and a change skips the extension until re-enabled.
- Enable probes before locking. Enable/disable decode and change the current document in
  `tmt_extension_state::Document::update`, using the caller's global directory as state root.
  Its stable directory-identity lock waits up to five seconds for admission; callbacks are
  never replayed. Existing destinations must be owned regular 0600 files for mutation;
  a mode refusal names the path, `chmod 600` fix and exact retry command. Invalid schemas
  retain `EXTENSION_HOOKS_INVALID`; lock/storage failures and uncertain publication retain
  `EXTENSION_HOOKS_UNAVAILABLE` with the shared cause and retry. Never infer rollback from
  an uncertain publication. Missing-file disable creates nothing; existing-file no-ops
  check under the lock without replacing bytes. The shared leaf owns staging and recovery.
- Lifecycle observers keep their once-per-process consent snapshot; context contributions
  read fresh consent. Neither reader takes the writer lock, and disabling does not cancel
  an observer already captured by the current process.
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
