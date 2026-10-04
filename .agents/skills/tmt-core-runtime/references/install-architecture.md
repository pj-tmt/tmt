# Managed skills and native installation

Code map and invariants for `tmt-adapters::skill_installation` and
`tmt-adapters::native_install`. The cross-cutting summary is in
[ARCHITECTURE.md](../../../../ARCHITECTURE.md#managed-skills-and-native-installation); the
handoff wire format is
[native-install-handoff-v1](../../../../contracts/native-install-handoff-v1.md); build,
publication and verification procedures are in the [tmt-release skill](../../tmt-release/SKILL.md).

## Managed skills

- `skill_installation::catalog` pairs each name in `tmt_core::skill_catalog` with its embedded
  bytes and the earlier bundle layouts upgrades still verify; `Catalog::new` joins owner-held
  skills and never lets an owner shadow a core name. Skills install as one versioned bundle
  materialized by digest. Skill installation never opens configuration, SQLite or tmux.
- Ownership needs a known skill name and a link into this home's `skill-assets` store. A missing
  generation is a dangling TMT link (refreshable or removable); a real directory, mismatched
  name, outside link or modified source is preserved as a conflict.
- Extension-owned skills arrive through `skills.install`/`skills.remove` (explicit
  `consent: true`). `skill_installation::owned` validates them, stores each under
  `skill-assets/owners/<owner>/<digest>/<name>` (digest recomputed) and links it into the same
  roots; `skill-owners.json`, separate from the target intents core refresh reads, records each
  name's owner, digest and targets. The first owner of a name keeps it until an explicit force.
  The same-user API cannot authenticate its caller, so install and remove refuse targets another
  owner holds. Claims and unmanaged paths are checked for every target before any effect;
  removal deletes only links into the owner's store.
- The Office skill sources under `extensions/tmt-office/skills/` are still embedded into the core
  bundle (extraction debt owned by #328, not a second source).

## Native installation

- `native_install::Product` is fixed policy with no filesystem or network effect: identity,
  inventory, namespace and links for the CLI and the official extensions (Squad with `tmt-squad`
  and `tmt-sq`, Remote, Colab, Office). Archive data never adds a product. Every product uses one
  acquisition, receipt and atomic-publication path with independent links, lock and current
  release. Manifest selection uses product and target together and rejects ambiguous or multiply
  owned artifacts.
- Discovery lists matching refs per product tag prefix (CLI `v`, others `tmt-<name>-v`), finishes
  bounded ref discovery before channel filtering and semantic-version selection, and treats
  equal-precedence published versions as ambiguous; incomplete discovery fails closed. GitHub's
  latest pointer never selects a channel.
- `release_http::Https` treats a 403/429 as rate limiting only with primary headers reporting
  zero remaining plus a reset, or a secondary Retry-After, and retries once within the caller's
  unchanged absolute deadline. `GITHUB_TOKEN` is sent per hop only to `api.github.com`.
- Publication runs the caller's release verifier on the written candidate before the receipt, so
  a rejection keeps the previous release current; a product whose row requires a verifier is
  refused without one before anything is written.
- Modules: `artifact` (cargo-dist metadata and archive checks), `publication` (stage under an
  invocation-owned prefix; receipt, current and links written atomically under the installer
  lock; old owned releases kept until ownership and integrity permit cleanup), `receipt`,
  `release`, `managed`, `upgrade`, `repair`, `skills_tree`, `active_companion`. A receipt's
  repository must be `pj-tmt/tmt` or the read-only historical `wkh237/tmt` or `wkh237/tmux-team`;
  new receipts use `pj-tmt/tmt`. `Product::accepts_prerelease_flag` mirrors
  `native-release-policy.mjs`.
- Receipts anchor to the installation prefix and current executable, not `ConfigPaths.global_dir`.
  Failed validation or cancellation keeps the previous release and receipt; a post-activation
  skill failure reports partial completion, not an atomic transaction.

## Companions and skills trees

- A release may carry a companion beside its executable (`Product::companions()`; the CLI carries
  `tmt-driver-herdr`); `typescript/scripts/native-artifact-policy.mjs` mirrors the list. An
  archive under release must declare and carry every companion, while older archives still read.
  The manifest declares a companion at most once and the archive must hold it as an executable
  regular file. Publication writes it 0755; the receipt records its digest exactly when present;
  inspection fails closed on a missing digest, missing file, changed bytes or lost execute bit.
- An extension release (never the CLI) may carry `skills/<name>/<path>`
  (`native_install::skills_tree`): the manifest declares the single asset `skills`, the inventory
  comes only from the archive (SHA-256 verified before parsing), only regular files under
  `skills/` are extracted, paths follow `valid_skill_name`/`valid_skill_file`, and bounds are 16
  skills of at most 64 files, 1 MiB each, with a `SKILL.md` per skill. Any violation rejects the
  release before publication; an unrecorded file, link or special entry fails with "Installed
  release inventory has changed". Extension receipts are bounded by a limit derived from the skill
  bounds; the CLI receipt stays at 16 KiB.
- After activation `native_install::release_skills` re-reads the tree under the installation lock
  against the receipt. Install and upgrade refresh the owner's tree skills and remove by name the
  ones a new release dropped; `uninstall` removes every skill the owner holds, with one consent
  prompt naming skills and targets.

## Extensions through `tmt extension`

- The facade `extension_install_command` keeps dispatch, consent, errors, interruption, rendering
  and uninstall; private modules own install, repair, list/upgrade and skills settlement. Names
  come from the fixed product table. Squad, Remote and Colab are installable; with no published
  release in the selected channel, install returns `EXTENSION_RELEASE_UNAVAILABLE` (marked by
  `release::ReleaseUnavailable` after complete discovery) and changes nothing.
- Office is frozen: install and explicit upgrade refuse before consent or acquisition, root
  upgrade skips it, listing marks an existing one frozen and never looks up an upgrade, and
  `tmt office install|upgrade` share the same `require_installable` guard. Historical receipts and
  Office skill names stay available for listing and consented removal.
- Install, upgrade and uninstall need consent (`--yes` or a prompt) and refuse non-interactive
  runs without it. `ls` reads local receipts only; `--check` adds one bounded metadata lookup
  and reports `unknown` on failure.
- Removal validates ownership of every command link, refuses a foreign same-named command and
  deactivates links without deleting releases or data. It is recoverable, not atomic: a missing
  link with a retained activation lists as `partiallyRemoved` with an exact removal command.
- `ls` degrades per entry: a command link with no TMT activation behind it lists as `unmanaged`
  and an activation that cannot be read as `invalid`, each with its path and a repair that never
  points back at `ls` (move or remove the file, or `tmt extension rm`). One bad entry never fails
  the listing or changes its exit status.
- Remote and Colab use the same manifest and receipt under independent `lib/tmt-remote` and
  `lib/tmt-colab` namespaces; install, removal and upgrade never run their `serve` command or
  open their private data roots.
- `install --repair` (`native_install::repair`) admits only safe owned layouts with no-follow
  regular files, re-acquires the exact recorded artifact with matching provenance and digests and
  keeps version, channel and pin; `RepairRequired` carries the single quoted hint to the CLI.
  Acquisition holds no installation lock; the stable lock and a current/receipt revalidation
  fence publication. The damaged release stays untouched at its path, is never an execution
  candidate and is not cleaned up automatically. Local receipts are installation evidence, not
  signatures.

## CLI self-upgrade

- The running binary verifies release metadata, product/tag/target identity, manifest and archive
  digests and bounded archive safety (canonical relative paths, regular files, no links,
  duplicates or special permissions, bounded sizes and entry count) before executing any
  candidate, materialized in a private invocation-owned directory. SHA-256 does not protect
  against a compromised release origin.
- The candidate's `__native-install` owns inventory, companion policy, receipt creation and the
  atomic publisher. Neither acquisition nor the parent holds the installation lock across the
  child, and the parent never reads the candidate's inventory. Pre-activation failures keep the
  previous release; a reported post-activation failure keeps the active result; a missing report
  is uncertain and asks the user to inspect before retrying.
- Root upgrade updates the CLI first and lets that CLI judge extension inventories; core registers
  each product's files before that product publishes them, so the supporting CLI release must reach
  users first. `native_upgrade_command` refreshes managed skills in the new executable, then asks
  it for hidden `__native-upgrade-extensions --json --plan`, asks one consent question and passes
  that exact plan on stdin with `--yes`. An unsupported older target fails with a rerun hint; there
  is no old-process fallback or rollback. `upgrade_all` keeps each channel and pin; non-terminal
  runs without `--yes` report `consentRequired`; product failures stay independent, a CLI failure
  stops the extension phase and a pinned CLI permits it.
- `NATIVE_UPGRADE_FAILED` carries a diagnostic `cause`; HTTPS failures never echo URI or proxy
  credentials. Managed-skill conflicts keep the path array and one shell-quoted backup command per
  preserved entry.
