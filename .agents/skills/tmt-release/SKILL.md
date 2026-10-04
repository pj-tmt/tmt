---
name: tmt-release
description: Maintain and promote tmux-team release lines, versions, and prerelease readiness without assuming publish authorization.
---

# tmux-team release management

Use this skill for release-line maintenance, version synchronization, CI selection,
native archives, installer/upgrade proofs, prerelease readiness or release tracking.
Follow [AGENTS.md](../../../AGENTS.md) for development and review;
[DEVELOPMENT.md](../../../DEVELOPMENT.md) owns shared checks and the
[architecture](../../../ARCHITECTURE.md#main-release-cuts) owns the release model.

## Main cut authorization

Ben alone authorizes stable releases, major/minor or version-line changes, breaking
releases, manual publication and releases from another branch line. An explicit
cut version requests a draft and verification; it overrides no publication gate.

A new product's first alpha needs its product owner's authorization and reviewed
component-map activation accepted by tmt-lead and its squad lead. Existing-product
authorization activates no new product. Remote and Colab are activated (#1523);
Herdr still awaits activation (#1418).

Ben also retains authorization for README installer promotion, release App
credential creation/rotation, changes to the main-only `release` Environment,
release immutability settings and changes to this standing authorization.
Verification, a downloadable CI bundle or a tracking dispatch grants none of it.

## Automated alpha publication

The pipeline has standing authorization to publish verified alpha cuts from `main`
for released native products on their existing core version. Every publication
gate must pass; `release:false` disables cuts and publication. Forward-only SQLite
migrations are reported without holding an alpha; breaking markers always hold it
for Ben's explicit authorization.

This authorization belongs to the pipeline. Agents need Ben's explicit approval
for the exact release before tagging, creating/editing/publishing a release or
dispatching a publishing run. `native-release.yml` with `prepare` off can publish;
`hold`, `retry` and `rerun` recovery also require Ben's exact-tag authorization.
Releasing a hold skips only its named gate and never `channel`; rerun re-proves
every gate. Read the marker and recovery procedure before either operation.

`native-release.yml` with `prepare` on, `release.yml` with `dry_run` on and the
upgrade proof do not publish. Never commit injected versions to main, create tags
early, replace public assets or replay publication to recover a smoke failure.
Published releases are immutable; a repair requires a new reviewed version.

## Incident rule

Every failed or held release answers one question in its tracking issue: which pre-merge check
should have caught this? Close that gap in `.github/release-parity.json` (an incident row naming
the release step and either a pre-merge counterpart or a concrete release-only reason, with the
guard's incident list and tests extended) in the same change as the fix. The fix alone is not
enough: a release must never be the first run of a check. See
[Release gate parity](references/ci-selection.md#release-gate-parity).

## Procedure routing

Read the relevant reference before acting; archive, installer, upgrade, bootstrap
and publication work requires the complete native verification reference.

| Task                                                                                                                                                                          | Reference                                                       |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Main/v4 branch policy; cut cadence/allocation; version injection; Remote/Colab packaging; exact-tag pipelines; publication gates, recovery, latest selection and public smoke | [main-cuts.md](references/main-cuts.md)                         |
| Four-host archive/notices/installer/bootstrap acceptance; CLI support-floor and product upgrade proofs; packed cleanup; PR titles; Project tracking; queue metrics            | [native-release.md](references/native-release.md)               |
| Component scope, cumulative queue diffs, required worker gates, shards, caches and advisory browser selection                                                                 | [ci-selection.md](references/ci-selection.md)                   |
| Non-publishing synthetic versions; installation-fixture preparation; development-version comparisons; native recording driver                                                 | [installation-fixtures.md](references/installation-fixtures.md) |

## Project release reconciliation

Use the [tracking procedure](references/native-release.md#project-release-tracking)
for delivery/publication evidence and its focused checks. Shared issue lifecycle
and Project status definitions stay in [DEVELOPMENT.md](../../../DEVELOPMENT.md#project-tracking).
