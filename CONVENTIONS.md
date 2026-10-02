# Coding conventions

These are rules for new and touched code, not a claim that all legacy code
already follows them. [ARCHITECTURE.md](ARCHITECTURE.md) owns module boundaries
and known exceptions; [AGENTS.md](AGENTS.md) owns review and maintenance policy.
Use the [development skill](.agents/skills/tmt-dev/SKILL.md) to apply them.

## Style and readability

- Write repository content in English. Use rustfmt for Rust and the nested
  workspace-pinned Vite+ formatter and strict TypeScript configuration for developer
  tooling and extensions. Preserve each owner's explicit formatter configuration and
  file selection; run checks rather than restyling unrelated files.
- Rust uses snake_case modules and owner-local tests. Node tooling uses
  kebab-case files, `<module>.test.ts` tests and `.e2e.test.ts` integration scenarios.
- Use ESM with explicit `.js` extensions for local TypeScript modules and
  `import type` for type-only dependencies. Prefer `node:` for new built-in
  imports without opportunistic whole-repo rewrites.
- Prefer small function-based modules and typed inputs/results. Use discriminated
  unions for alternative states, explicit return contracts on exported APIs,
  and `unknown` plus validation at untrusted boundaries instead of `any`
  or assertions that hide missing evidence.
- Name units and ownership (`timeoutMs`, `identityId`, `socketPath`).
  Extract helpers around shared behavior, not incidental line sequences.
  Do not add wrappers, speculative frameworks or generic utilities without a
  concrete consumer and demonstrated benefit.
- Explain invariants and non-obvious decisions in comments; avoid narrating each
  statement or requiring decorative banners. Favor early validation and cohesive
  control flow over nested special cases.

## Commands, effects and contracts

- Commands are effectful adapters, not pure domain functions. Compose core
  ports with concrete adapters in `tmt-cli`; use its shared `Failure` and output
  lifecycle rather than writing or exiting from domain logic. Pure core code
  must not depend on console, filesystem or process exit.
- Extend the typed parser and dispatcher together. Do not slice argv again in a
  handler, guess an omitted identity, or add a parallel name/pane resolver.
- Specify and test JSON shape, stdout/stderr, exit codes, human output, defaults
  and units for changed commands. Prefer one structured result/error in JSON mode
  without progress text mixed into it. Uniformity is a target with known gaps;
  preserve public behavior unless the issue explicitly changes it.
- Human output and help follow the [CLI style](design/cli-style.md): render through
  `tmt-cli-style` and register commands with its `CommandSpec`, rather than
  printing or building help directly.
- Use the native grammar/invocation and explicit command error mappings as
  authoritative registries. Do not copy stale numeric tables or claim unsupported
  duration suffixes. Convert CLI time values and internal milliseconds explicitly.
- Resolve paths through the existing native config adapter boundary. Do not
  invent another global directory or reconstruct XDG rules in handlers.
- Keep expected domain errors distinguishable from unexpected failures. Preserve
  causes for diagnosis. Catch only to translate, recover, clean up or implement
  documented best-effort effects; never silently discard a failed mutation.
- Bound external process/file work and clean up resources on every exit path.
  Direct file writes are not atomic or concurrency-safe. Follow the state owner's
  transaction/recovery contract, not an "atomic-like" write.
- Message framing and adaptation belong to shared delivery implementation.
  Talk completion comes only from the shared durable request service. Generated
  request-instruction framing is not a terminal-output completion boundary.
  Never add capture/idle/marker fallbacks or provider-specific result cleanup.
  The `!` policy protects coding-agent shell/bash mode, not TTY cosmetics.
  Consult ARCHITECTURE's message delivery and uncertainty contract before
  changing payload/fallback behavior.

## Dependencies and refactoring

Office is a React SPA, not a second CLI runtime. Keep routes, view components
and UI state under `extensions/tmt-office/typescript/apps/office/src`, with behavioral tests beside the owner.
Use TanStack Router for navigation and Jotai for shared cross-view presentation
state; component-local forms and selection may use React state.
Do not parse URLs or invent an application-wide store in view components.
Remote state gets one owner, not mirrored Query/Jotai/Firestore copies. See
[Office architecture](extensions/tmt-office/docs/architecture.md) before adding a service,
contract, drawing dependency or cross-package abstraction.

The separate Office Functions package uses NodeNext TypeScript, Vitest, Oxlint
and the Vite+ formatter. Keep SDK initialization in its entry point, transactions in the store
and credential signing outside transaction retries. Do not import service/Admin
code into SPA production source. Combined emulator fixtures may import the
service's test-only owner and have an explicit E2E type-check target.

Native Rust uses edition 2024, rustfmt, snake_case module files, explicit typed
requests and standard `Result` boundaries. Keep Clap and output in `tmt-cli`,
pure validity rules/use cases in `tmt-core`, and concrete effects in the
`tmt-adapters` package. Follow the declared MSRV and committed lockfile; do not
silence warnings to avoid correcting a touched implementation. Unit tests stay
with their owner; shared process scenarios reuse the repository test harness.

The native architecture integration test separates module discovery, policy and
adversarial examples. Extend its reviewed permissions only with a documented
boundary decision; do not suppress failures with source exclusions or a second
dependency inventory. Derive shared declaration ownership from production code.
Keep generic textual errors local to their command's explicit mapping instead
of adding a crate-wide `From<String>` implementation for shared `Failure`.

Before adding a dependency or abstraction, inspect existing helpers and compare
the concrete benefit, compatibility/native-install impact, maintenance, license,
security and operational cost. Record the decision in the issue. A built-in is
not automatically preferable; a dependency is not a substitute for design.

Refactor within the issue when it makes implementation coherent and testable.
Keep behavioral changes explicit. Do not mix unrelated renames, formatting or
migrations into a correctness fix; split larger prerequisites into linked issues.

## Tests and review

Follow [DEVELOPMENT.md](DEVELOPMENT.md) and the
[E2E skill](.agents/skills/tmt-e2e/SKILL.md) for commands and test boundaries.
Name the invariant or realistic defect each test detects. Use real temporary
storage for repository behavior and injected ports for focused service tests;
mocking a storage operation is not proof of persistence or atomicity.

Use deterministic barriers for races, bounded polling for readiness, and
observable failure/cleanup postconditions. Verify causal agent output rather
than terminal echo. Cover compatibility, negative paths and partial failure,
not only the new happy path. Demonstrate regression tests fail against the
original defect when practical; explain when that evidence cannot be obtained.
For high-risk guards, use a valid positive control and change the relevant
condition in isolation: an unrelated failure must not satisfy the negative
assertion. Where practical, disable or perturb the intended guard to confirm
the test detects its absence; report unverified sensitivity honestly.

Primary review examines test setup and assertions as code. Do not weaken an
assertion, bypass a gate or count more tests as proof of correctness. Use fixture
types compatible with the pinned runtime; an example from a newer framework
version is not automatically a project convention.
