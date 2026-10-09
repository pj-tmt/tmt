---
name: tmt-dev
description: Implement, review, or maintain TMT repository changes with the project's issue, architecture, verification, and review gates.
---

# TMT development

Use this skill for repository implementation and review work. It is a development
workflow, not end-user documentation for the `tmt` CLI.

Read the repository guidance before planning work:

- [`AGENTS.md`](../../../AGENTS.md) — pattern inspection, discretionary delegation, ownership, and lifecycle policy.
- [`ARCHITECTURE.md`](../../../ARCHITECTURE.md) — current and target boundaries, legacy debt, and architecture change triggers.
- [`CONVENTIONS.md`](../../../CONVENTIONS.md) — code and test style.
- [`DEVELOPMENT.md`](../../../DEVELOPMENT.md) — commands and the focused verification matrix.
- [`tmt-layout`](../tmt-layout/SKILL.md) when adding or moving repository files.
- [`tmt-e2e`](../tmt-e2e/SKILL.md) for Docker/tmux integration work.
- [`tmt-release`](../tmt-release/SKILL.md) for release-line or packaged-install work.

## Required workflow

1. Follow AGENTS for issue/branch lifecycle, incidental-edit exceptions, pattern
   inspection, delegation and context budget. Confirm the bounded outcome and
   acceptance criteria before implementation.
2. Define the affected inputs/outputs, state transitions, responsibility owners
   and risks. Reuse existing ports/helpers; avoid parallel sources of truth.
3. Implement and apply AGENTS' primary-review gate. For a feature spanning PRs,
   review the integrated flow across those PRs, not only the latest diff:
   ownership, transitions, retries, partial failure and cleanup must agree.
   Consolidate confirmed duplicate responsibility; do not create a generic
   framework merely to centralize code.
4. Before a Docker suite, run `scripts/dev-disk-check.sh`, and use the
   per-worktree image tag and cleanup in DEVELOPMENT's
   [disk section](../../../DEVELOPMENT.md#keep-local-development-from-filling-the-disk).
   Verify the changed layers using DEVELOPMENT and CONVENTIONS. Shared markup
   changes follow the [tmt-tui](../tmt-tui/SKILL.md) admission, geometry/paint and parity
   gates. Record the reviewed revision, findings, dispositions and exact verification evidence.
   When replacing implementations, map behavioral assertions, not test counts:
   returned-error rollback is not crash recovery. Preserve resource cleanup
   ordering through the existing child-process owner. Native CLI scenarios use
   the shared sandbox runner's native ancestry and input connection isolation,
   documented in DEVELOPMENT;
   keep direct runtime-caller positive controls fenced rather than adding
   scenario skips or production guard overrides.
   Rust executable-fixture changes follow DEVELOPMENT's
   [three ETXTBSY cases](../../../DEVELOPMENT.md#rust-checks) and the narrowly owned
   publication boundary in ARCHITECTURE. Verify its exact dev edges with the
   native architecture test before each Rust push.
5. Close the bounded review when relevant evidence supports the agreed behavior,
   correctness/security blockers and confirmed duplicate responsibilities in
   scope are resolved, and deferred risks are explicit. A broad audit is not a
   demand to find nothing else to improve. Follow AGENTS for authorized merge,
   tracker updates, [Project status](../../../DEVELOPMENT.md#project-tracking)
   and safe cleanup: when the PR merges, remove its worktree
   with `scripts/dev-worktree-remove.sh` and then that worktree's Docker images
   in the same turn (the DEVELOPMENT disk section documents its REST check and
   exact steps; if the script refuses, stop and ask), and start the next task in
   a clean existing worktree rather than a new one.

## Architecture maintenance

Rust is the sole CLI runtime; the optional Office SPA is a separate browser
application. Read [Office architecture](../../../extensions/tmt-office/docs/architecture.md)
when touching the app, workspace or planned cloud/connector boundaries. Node
modules under scripts and test directories are developer tooling. Keep Office,
native process, Docker and tooling checks distinct,
and never substitute obsolete TypeScript coverage percentages for native
verification. A raw-binary platform smoke does not replace the release archive,
bootstrap or upgrade gates. Independently released drivers use the
[tmt-release](../tmt-release/SKILL.md) product-specific archive and approval proof using the current published CLI. Keep the shared
runtime proof in one owner. Process/archive fixture producers must build both
`tmt-cli` and `tmt-driver-herdr`; transferring only the CLI leaves the independently
owned companion unavailable. See DEVELOPMENT's native fixture build contract.

Formatter migrations preserve DEVELOPMENT's per-owner options and separate
code/docs selections, with mechanical output isolated from configuration changes.
Keep exact regeneration and semantic equivalence evidence in the PR.
Lint migrations preserve per-owner file arguments, plugins, warning severities and
exit behavior. Compare effective rules and diagnostic multisets, and record newly
enabled defaults separately; DEVELOPMENT owns the workspace lint commands and shared
configuration. Keep compiler checks separate until an explicit equivalence decision.

For installed-agent guidance changes, follow DEVELOPMENT's installed guidance
source ownership: edit the single canonical skill, verify native provider links
and local drift detection, and review semantic changes and provider invocation
behavior. Matching installed bytes alone does not prove that instructions are
correct. Do not reintroduce command wrappers or provider-specific copies.

For every change, record a substantive architecture-impact assessment. If the
change modifies a module boundary, dependency direction, public contract,
legacy compatibility path, persistence model, or test/fixture ownership, update
`ARCHITECTURE.md` in the same PR and explain the change. If none of those
triggers apply, record why in the PR/GitHub issue; a checkbox alone is not
evidence. Keep architecture rules in `ARCHITECTURE.md`, style rules in
`CONVENTIONS.md`, and command guidance in `DEVELOPMENT.md` rather than copying
them into this skill.

## References

- [references/focused-checks.md](references/focused-checks.md): model-free focused commands for Codex driver, reply-notice, request-ID, target, host and completion checks.
