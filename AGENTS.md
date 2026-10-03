# Repository Working Agreement

## Required development context

Before planning or changing this repository, read this file, the
[development skill](.agents/skills/tmt-dev/SKILL.md),
[architecture](ARCHITECTURE.md), [coding conventions](CONVENTIONS.md), and
[development/verification guide](DEVELOPMENT.md). Inspect the relevant source
and issue as well. Use the linked E2E or release skill when applicable. If a
required reference is missing or contradicts the code, report and resolve that
discrepancy within scope before relying on it; do not invent a convention.

## Context and evidence budget

- Reuse completely read, unchanged guidance within a task unless a higher-priority
  instruction requires rereading. Record paths, revision and completed reads at
  handoff; verify provenance and changes before reuse. Reread when uncertain.
- Locate relevant source and reference sections first. Read required instructions
  completely, using bounded chunks to avoid truncation and repeated loading.
- Keep bulk inventories and logs in artifacts. Request concise findings with
  actionable risks and evidence locations; inspect original excerpts as needed.
  Verify published text with equality or a focused diff against its source.
- Keep handoffs to decisions, unresolved risks and evidence. While awaiting a
  gate, research only the next dependencies and decisions, not a full new feature.

These limits reduce repeated input, not required primary review or verification.

## Architecture ownership and primary review

The primary agent owns the architectural model, design tradeoffs, decomposition,
integration, and final review. Delegation transfers implementation work, not
accountability. Before accepting delegated work, the primary must personally
read every changed file's diff and surrounding implementation, relevant callers,
contracts, and tests. Assess dependency direction, module responsibility, reuse,
compatibility, state/failure behavior, readability, and test validity—not only
whether the ticket or CI is green. Apply the same gate to primary-authored work;
an independent reviewer is supplementary, not a replacement.

When the owner splits the work into squads, the squad structure is recorded in
the pinned team issue (#606). Each squad lead is the primary reviewer for pull
requests confined to the paths its squad owns, with the same obligations as
above. A change to shared paths needs review from the lead of every squad it
affects: contracts, `.github/`, `AGENTS.md`, guide index sections, workspace
configuration and lockfiles, and any change to another squad's paths. A squad
does not edit another squad's paths directly; it files an issue for that squad,
unless the owner gave it a cross-cutting mandate recorded in #606, such as the
refactor squad fixing what it finds. Such a change still needs the owning
squad lead's review.

Within its owned paths and contracts, a squad lead decides on its own, including
architecture and design choices, staffing within the recorded limits, and
releases of its components. The architecture owner (the core lead) keeps
decisions that change core paths or core contracts, seams between squads, and
the product's guiding principles; bring only those decisions to it. Status and
progress reports go to the project manager rather than the architecture owner.

Record the reviewed commit, affected boundaries, findings and their disposition,
and verification evidence in the PR and GitHub issue. If there are no findings,
state what was inspected rather than merely saying "LGTM". Review later changes
and rerun affected checks before accepting a newer head.

For every PR, follow the [architecture maintenance contract](ARCHITECTURE.md#maintenance-contract):
update affected architecture and developer guidance in the same PR, or explain
why the change does not affect them. The implementer proposes documentation
updates; the primary reviewer verifies them against code before merge. A linked
follow-up is not permission to ship inaccurate descriptions of current behavior.

## Pattern audit before changes

Before editing code or guidance, inspect relevant architecture, helpers, fixtures
and conventions for reusable patterns, duplicate responsibility and conflicts.
Keep inspection proportional; record material findings and resolve them within scope.

Delegate only when the saved work or independent coverage exceeds coordination
and review cost. Prefer `gpt-5.6-luna` for bounded scans and simple, verifiable work;
keep tightly coupled design and integration with the primary. Assign explicit
ownership and verification, prevent overlapping edits, and keep read-only audits
free of mutations. Delegation never expands authorization.

## Repository content language

All repository content must be written in English, including code, tests,
comments, documentation, configuration, workflows, repository skills, commit
messages, and pull request metadata. Non-language symbols and technically
required fixture data are allowed when necessary; explain any such exception
in English.

One exception: the prose of a translated handbook page, and the translated UI
strings in its `strings.json`, may be written in the language of the directory
`site/src/i18n/<lang>/` that holds them (`ja` or `zh`).
A language is allowed once the `languageExceptions` key of
[`.github/repository-layout.json`](.github/repository-layout.json) lists its
directory, which lands with that language's first translation. English remains the
source and the language of everything else, including those files' front matter,
keys, code, comments, tests, commits and pull request metadata.

## Durable documentation

Keep living documents focused on current results, definitions, contracts and
clearly labeled proposals. User and developer guides may include actionable
instructions and the constraints needed to use them safely. Keep implementation
chronology, rejected alternatives, per-run logs and review evidence in issues/PRs
or pinned history, not repeated in manuals. Preserve non-obvious invariants and
fixture provenance. Each definition has one document owner; other guides link
to it rather than copying it. Removing narrative must not remove a safety gate
or present planned behavior as shipped.

When revising guidance, replace or consolidate overlapping rules before adding
new ones. Resolve contradictions against the owning contract and verified behavior;
ask when resolution would require an undecided product or authorization choice.

## Delivery lifecycle

- GitHub Issues is the active tracker for TMT. Historical Linear links are
  references, not a second workflow or a requirement to duplicate tickets.
- For development beyond incidental edits, use one tracked GitHub issue, one
  dedicated branch/worktree, and one reviewable PR. Confirm outcome, scope,
  acceptance criteria, dependencies and project relationship before editing;
  mark the issue started when implementation begins. Split oversized work first.
- Keep decisions, progress, blockers, deferred work, branch/PR links and evidence
  synchronized in GitHub. Do not mark work done before its delivery state supports it.
- Keep the issue's Project fields, tracker parentage and Status current as defined in
  [Project tracking](DEVELOPMENT.md#project-tracking).
- Every Codex-created commit includes `Co-authored-by: Codex <codex@openai.com>`.
  Preserve the user's authorship and signing configuration.
- Merge only when authorized and all required CI has passed on the reviewed head.
  Never bypass protection or lower checks to deliver. Publishing, releases and
  destructive operations require their own applicable authorization.
- Before removing a completed worktree, verify it is clean, committed and safely
  pushed or handed off, with branch/PR recorded in GitHub. Do not discard user
  changes or unpushed work. Remove the safe worktree and prune stale metadata.
- Follow the [release skill](.agents/skills/tmt-release/SKILL.md) for branch-line
  policy; do not duplicate or improvise long-lived branch rules here.

## Code organization

Follow the [layout procedure](.agents/skills/tmt-layout/SKILL.md) when adding or moving repository files.

- Prefer fixes that simplify ownership and data flow over accumulating defensive patches. Before adding flags, counters, branches or abstractions, check whether moving responsibility to its natural owner or removing redundant state eliminates the defect. Judge simplicity across the affected flow, not by the smallest diff. Keep necessary trust-boundary validation and behavior tests; this is not permission for unrelated rewrites.
- Before launch, breaking refactors are allowed within the agreed scope. Do not retain obsolete APIs, commands or compatibility layers solely to preserve an unreleased design. Update callers, tests and guidance together; this does not authorize discarding user data or uncommitted work.
- Keep production behavior, test infrastructure, fixtures, and scenario assertions in clearly separated modules.
- Prefer small, purpose-specific interfaces and existing dependency-injection boundaries over new global state or parallel abstractions.
- Put shared behavior in one named helper only after more than one caller needs it; keep scenario-specific behavior close to the scenario.
- Use names that describe observable behavior and stable domain concepts rather than implementation accidents.
- Keep completion criteria bounded to the agreed outcome. Classify adjacent findings
  as blocking correctness/security defects or deferred improvements; explain the
  dependency before expanding scope. Do not silently turn optional hardening into
  a milestone gate or declare unresolved blockers complete.

## Verification quality

- Verify observable behavior and durable state, not only exit codes, log echoes, snapshots, or test counts.
- Include representative success, failure, cleanup, and lifecycle cases for the changed behavior.
- Prefer deterministic readiness signals and bounded polling over fixed sleeps.
- Treat false positives, leaked processes, leaked tmux servers, and non-isolated state as test failures.
- Run the repository checks relevant to every changed layer and report the exact commands and results.
