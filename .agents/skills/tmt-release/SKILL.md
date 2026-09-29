---
name: tmt-release
description: Maintain and promote tmux-team release lines, versions, and prerelease readiness without assuming publish authorization.
---

# tmux-team release management

Use this skill for release-line maintenance, v4 compatibility fixes, v5 promotion, version synchronization, or prerelease readiness in this repository.

## Long-lived branch policy

- `main` is the active v5 line after promotion.
- `v4` is the maintenance line rooted at commit `7056679dfa816a1acef8e7c978cf1733a578115b`.
- If remote `v4` does not exist, create it at that exact anchor only when the user has explicitly requested the maintenance line, then verify the remote ref before continuing.
- Before any branch mutation, verify the relevant remote refs and ancestry. Never force-push or repoint a long-lived line.
- A v4 maintenance fix requires a tracked issue, a dedicated branch and worktree, and a reviewable pull request. Keep the fix on the v4 line unless an explicitly scoped backport is requested.
- Use the checks available on the v4 line for maintenance pull requests; do not require contexts that the target branch cannot produce. Record any coverage gap in the issue.
- The native version is owned by `rust/Cargo.toml` and exposed through Cargo's package version; there is no TypeScript fallback. Keep any retained developer package version and public release instructions consistent when changing versions. The native skill ships with the CLI; there is no separately versioned plugin or marketplace.
- Follow `AGENTS.md` for GitHub issue state, branch and pull-request links, verification evidence, and safe worktree cleanup.

## Promotion and prerelease checks

Read the complete [native release verification guide](../../../docs/native-release-verification.md)
before archive, installer, upgrade, bootstrap or publication work. It owns the
procedures referenced below; DEVELOPMENT owns ordinary native checks.

- For Rust archives, follow the guide's native Rust release archive procedure.
  Keep cargo-dist's manifest as the artifact metadata owner; independently verify
  bounded extraction, notices, linkage, skill installation and persisted state.
  Raw PR runtime checks do not establish release archive correctness. Do not enable a
  generated installer or publication workflow merely to obtain local archives.
- Follow DEVELOPMENT's native runtime checks and the guide's archive verification for artifact changes.
  Reuse the shared runtime proof for linkage, exact embedded skills and SQLite
  reopen behavior. Keep the independent archive inventory/checksum/notices and
  installer failure/cleanup evidence; raw binaries are not release artifacts.
- For native binary publication changes, also follow the guide's offline
  installer lifecycle procedure using actual separately versioned archives.
  Keep ownership anchored in the installation prefix, not application-state
  selectors; verify old executable preservation, pin policy, partial command-link
  finalization and unchanged data. The internal preview entrypoint is not a
  public bootstrap or permission to replace a user/package-manager installation.
- Promotion requires passing Code quality, Unit tests, and Docker E2E checks.
- For a public native alpha, follow the guide's explicit multi-platform
  release preparation procedure. The manual artifact workflow never publishes;
  all four final native verifiers must pass on the recorded reviewed commit.
  Keep cargo-dist as the merged manifest owner. Authorized publication uses an
  immutable draft-to-published GitHub release and verifies its attestation and
  public installer before promoting README instructions. Publish with the
  bundle's `release-publication.json` flags: the CLI release is a normal release
  marked latest (the README's `releases/latest/download/install.sh` depends on
  it); Office and Squad releases stay prereleases with `--latest=false`. After
  publishing, run the guide's `--check-latest` check. Do not equate a
  downloadable CI bundle with a published or accepted release.
- Every CLI or extension release also passes the guide's upgrade from the last
  published release (its public installer, then the candidate's installer and
  `tmt upgrade`), not only a fresh install. Old receipts must stay readable.
- Every CLI or extension release also passes the guide's upgrade from the last
  published release (its public installer, then the candidate's installer and
  `tmt upgrade`), not only a fresh install. Old receipts must stay readable.
- For curl bootstrap, follow the guide's native curl bootstrap verification.
  Generate from final verified cargo-dist artifacts and invoke the existing
  native publisher; do not enable a competing stock installer. Test an actual
  matching-host archive without Node/Rust on runtime PATH, and distinguish
  controlled-download evidence from an authorized public release smoke test.
  npm/pnpm replacement is a fresh installation without data-transfer machinery,
  not permission to delete old state or silently uninstall another manager.
- Tags, GitHub Releases, npm publishing, and npm dist-tags are separate operations that require explicit authorization; this skill never assumes permission for them.
- Update user-facing installation or channel documentation whenever a version change would make it inaccurate.
- The v5 root npm package is private developer tooling, not a product distribution.
  Do not restore npm publishing or a download wrapper without a separately scoped
  distribution decision. Historical v4 publishing uses that branch's own rules.
