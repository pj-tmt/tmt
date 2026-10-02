---
name: tmt-layout
description: Review repository file additions and moves against the owned layout map.
---

# Repository layout review

Use this procedure when adding or moving repository files. Read the single
[Repository layout map](../../../ARCHITECTURE.md#repository-layout) first; it owns
homes, responsibilities, pending moves and the linked top-level allowlist.

1. Compare the proposed paths with the map and existing neighboring owners. Check
   component ownership through the existing component map, and identify affected
   imports, links, build inputs, fixtures and release consumers before editing.
2. If a new home or exception is needed, send the infra lead a bounded proposal:
   responsibility, component owner, why an existing home does not fit, and a
   removal issue for a temporary exception. Obtain review before adding it; update
   the owning map and its linked JSON together rather than copying them here.
3. Keep moves separate from behavior changes. Use `git mv` for retained files and
   separate pure rename commits from link/import/config rewrites so reviewers can
   verify preservation. Respect the pending-move gates in the map and coordinate
   shared paths with their owners; never move dirty or unowned work.
4. Stage new files before running the tracked-file layout guard. Run
   `pnpm exec vp test run --config vitest.config.ts test/tooling/repository-layout.test.ts` from `typescript/`,
   then the affected consumer checks and `pnpm docs:format:check`. Inspect the diff
   for lost contents, stale paths and accidental generated outputs. Remove a
   temporary exception only when its last tracked entry has moved or been deleted.
5. Report the reviewed head, source/destination ownership, rename/content evidence
   and check results in the PR and issue. Follow AGENTS for approval and cleanup.
