---
name: tmt-e2e
description: Add, change, diagnose, or run this repository's Docker-isolated end-to-end tests for tmt and tmux.
---

# TMT end-to-end testing

Use this skill for the repository's Docker E2E test foundation. Keep E2E tests separate from unit tests and unit-test coverage: Docker provides isolation, while Vitest is the scenario runner and assertion layer.

## Required test boundary

- Exercise the real `tmt`/`tmux-team` CLI and a real tmux server.
- Run deterministic mock agents inside tmux panes; never invoke a real AI agent, model, credential, or network service.
- Never touch the host user's tmux server, host credentials, or unrelated processes. The container must be network-isolated.
- Reuse the existing E2E harness and its cleanup hooks. Every scenario must leave its temporary tmux server, socket, panes, and files cleaned up, including on assertion failure.
- For lifecycle or cleanup changes, run the Docker suite twice to catch leaked state and non-idempotent teardown.
- Docker selects its built Rust CLI by default; mock replies inherit that selection.
  Verify the default path as well as fail-closed invalid selectors. Host test
  selection defaults to the repository-built native executable and must never
  discover an installed host CLI or fall back to a retired runtime.

## Test quality gate

Apply [CONVENTIONS' test review rules](../../../CONVENTIONS.md#tests-and-review),
including concrete defect coverage and meaningful positive/negative controls.

- Cover distinct boundaries and failure modes instead of repeating equivalent happy paths.
- Exercise the subject under test rather than recreating its logic in the harness. Mock agents are allowed because they are deterministic peers; `tmt` and tmux themselves stay real.
- Assert causal output and durable state. Terminal-echoed input is not proof that a mock agent processed a request; wait for agent-produced output and corroborate it with the structured event log or metadata.
- Keep observation independent of the action being scheduled. Request-service reads can trigger housekeeping; use read-only SQL/repository snapshots when proving cleanup ordering, expiry, or state preservation. Compare original stored messages separately from composed/protected transport text.
- Include negative paths for exit-code propagation, invalid operations, timeouts, partial startup, and thrown scenario errors when those risks are relevant.
- Use bounded polling for observable state changes. Do not use fixed sleeps to hide races or make a flaky scenario appear stable.
- Verify state preservation after failed operations and verify cleanup with observable absence, not only by calling a cleanup function.
- Prefer stable JSON fields, exit codes, pane IDs, metadata, and essential output over incidental formatting.
- Identity scenarios must choose lifetime deliberately: plain `name`/`add` is
  temporary; use `-s` only when persistence is part of the scenario. Global
  listing includes offline saved rows but does not expand cross-server routing.
  Correlate public UUID/lifetime with the existing read-only identity SQL oracle.
  If reconciliation refreshes `last_verified_at`, exclude only that field from
  preservation comparisons and validate its timestamp separately; retain every
  other binding field. Grouped pane presentation follows inventory selection,
  which need not match `display-message`'s chosen linked session.
- Keep foundation tests focused on infrastructure invariants. Add feature semantics only through the issue that defines that feature's state matrix.
- Treat a test that can pass without the intended action occurring as a test bug. Fix false positives before expanding coverage.

## Maintained implementation

Read the relevant files before changing behavior:

- [`typescript/test/e2e/`](../../../typescript/test/e2e/): Vitest configuration, scenarios, harness, and mock-agent behavior.
- [`typescript/scripts/run-e2e.mjs`](../../../typescript/scripts/run-e2e.mjs): Docker build/run wrapper and exit-code handling.
- [`DEVELOPMENT.md#selecting-the-cli-under-test`](../../../DEVELOPMENT.md#selecting-the-cli-under-test):
  shared executable/peer descriptors and container-path requirements. Reuse
  `typescript/test/support/cli-executable.mjs`; never add a hard-coded Node launcher or
  silent TS fallback. Prove selection reaches nested replies and real descendants
  with causal results. Historical TypeScript worker/pack tests are retired;
  use [scenario ownership](../../../DEVELOPMENT.md#scenario-ownership) and
  [evidence boundaries](../../../ARCHITECTURE.md#testing-and-evidence-boundaries)
  to choose the maintained native coverage.
- Keep retained native/E2E helpers under `typescript/test/support/`, independent of the old
  TypeScript product. Historical SQLite migration inputs and reference results
  live in `typescript/test/fixtures/storage-history/`; preserve their provenance, integrity
  checks and independent SQL assertions. Never use the native implementation to
  regenerate its own expected migration results. Keep stopped-schema public reply
  coverage on schema 8 rather than substituting an already-current database.
- [Optional performance probes](../../../DEVELOPMENT.md#optional-performance-probes) for optional runtime
  measurements. Use the same isolated fixture and causal assertions; keep timing
  samples out of ordinary CI thresholds and distinguish measured from unavailable
  process resources. A skipped benchmark is not native parity evidence.

Keep orchestration in the wrapper and scenario assertions in Vitest. Do not duplicate harness or mock-agent implementation in this skill.

## Scope guard

This skill covers CLI/tmux integration; Office browser/emulator verification is
defined in DEVELOPMENT. Test existing persistence and identity contracts where
the CLI flow requires them. Test work does not authorize new product behavior
or silently expand the tracked feature's acceptance criteria.

## Verification

Prefer the repository's documented E2E command through [`typescript/scripts/run-e2e.mjs`](../../../typescript/scripts/run-e2e.mjs). Confirm failures propagate as non-zero exit codes, and inspect cleanup behavior when tests fail—not only when they pass.
