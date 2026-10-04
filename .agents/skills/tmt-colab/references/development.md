# Colab commands

Run from the repository root; `(cd rust && ...)` runs from `rust/`. Use
`CARGO_BUILD_JOBS=2` on shared machines.

## Package gates

```bash
(cd rust && cargo build --offline --locked -p tmt-colab)
(cd rust && cargo test --offline --locked -p tmt-colab)
(cd rust && cargo clippy --offline --locked -p tmt-colab --all-targets -- -D warnings)
(cd rust && cargo test --offline --locked -p tmt-cli --test architecture)
(cd typescript && corepack pnpm exec vp test run --config vitest.config.ts test/tooling/ci-scope.test.ts)
```

Model crate: `cargo test --offline --locked -p tmt-colab-model` and its clippy, from
`rust/`. Independent oracles (need Python `cryptography`; add `--write` only after
reviewing changed bytes; Rust consumes the frozen vectors without Python):

```bash
python3 extensions/tmt-colab/contracts/vectors/model-reference.py
python3 extensions/tmt-colab/contracts/vectors/authority-reference.py
python3 extensions/tmt-colab/contracts/vectors/baseline-reference.py
python3 extensions/tmt-colab/contracts/vectors/send-preview-reference.py
```

## Focused Rust suites

Each is `(cd rust && cargo test --offline --locked -p tmt-colab <selector>)`:

| Area                                            | Selector                                                                                                               |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| Store state, paired checkpoints                 | `--test state`                                                                                                         |
| Owner state and schema preservation             | `--test owner_state --test registration`                                                                               |
| Owner transitions (real child, FIFO barriers)   | `--test transitions`                                                                                                   |
| Registration, revoke callback, mounted endpoint | `--test registration`                                                                                                  |
| Management DTOs and mounted socket              | `management`, `--test socket management`                                                                               |
| Management CLI (`ls`, `show`, `share ...`)      | `--bin tmt-colab --test cli`                                                                                           |
| Stream sync                                     | `--test sync`, `--test socket`                                                                                         |
| Readers                                         | `--lib readers::tests`, `--lib mounted_`, `--test socket mounted_public_readers`, `--test socket archived_owner_pages` |
| Decoder (real child)                            | `--test decoder -- --nocapture`, `--test decoder baseline_`, `--test checkpoint_vectors`, `--test own_vectors`         |
| Page source CLI                                 | `--test page`                                                                                                          |
| Export                                          | `--test export`, `export::tests`                                                                                       |

- Management subcommands precede operands: `tmt colab share link list <page>`,
  `tmt colab share mode <page> link --yes`.
- Tests that start the decoder use the shared `tests/support` test-only decoder
  configuration (60 s invocation, 30 s FIFO readiness); production keeps the
  two-second deadline and all caps. The decoder-timeout cases keep two seconds and
  must see exactly `Deadline`, confirmed cleanup and a recorded PID gone.
- Decoder load proof: compile first, repeat the affected decoder, transition and
  socket tests under at most two owned CPU burners (180 s per run, terminated and
  reaped on every exit), `CARGO_BUILD_JOBS=2` and a private `CARGO_TARGET_DIR`; keep
  logs outside the repository; no Docker or release builds.
- On macOS the decoder reports `memory limit unavailable`; only Linux enforces the
  child address-space limit.
- The `yrs` 0.28.0 pin brings `smallstr` (RUSTSEC-2026-0215, unmaintained, no fix):
  accepted as maintenance debt with decoder containment and hostile-corpus gates.

## Run it

```bash
(cd rust && cargo build --offline --locked -p tmt-cli -p tmt-colab)
PATH="$PWD/rust/target/debug:$PATH" tmt colab spaces --json
PATH="$PWD/rust/target/debug:$PATH" tmt colab serve --json
```

`serve` listens on the owner-only `<dataRoot>/colab/door.sock`; the data root comes
from `tmt api storage.root` (no path guess or Colab root variable), and direct
invocation needs an absolute `TMT_EXECUTABLE`. A stale socket is replaced; any other
file at the path refuses with `COLAB_STATE_UNSAFE`, and a too-deep root with
`COLAB_SOCKET_PATH_TOO_LONG`. Browsers reach Colab through `tmt remote serve` at
`/r/<prefix>/x/colab/`. Page source and export:

```bash
tmt colab page read <page-uuid> --json
tmt colab page write <page-uuid> --file page.html --expected-revision 'v1:<token-from-read>' --json
tmt colab export <page-uuid> --json          # or --dir /existing/export-parent
```

Use the exact revision from `read`; a stale base returns `COLAB_STALE_BASE` (exit 1)
and is never retried. A failed or uncertain serving IPC returns `COLAB_UNAVAILABLE`
without an offline fallback. Export needs an existing parent, creates a new UUID
directory with `page.html` and `manifest.json` (never replacing output), and reports
`error.partialDirectory` on a failed publication.

## App and browser client

Without `TMT_COLAB_APP_DIR` the executable serves the checkout's
`extensions/tmt-colab/typescript/app/dist` or a build hint. To embed a build:

```sh
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match check
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match build
CARGO_BUILD_JOBS=2 TMT_COLAB_APP_DIR="$PWD/extensions/tmt-colab/typescript/app/dist" cargo build --offline --locked --manifest-path rust/Cargo.toml -p tmt-colab
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app exec playwright install chromium
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test:browser
```

`TMT_COLAB_APP_DIR` must be absolute; an invalid build fails compilation, and
`serve --app-dir <absolute dist>` overrides embedded assets (invalid input fails
`COLAB_APP_UNAVAILABLE` before creating state). Rebuild the binary to adopt new
embedded assets; restart serve to adopt disk builds.

- `test:browser` needs the debug `tmt-colab` built first; `COLAB_SERVE_EXECUTABLE`
  selects another absolute binary. For an embedded relocation run set
  `COLAB_SERVE_EMBEDDED=1` and `COLAB_SERVE_APP_DIR` (a separate expected-byte copy)
  and make the checkout `dist` unavailable.
- The native Chromium acceptance also needs `COLAB_PAGE_FIXTURE_EXECUTABLE`: the
  compiled `browser_fixture-*` test executable from
  `cargo test --offline --locked -p tmt-colab --test browser_fixture --no-run`.
- Dev server chrome carries no production CSP (hot reload needs inline scripts); only
  the real-socket built-app scenario proves parent inline blocking.
- The `@tmt/colab-app` lint and format config lives in its Vite configuration
  (single quotes, trailing commas, 100 columns, import/package-key sorting off).

Browser management checks are `test/management.test.ts` (real signatures,
policy/acknowledgment verification and metadata framing), `test/mounted.test.ts`
(tab/session fencing), `e2e/mounted.spec.ts` (parent chrome and screenshots), and
`acceptance/management.spec.ts` (real native management with paired Chromium).
Run app gates and the real-binary lifecycle acceptance after changing mounted
management/Ask lifetimes. Use a seat-owned `COLAB_APP_TEST_PORT`, not the default
4179 on a shared machine; the acceptance harness picks private free ports.

Browser client (three engines):

```sh
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match check
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match test
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client exec playwright install chromium firefox webkit
(cd rust && cargo build --locked -p tmt-colab-model --example browser_conformance --example browser_authority)
COLAB_REPORT=/tmp/colab-browser-results.json corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client --fail-if-no-match test:browser
```

Add `--with-deps` to the Playwright install on Linux. The harness requires Chromium,
Firefox and WebKit, every strict Ed25519 row and the accepted controls; missing or
skipped engines fail. Build the two Rust examples first so cold compilation does not
consume its bounded native-call deadline. `COLAB_RUST_TOOLCHAIN` (default `+1.97.0`),
`COLAB_CHROMIUM_EXECUTABLE`, `COLAB_FIREFOX_EXECUTABLE` and `COLAB_WEBKIT_EXECUTABLE`
select toolchain and binaries; launch failures never skip an engine.
`test:browser --engines chromium` (no extra `--`) is a scoped diagnostic, not a
three-engine pass. The advisory `Colab browser verification` workflow runs Chromium
on scoped PRs and all engines weekly or manual; `COLAB_HARNESS_ROOTS` and
`COLAB_HARNESS_INPUTS` in `ci-scope.mjs` own its selection
([CI selection](../../../../ARCHITECTURE.md#ci-selection-and-worker-model)).

## Packaging and archives

Colab is a released `native-release.yml` product (`tag tmt-colab-v<version>`,
prerelease, `latest=false`): `release: true` with `initialVersion` `0.1.0-alpha.1`
and `requiresCliSha` in `.github/components.json`, and `dist = true` in its Cargo
package. Core registers Colab with the shared installer
(`EXTENSION_RELEASE_UNAVAILABLE` until an archive exists; see the registration
commands in the [release reference](../../tmt-release/references/native-release.md#remote-and-colab-installer-registration)).

`scripts/build-native-artifact.sh <target> colab` installs frozen dependencies with
`corepack pnpm@10.33.0`, builds `@tmt/colab-app` (requires index, assets and a
nonempty `THIRD-PARTY-NOTICES.txt`), exports the absolute dist path as
`TMT_COLAB_APP_DIR` and appends the Vite notices after cargo-about. The release
Cargo wrapper gets `TMT_NATIVE_PRODUCT=colab` and rejects a build without an
absolute existing `TMT_COLAB_APP_DIR`. Fixture checks (no Docker, no release build):

```sh
(cd rust && cargo build --locked -p tmt-test-support --example colab-runtime-fixture)
(cd typescript && corepack pnpm@10.33.0 exec vp test run --config vitest.config.ts test/tooling/colab-runtime-proof.test.ts test/tooling/native-runtime-proof.test.ts test/tooling/cli-process.test.ts test/tooling/native-artifact-stdout.test.ts test/tooling/native-cargo.test.ts test/tooling/native-release-policy.test.ts test/tooling/plan-release-builds.test.ts test/tooling/verify-public-install.test.ts test/tooling/release-workflow.test.ts)
(cd typescript && corepack pnpm@10.33.0 check:tooling)
sh -n scripts/build-native-artifact.sh && sh -n scripts/native-cargo.sh
actionlint .github/workflows/native-release.yml .github/workflows/native-release-bundle.yml .github/workflows/native-release-smoke.yml .github/workflows/native-release-upgrade.yml
```

The fixture binary comes from `rust/target/debug/examples/colab-runtime-fixture` or an
absolute `TMT_TEST_COLAB_FIXTURE`; tests select defects by `--fixture-variant` and
never compile during execution. Only the Colab verifier loads its app proof;
`native-runtime-proof.test.ts` checks that an eager Colab import fails in the raw
CLI verifier's minimal image. For an actual archive, reserve the heavy slot, build
the expected Vite app from the same frozen source and move its dist outside the
checkout, then:

```sh
node typescript/scripts/verify-native-artifact.mjs --product colab \
  --manifest /absolute/colab-manifest.json \
  --archive /absolute/tmt-colab-aarch64-apple-darwin.tar.gz \
  --target aarch64-apple-darwin --app-dir /absolute/expected-colab-app \
  --notices /absolute/combined-notices.txt --license LICENSE
```

The extracted binary is copied to a fresh directory and must serve without
`--app-dir`, checkout output or pnpm on `PATH`; exact HTML, every asset and both
Rust and frontend notices must match.
