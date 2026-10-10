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

- Read required instructions completely once per task, in bounded chunks. Reread
  them only when they may have changed or you are unsure.
- Locate the relevant source and guide sections before reading broadly. Keep bulk
  inventories and logs in files or artifacts, and verify published text with an
  equality check or a focused diff against its source.
- Hand off decisions, unresolved risks and evidence locations, not transcripts.
  While waiting on a gate, prepare only the next step.

These limits reduce repeated input; they never replace required review or verification.

## Architecture ownership and primary review

The owner splits the work into squads; the pinned team issue (#606) records each
squad's lead, owned paths and contracts, and how squads communicate and merge.
The core lead owns the architecture: core paths and contracts, the seams between
squads, and the product's guiding principles. Within its owned paths and
contracts, a squad lead decides on its own, including architecture and design
choices, staffing within the recorded limits, and alpha releases of its
components. Stable releases, breaking changes and any other publishing
authorization stay with the owner.
Bring only decisions that change core paths, core contracts or a seam between
squads to the core lead.

A squad lead is the primary reviewer for pull requests confined to its squad's
paths. Members implement; the reviewing lead keeps the design and stays
accountable. Before accepting any work, including its own, the primary reviewer
personally reads every changed file's diff and surrounding implementation,
relevant callers, contracts, and tests. Assess dependency direction, module
responsibility, reuse, compatibility, state/failure behavior, readability, and
test validity—not only whether the ticket or CI is green. An independent
reviewer is supplementary, not a replacement.

A change to shared paths needs review from the lead of every squad it affects:
contracts, `.github/`, `AGENTS.md`, guide index sections, workspace
configuration and lockfiles, and any change to another squad's paths. A squad
does not edit another squad's paths directly; it files an issue for that squad,
unless the owner gave it a cross-cutting mandate recorded in #606, such as the
refactor squad fixing what it finds. Such a change still needs the owning
squad lead's review.

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

Give each member one bounded topic with an explicit owner, done condition and
verification; prevent overlapping edits and keep read-only audits free of
mutations. Keep decisions that span topics with the reviewing lead. Model choice
is a staffing decision recorded with the team, not a repository rule.
Delegation never expands authorization.

## Repository content language

All repository content must be written in English, including code, tests,
comments, documentation, configuration, workflows, repository skills, commit
messages, and pull request metadata. Non-language symbols and technically
required fixture data are allowed when necessary; explain any such exception
in English.

One exception: the prose of a translated handbook page, and the translated UI
strings in its `strings.json`, may be written in the language of the directory
`site/src/i18n/<lang>/` that holds them (`ja`, `zh-hant` or `zh-hans`).
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
- Every agent-created commit ends with its provider's co-author trailer, such as
  `Co-authored-by: Codex <codex@openai.com>` or
  `Co-Authored-By: Claude <noreply@anthropic.com>`, plus any session trailer its
  harness requires. Preserve the user's authorship and signing configuration.
- Every PR body names its contributors with visible `Agent:` lines; see the [attribution grammar](.agents/skills/tmt-release/references/native-release.md#pr-agent-attribution).
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
