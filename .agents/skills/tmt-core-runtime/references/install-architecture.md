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
  inventory, namespace and links for the CLI and the official extensions (Ops with the sole `tmt-ops`
  link, Remote, Colab, Office). Archive data never adds a product. Every product uses one
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

## Former product replacement

`Product::Ops.former()` is a fixed read-only installation identity: `squad`,
`tmt-squad`, links `tmt-squad`/`tmt-sq`, namespace `lib/tmt-squad`, tag prefix
`tmt-squad-v`, and skill `tmt-squad` -> `tmt-ops`. It is never parsed from user
input or offered by completion. `managed` accepts its fully verified current
receipt and payload; an invalid new activation never falls back to it. Activation
locks the new namespace before the old namespace and applies existing channel,
version and pin policy. Interrupted preparation can retry from an empty new layout.

After new activation, `owned::migrate_former_owned` verifies immutable skill
sources and every recorded target before effects. It changes the owner to `ops`
and the lead skill name to `tmt-ops`, retaining custom targets and leaving removed
targets absent. Modified/foreign targets fail closed. Immutable source generations
remain; interrupted link transitions can retry. Ordinary Ops refresh uses recorded
targets only; discovering new providers needs explicit publication consent.

After successful skill settlement, the CLI verifies both installations and
uses `extension_hooks::disable` to withdraw only the former name's recorded
consent. Prior consent removal adds the separate-successor enable command to human
output and `hooks.disabled`/`hooks.enableCommand` to JSON; absent consent adds
neither. No successor consent is created or refreshed. A consent read/write failure
reports the hook error and settings path, leaves the successor active and retains
the former installation for retry. A later former-removal failure keeps consent
withdrawn and reports partial replacement. An unpinned upgrade with no publication
change retries settlement only when a verified former installation remains;
pinned no-ops retain it.

`remove::finish_product_replacement` then verifies the new activation and links,
locks and verifies the old activation, removes only old command links whose target
resolves inside the verified old namespace, and removes that installation namespace.
Foreign same-named commands remain. The CLI reports `replaced`, `removed` and `kept`;
application config, cron, checklist and other data are never removed. A skill failure
retains the old installation for retry. Pinned no-ops retain the former installation.
Full `tmt uninstall` plans and removes a verified former installation through the
same receipt and command-link fence, even without Ops installed; ordinary data
retention and explicit `--purge` policy still apply.

A parent CLI released before Ops registration parses the candidate's upgrade plan with its own product table and rejects `ops` (`EXTENSION_UPGRADE_FAILED`, "invalid extension upgrade report") after the CLI itself is updated; rerunning `tmt upgrade` on the new CLI completes the replacement, and candidates emit no former-name wire compatibility.

## Companions and skills trees

- A release may carry a companion beside its executable (`Product::companions()`; the CLI carries
  `tmt-driver-herdr`); `typescript/scripts/native-artifact-policy.mjs` mirrors the list. An
  archive under release must declare and carry every companion, while older archives still read.
  The manifest declares a companion at most once and the archive must hold it as an executable
  regular file. Publication writes it 0755; the receipt records its digest exactly when present;
  inspection fails closed on a missing digest, missing file, changed bytes or lost execute bit.
- `Product::optional_files()` lists plain non-executable files an extension release may carry
  (`TMT-USES.json`): declared in the manifest at most once, mode without execute bits, published 0644,
  recorded in the receipt exactly when present and re-verified by inspection like a companion. Its content
  is parsed at acquisition (`native_install::uses`), so a malformed file rejects the release. An older
  installer rejects the unknown path, so the supporting CLI must publish first.
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
  come from the fixed product table. Ops, Remote and Colab are installable; with no published
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
- `ls` degrades per entry: a command link with no TMT activation behind it lists as `unmanaged`,
  damage `install --repair` can restore as `repairRequired` (carrying that exact command) and any
  other unreadable activation as `invalid`, each with its path and a repair that never points back
  at `ls`. One bad entry never fails the listing or changes its exit status; install and upgrade
  still refuse on it.
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

- Parameterized `pr<N>` channels, current-head resolver, eligibility epoch, application-schema
  admission and protocol 2 are defined once in [the PR channel contract](../../../../contracts/native-pr-channel.md).
  `pr_catalog`, `pr_json`, `pr_zip`, `pr_resolver` and `pr_receipt` are acquisition/receipt
  modules within the existing native installer; they do not create another publisher. Ordinary
  release provenance and protocol 1 remain readable. The CLI receipt bound is 64 KiB, including
  the two bounded schema source closures and captured admission evidence.

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
