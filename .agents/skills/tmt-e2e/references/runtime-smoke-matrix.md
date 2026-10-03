# Runtime smoke matrix

Native smoke required on full-scope PRs: macOS x64 and arm64, Linux glibc x64 and
arm64, Linux musl x64 and arm64.

- Both macOS targets build on `macos-15` arm64 runners. The x64 row cross-compiles
  `x86_64-apple-darwin`, selects an x64 Node and runs the whole verification process
  tree through `scripts/run-native-verification.sh` under `arch -x86_64` (including
  shell installers and upgrade children). `lipo -archs` must show exactly one
  architecture; an arm64 or universal executable cannot satisfy the Intel row, and
  cross-compilation alone is never runtime evidence.
- `Native Intel verification` (`.github/workflows/native-intel.yml`) supplements Rosetta
  every Monday and on manual dispatch on `macos-15-intel`: it builds and proves the locked
  x64 CLI, then installs the latest published CLI alpha through the public installer and
  runs `tmt upgrade --channel alpha --json`. It is advisory, not a branch-protection check;
  the infra lead triages failed scheduled runs, and public acquisition errors (including
  exhausted rate limits) keep the failed conclusion. Dispatch only this non-publishing
  workflow for the proof, never the release pipeline.
- CI builds four raw targets once (two macOS, two static musl) and reuses the musl
  executable for both Linux smokes. Preserve the `Packed install (<environment>)` check
  names and the `Native package matrix` aggregator. Each smoke runs outside the checkout
  with isolated HOME/state and no Node or Rust on the product `PATH`, and checks
  version/help, exact embedded skill bytes, managed skill installation and SQLite reopen.
- Merge groups keep the two Linux builds and four Linux smoke rows and skip the macOS
  build and install jobs (the aggregate requires `skipped` only on that event); PRs and
  native release workflows still run macOS. The queue tests each cumulative group head.
- Raw PR executables prove source-runtime behavior only, not archive inventory, notices,
  checksums or bootstrap.

For Intel workflow or tooling edits run the focused fixtures before the full suite:

```sh
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/intel-verification.test.ts test/tooling/native-runtime-proof.test.ts test/tooling/verify-public-install.test.ts test/tooling/xcrun-warmup.test.ts test/tooling/release-workflow.test.ts test/tooling/ci-scope.test.ts)
(cd typescript && corepack pnpm check:tooling)
sh -n scripts/run-native-verification.sh && shellcheck scripts/run-native-verification.sh
actionlint .github/workflows/ci.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-upgrade.yml .github/workflows/native-release-smoke.yml .github/workflows/public-install-smoke-pr.yml .github/workflows/native-intel.yml
```

Compare job and step durations over REST against preceding Intel runs and treat runner
queue time separately; production timings come from the next authorized pipeline release.
Preserve required job names, the merge-group macOS skip admission, cache ownership and the
`warm-xcrun` gate.
