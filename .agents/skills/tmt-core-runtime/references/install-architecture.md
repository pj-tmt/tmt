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
- Core publication selects bundled catalog names; official release publication verifies its
  installed product receipt before selecting names. These operations replace
  existing leaf entries at those names, without prompt, flag, backup or prior ownership/byte
  checks. Other names and symlink destinations are untouched. Immutable source validation and
  source-overlap guards remain; missing recorded targets stay missing during refresh.
- Extension-owned skills arrive through `skills.install`/`skills.remove` (explicit
  `consent: true`). `skill_installation::owned` validates them, stores each under
  `skill-assets/owners/<owner>/<digest>/<name>` (digest recomputed) and links it into the same
  roots; `skill-owners.json`, separate from the target intents core refresh reads, records each
  name's owner, digest and targets. The first owner of a name keeps it until an explicit force.
  The same-user API cannot authenticate its caller, so install and remove refuse targets another
  owner holds. API publication refuses unmanaged targets; explicit force backs them up outside
  discovery before replacement. API names/owners cannot authorize catalog replacement.
  Name claims are checked before publication;
  removal deletes only links into the owner's store.
- Historical bundle inventories remain verification metadata only; retired guidance is no longer embedded or published by the bundled installer. The generic owner-supplied JSON API is unchanged. Refresh and default uninstall preserve existing retired links, owner records and immutable sources, regardless of the recorded owner. Full inventory and digest admission still fences old Core-name replacement.

## Native installation

`tmt extension install <name> --to <version>` selects and pins the exact release through the
same fixed release-record acquisition as upgrade, without reading a channel pointer.
It retains install consent, rejects local archive/repair combinations, and uses the
existing exact-upgrade policy (including downgrade refusal) when already installed.

[Release-index v1](../../../../contracts/release-index-v1.md) owns the publication-side
record contract and the fixed-origin API-free native resolver.

After successful publication, the native installer retains the active release and
its exact predecessor. It best-effort removes only older
generations whose complete receipt and payload still verify; unknown, damaged,
foreign and legacy content stays intact. Cleanup never fails activation, and
repair does not prune.

The one-release Core-name cutover is owned by `skill_installation::retired` and
`native_upgrade_command::rename`. Install and the upgrade skill-refresh child
replace the former main skill only after checking the entire digest-tracked
source inventory; edited or unmanaged entries remain conflicts even with force.
Former-name conflicts refuse ordinary main skill publication, avoiding a newly
published duplicate in that root.
Publication and new target registration precede exact old-link revalidation and
removal. Recorded custom roots participate; missing integrations stay missing.
After the default directory moves, a dangling absolute source coordinate may be
republished only from a present, fully digest-verified relocated generation,
including owner-held skills. This is installer recovery, never a runtime path alias.

Only the CLI upgrade's successful, unpinned finish attempts the pane-option
cutover. The owner runs the first rename-A upgrade and an immediate second
unpinned upgrade from newly active A: the prior executable owns the first finish,
so only the second executes the new pane converter. The first refresh runs after
activation; its ConfigPaths move uses active-release admission and a silent
report callback to retain the prior updater's JSON/empty-stderr contract. Managed
refresh dispatch bypasses generic driver registration and observers so they cannot
discover configuration before this protocol owner. Install and setup never gain
host effects. It selects the caller's
explicit server socket and existing stored bindings, captures exact old metadata
and native server/pane incarnations, then compares the full binding record under
the mutation transaction before tmux's exact-value command-queue fence. Changed
rows, occupied new keys, unknown evidence and edited pane formats remain intact.
Only an unchanged owned pane border prefix is rewritten; inherited formats and
other host presentation are untouched. The former server key remains recovery
evidence for unverifiable panes; ordinary adapters read only the new keys.
Storage opening has its own 3-second budget; each selected pane gets a fresh
3-second deadline within an overall 3-minute native-effect cap. A failed pane
keeps its metadata and cannot consume a later pane's allocation. A summary reports
converted and unchanged counts only after a conversion; panes without retired markers
stay quiet. A newer schema silently skips this cosmetic pass before host work;
invalid or incomplete histories retain the unavailable hint. Partial failure is a human hint
or optional `paneRenameHints` JSON array, never upgrade failure, retirement, re-launch or automatic rebind.

- `native_install::Product` is fixed policy with no filesystem or network effect: identity,
  inventory, namespace and links for the CLI and the official extensions (Ops with the sole `tmt-ops`
  link, Remote, Colab and Digest). Archive data never adds a product. Every product uses one
  acquisition, receipt and atomic-publication path with independent links, lock and current
  release. Manifest selection uses product and target together and rejects ambiguous or multiply
  owned artifacts.
- Normal discovery reads one fixed raw index pointer per product/channel, then its size/hash-bound
  record. Exact versions read the record from the tag's release assets; historical exact tags without
  a record refuse, without API fallback. Bootstrap pointers may name a raw index record. The
  [release-index contract](../../../../contracts/release-index-v1.md) owns identities and bounds;
  cargo-dist remains the archive, inventory and application-schema authority. Missing or unreadable
  pointers fail without changing installations; a stale pointer cannot authorize a downgrade.
- `upgrade_preview` prepares a release without acquiring its native payload or running a
  candidate. Ordinary channels authenticate the selected manifest; PR channels authenticate the
  catalog and recheck the current head, eligibility, producer and artifact metadata without fetching
  the payload ZIP. Pinned selection stays offline. Applying a prepared selection rechecks the
  captured installation and intent, then matches the acquired release identity before publication;
  a lagging ordinary pointer remains a no-op. Migration preview compares verified candidate schema
  with the existing read-only store observation and reports unknown for incomplete evidence.
- `release_http::environment_token` selects nonempty `GH_TOKEN`, then `GITHUB_TOKEN`, with no
  credential command or prompt. Only authenticated PR-mode API hops receive Authorization;
  normal raw/download hops and redirects never receive tokens or follow an API redirect. The
  existing API-only 403/429 bounded retry and absolute acquisition deadlines remain unchanged.
  Binaries published before this resolver still use the API for their first upgrade.
- Publication runs the caller's release verifier on the written candidate before the receipt, so
  a rejection keeps the previous release current; a product whose row requires a verifier is
  refused without one before anything is written.
- Modules: `artifact` (cargo-dist metadata and archive checks), `publication` (stage under an
  invocation-owned prefix; receipt, current and links written atomically under the installer
  lock; old owned releases kept until ownership and integrity permit cleanup), `receipt`,
  `release`, `managed`, `upgrade`, `repair`, `skills_tree`, `active_companion`. A receipt's
  repository must be `pj-tmt/tmt` or the read-only historical `wkh237/tmt`;
  new receipts use `pj-tmt/tmt`. `Product::accepts_prerelease_flag` mirrors
  `native-release-policy.mjs`.
- Receipts anchor to the installation prefix and current executable, not `ConfigPaths.global_dir`.
  Failed validation or cancellation keeps the previous release and receipt; a post-activation
  skill failure reports partial completion, not an atomic transaction.
- `Product::post_upgrade_check` declares Remote's post-replacement lifecycle step. The CLI
  captures one read-only running observation through the verified existing executable before
  replacement and passes it through success/partial failure. Only an actual replacement of a
  previously observed running Remote calls the captured replacement's `restart --json --no-open`,
  outside the install lock with captured Core context and stripped secrets. Core never signals
  or starts a stopped Remote. Remote owns safety, cutover and typed outcomes in the
  [lifecycle contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery). Success
  and existing deferred/failed hints retain the notice through skills settlement; stopped,
  initial and unchanged installs stay quiet. The parent keeps its JSON child silent and adds
  the one adapter-owned restart bound to its apply deadline only for a pending Remote replacement.

## Retired Squad installation

Ops is an ordinary product with no former identity. TMT no longer upgrades, replaces or
uninstalls a Squad installation (`lib/tmt-squad`, links `tmt-squad` and `tmt-sq`), switches its
boards, withdraws its hook consent, renames its skills or touches its configuration and data
(#3836). `native_install::retired_squad` names a leftover by its directory and the links that
still exist; it reads and verifies nothing, so a damaged leftover is reported too.
`tmt extension install` and `tmt upgrade` print it as `retiredSquadHint` (text, and JSON on
`extension install` and `upgrade`): the directory is left as it is, `tmt extension install ops --yes`
when Ops is absent, that Ops moves the Squad config and data on first use (a running former
board defers that until it exits), `tmt extension hooks disable squad` when hook consent for
`squad` is recorded (the consent is never withdrawn for the user), and the leftover links and
directory to remove by hand. `native_install::release_prefix` is the one place that maps a release's
executable to its installation prefix; the hint and the managed inspection both use it.
Squad-only prefixes are not an installed Ops: `tmt extension ls` and `tmt upgrade` list no Ops row.

## CLI prefix move

The CLI's prefix is `lib/tmt`. `lib/tmux-team` is its former layout through the same
`Product::Cli.former()` mechanism as above: same product and commands, `pr_channel` true so a
PR-channel installation moves with its provenance. Links stay `tmt` and `tmux-team` for fresh and
moved installations until the separate alias decision.

- **Move.** A current caller (bootstrap, offline `__native-install`, handoff protocol 3 or 4) on an
  installation whose only activation is in `lib/tmux-team` publishes under `lib/tmt`, locking new
  then former, and retargets `bin/tmt` and `bin/tmux-team` with a temporary symlink and rename, only
  when a link equals exactly the former target. `lib/tmux-team` is not touched; it stays the rollback
  source. An interruption after activation leaves `lib/tmt/current` with the former links; the next
  installation completes the link move. Once `lib/tmt` has an activation it is authoritative and a
  caller that observed the former current is refused with "changed while downloading". The first
  activation in `lib/tmt` publishes even when its release equals the former one (a bootstrap of the
  installed version), so the links never point into an empty prefix.
- **Refusals** happen before any write and leave the installation usable: a `lib/tmt` that holds
  entries the publisher did not write, a foreign command at a link name, an activation in either
  layout that fails receipt verification.
- **Rollback** has no command. The previous release stays current in `lib/tmux-team`, verified
  through the former-layout read path and runnable by its own path; the next installer run moves forward again.
- **Retirement.** The first activation by a current caller that finds `lib/tmt` already active (not the
  move itself) removes `lib/tmux-team` when the new release is receipt-verified, no `bin/` symlink and
  no provider hook settings file names the former prefix, and the former directory holds only what the
  publisher writes. Otherwise it stays and `tmt upgrade` that ran from `lib/tmt` prints one hint
  (`formerPrefixHint` in JSON). A refusal never fails the upgrade; there is no timer.
- **Persisted paths.** Command links and `tmt setup` hook settings (launcher resolved from `PATH`, so
  `bin/tmt`) are the places that can name the prefix; the audit reads both. Receipts record only the
  installation prefix, skills are published as copies and extension records name extension paths.
  Core's `core_executable` selects the stable `<prefix>/bin/tmt` launcher for companion
  callbacks only after inspection identifies the invoking executable as the active managed CLI.
  It preserves that link through activation and pruning, without canonicalizing it into a release
  path. Unmanaged, stale or uncertain installations keep the exact invoking executable; embedded
  processes pass an inherited absolute `TMT_EXECUTABLE` through unchanged.
- **Handoff protocols** (frozen; wire details in the [handoff contract](../../../../contracts/native-install-handoff-v1.md)):

| Protocol | Request                                 | Updater validates | Candidate publishes                            |
| -------- | --------------------------------------- | ----------------- | ---------------------------------------------- |
| 1        | release transfer (transfer protocol 1)  | `lib/tmux-team`   | `lib/tmux-team` while the CLI is active there  |
| 2        | PR wrapper around transfer protocol 1   | `lib/tmux-team`   | same as 1                                      |
| 3        | release transfer (transfer protocol 3)  | `lib/tmt`         | `lib/tmt`, moving the installation when needed |
| 4        | PR wrapper 4 around transfer protocol 3 | `lib/tmt`         | `lib/tmt`, moving the installation when needed |

Updaters of this generation speak 3 and 4 only; the candidate serves all four. A candidate older than
protocol 3 fails the probe with the existing "needs a newer installer" message, so a downgrade across
the move is a bootstrap reinstall.

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
  Its optional `requiresCli` minimum is frozen in receipt `requires_cli`; optional
  `messageProtocols` is read from the verified release file only. Activation refuses an
  incompatible candidate before layout creation; listing compares the verified receipt with the
  running CLI without acquisition. The [extension contract](../../../../contracts/extension-api.md#optional-cross-extension-uses)
  owns grammar, precedence, errors, JSON and publication ordering.
- An extension release (never the CLI) may carry `skills/<name>/<path>`
  (`native_install::skills_tree`): the manifest declares the single asset `skills`, the inventory
  comes only from the archive (SHA-256 verified before parsing), only regular files under
  `skills/` are extracted, paths follow `valid_skill_name`/`valid_skill_file`, and bounds are 16
  skills of at most 64 files, 1 MiB each, with a `SKILL.md` per skill. Any violation rejects the
  release before publication; an unrecorded file, link or special entry fails with "Installed
  release inventory has changed". Extension receipts are bounded by a limit derived from the skill
  bounds; the CLI bound is owned by the [PR-channel contract](../../../../contracts/native-pr-channel.md#admission-receipt-and-activation).
- After activation `native_install::release_skills` re-reads the tree under the installation lock
  against the receipt. Install and upgrade refresh the owner's tree skills and remove by name the
  ones a new release dropped; `uninstall` removes every skill the owner holds, with one consent
  prompt naming skills and targets.

## Extensions through `tmt extension`

- The facade `extension_install_command` keeps dispatch, consent, errors, interruption, rendering
  and uninstall; private modules own install, repair, list/upgrade and skills settlement. Names
  come from the fixed product table. Ops, Remote, Colab and Digest are registered for installation;
  an unavailable channel pointer is an acquisition failure, never confirmation that no release exists,
  and changes nothing. Digest remains unpublished until its release activation.
- Missing-command offers reuse the explicit install composition after one human consent.
  A three-second read-only metadata preview may fail soft to unknown version/size.
  `native_install::ProgressEvent` observes acquisition, actual HTTP byte counts, verification
  and publication; CLI maps it into the application-neutral `tmt-cli-style::progress`.
  Observer output failure cannot affect activation. Successful publication precedes skill
  settlement; skill warnings retain the active release and remain visible in the final result.
- Install, upgrade and uninstall need consent (`--yes` or a prompt) and refuse non-interactive
  runs without it. `ls` inspects local state only: receipts plus the current activation,
  payload, command links and PATH shadowing. Only `--check` adds one bounded metadata
  lookup, and it reports `unknown` on failure. The listing installs and repairs nothing;
  like any full invocation, it may still record verified caller-session evidence through
  the generic session observer.
- Removal validates ownership of every command link, refuses a foreign same-named command and
  deactivates links without deleting releases or data. It is recoverable, not atomic: a missing
  link with a retained activation lists as `partiallyRemoved` with an exact removal command.
- `ls` degrades per entry: a command link with no TMT activation behind it lists as `unmanaged`,
  damage `install --repair` can restore as `repairRequired` (carrying that exact command) and any
  other unreadable activation as `invalid`, each with its path and a repair that never points back
  at `ls`. One bad entry never fails the listing or changes its exit status; install and upgrade
  still refuse on it.
- Remote and Colab use the same manifest and receipt under independent `lib/tmt-remote` and
  `lib/tmt-colab` namespaces; installation/removal never run `serve` or open private data roots.
  Remote’s separate post-upgrade lifecycle command owns any replacement serve.
- `install --repair` (`native_install::repair`) admits only safe owned layouts with no-follow
  regular files, re-acquires the exact recorded artifact with matching provenance and digests and
  keeps version, channel and pin; `RepairRequired` carries the single quoted hint to the CLI.
  Acquisition holds no installation lock; the stable lock and a current/receipt revalidation
  fence publication. The damaged release stays untouched at its path, is never an execution
  candidate and is not cleaned up automatically. Local receipts are installation evidence, not
  signatures.

## CLI self-upgrade

- Parameterized `pr<N>` channels, current-head resolver, eligibility epoch, application-schema
  admission, protected-main producer trust and protocol 2 are defined once in [the PR channel contract](../../../../contracts/native-pr-channel.md).
  `pr_catalog`, `pr_json`, `pr_zip`, `pr_resolver` and `pr_receipt` are acquisition/receipt
  modules within the existing native installer; they do not create another publisher. Ordinary
  release provenance and protocol 1 remain readable. The [PR-channel contract](../../../../contracts/native-pr-channel.md#admission-receipt-and-activation) owns the derived CLI receipt bound, including
  both schema source closures and captured admission evidence.

- The running binary verifies release metadata, product/tag/target identity, manifest and archive
  digests and bounded archive safety (canonical relative paths, regular files, no links,
  duplicates or special permissions, bounded sizes and entry count) before executing any
  candidate, materialized in a private invocation-owned directory. SHA-256 does not protect
  against a compromised release origin.
- The candidate's `__native-install` owns inventory, companion policy, receipt creation and the
  atomic publisher. Candidate handoff, post-upgrade status and bootstrap CLI children
  omit `GH_TOKEN` and `GITHUB_TOKEN`; the parent retains them. Neither acquisition nor the parent holds the installation lock across the
  child, and the parent never reads the candidate's inventory. Pre-activation failures keep the
  previous release; a reported post-activation failure keeps the active result; a missing report
  is uncertain and asks the user to inspect before retrying.
- Root upgrade prepares a metadata-only, frozen CLI and installed-extension preview before one
  human approval (`[Y/n]`: Enter agrees, end of input declines). Decline or a failed preview
  changes no component. `--yes` uses the same frozen selection; JSON, pipes and agent callers
  never prompt. Machine runs retain CLI update behavior and report extension `consentRequired`
  without `--yes`. The CLI then activates and refreshes managed skills through the new executable,
  which keeps extension inventory authority. Its hidden `__native-upgrade-extensions --json --plan`
  must match the consented products, installed versions, pin states and exact targets before the
  parent sends the legacy plan on stdin with `--yes`. The apply child refuses a changed old
  version or a newly pinned installation before acquisition; its immutable executable and the
  adapter's current-id publication fence retain that observation. Unsupported targets fail with
  a rerun hint, without an old-process fallback, activation retry or rollback. Product failures
  remain independent; CLI failure stops extensions, and a pinned CLI permits them.
- `NATIVE_UPGRADE_FAILED` carries a diagnostic `cause`; HTTPS failures never echo URI or proxy
  credentials. After binary activation, skill failures become `skills.status: "warning"` with
  path/cause and partial publication; they never turn that product into a failed upgrade or stop
  later products. Human and JSON output retain each product's activation and skill summary.
  Human output (`native_upgrade_command/summary.rs`) is a table of the components that changed,
  are pinned, wait for approval or failed (`from → to`, installed versions read before the
  upgrade), one muted `Up to date` line, one summary and one `Next:` list, required lines first and
  optional ones muted and each said once; the JSON document is built apart from it.
