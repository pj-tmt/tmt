# Colab commands

Run from the repository root; `(cd rust && ...)` runs from `rust/`. Use
`CARGO_BUILD_JOBS=2` on shared machines.

## Shipped agent skill

`extensions/tmt-colab/skills/tmt-colab/SKILL.md` is the canonical user-facing agent
skill; this development skill remains the repository guide. The executable embeds
it, and `tmt colab skill` prints its exact bytes without core discovery or storage.
Its Page look starter is generated from `design/tokens/tokens.json`. After changing
those tokens or the starter generator, run
`node extensions/tmt-colab/scripts/generate-page-style.mjs --write`, then `--check`.
The generator replaces only the marked style block; the surrounding prose stays
handwritten. It is a development command, never a Cargo, serve or install hook.
The app's `page-style.test.ts` checks token equality, forbidden chrome and effects,
non-writing failures, and budgets of 3,000 bytes for the starter and 12,000 bytes /
500 lines for the complete skill.
`tests/cli.rs` verifies a relocated executable with no core or checkout. Its
fixture publishes the exact binary bytes and mode through the existing dev-only
`tmt-test-support::write_executable` boundary before execution; the test process
never opens that executable for writing and does not retry its invocation.

The component map declares `skills: true`; cargo-dist includes `../../skills`.
The existing consented `tmt extension install colab --skills` path owns provider
links, update receipts and unmanaged conflicts. Archive verification must supply
the expected canonical tree via `--skills extensions/tmt-colab/skills`; a raw
executable smoke does not establish that the tree ships. Shared release prepare
verification derives this argument from `shipsSkills(product)`.

## Hosted bundle build

`tmt colab hosting-bundle --json` reads only the executable's embedded hosted inventory,
without core discovery, storage, or network. Ordinary builds embed none and return
`COLAB_UNAVAILABLE`; `TMT_COLAB_APP_DIR` and native `--app-dir` do not supply hosted bytes.

The build-only `TMT_COLAB_HOSTING_DIR` selects an absolute, separate hosted distribution.
Its files are published at the site root; `index.html` and `THIRD-PARTY-NOTICES.txt` are
required. The shell must use absolute `/assets/` URLs with no `<base>` so rewritten
short routes resolve the same assets. Build a hosted distribution with Vite's `--base /`
option; native mounted distributions retain their relative base. The generator copies Remote's existing built output
`extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js` to `/sdk/remote-v1.js`,
checking its copied SHA-256 against that output. The input tree cannot override this path.
Files and directories must be real, with supported bare content types. The build rejects
inventory reaching 128 files, 2 MiB per file, or 8 MiB total (50% of Remote's v1 caps).
Every manifest file carries the existing native CSP; a renderer entry retains its existing
renderer CSP. Runtime bytes and the canonical manifest are immutable Cargo snapshots.

This pipeline does not supply hosted sign-in/join glue or the deployment declaration's
hosting field. Those wait for #2397 and Remote's Firebase SDK surface; only their exact
Firebase connect origins may be added when that entry is implemented. A synthetic build
fixture proves the wire inventory and command without claiming a working hosted entry.

Focused checks: `cargo +1.97.0 test --offline --locked -p tmt-colab --test build_hosting --test hosting`
from `rust/`. Run `hosting` again with a separate explicit hosted fixture to exercise the
embedded-success path; the ordinary build exercises `COLAB_UNAVAILABLE`.

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

| Area                                            | Selector                                                                                                                                                             |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Store state, paired checkpoints                 | `--test state`                                                                                                                                                       |
| Durable time and expiry                         | `--test expiry`                                                                                                                                                      |
| Owner state and schema preservation             | `--test owner_state --test registration`                                                                                                                             |
| Owner transitions (real child, FIFO barriers)   | `--test transitions`                                                                                                                                                 |
| Registration, revoke callback, mounted endpoint | `--test registration`                                                                                                                                                |
| Management DTOs and mounted socket              | `management`, `--test socket management`                                                                                                                             |
| Management CLI (`ls`, `show`, `share ...`)      | `--bin tmt-colab --test cli`                                                                                                                                         |
| Stream sync                                     | `--test sync`, `--test socket`                                                                                                                                       |
| Readers                                         | `--lib readers::tests`, `--lib mounted_`, `--test socket mounted_public_readers`, `--test socket archived_owner_pages`                                               |
| Decoder (real child)                            | `--test decoder -- --nocapture`, `--test decoder baseline_`, `--test checkpoint_vectors`, `--test own_vectors --test discussion`                                     |
| Page source CLI                                 | `--test page`                                                                                                                                                        |
| Export and attachment read                      | `--test export`, `export::tests`, `export::attachments::tests`, `--test socket root_local_attachment_read`                                                           |
| Object channel and admitted attachments         | `--lib object_channel::tests`, `--test attachments`                                                                                                                  |
| Native attach                                   | `--lib attachments::slots`, `--lib object_channel::attach`, `--lib object_channel::tests::attach`, `--test cli attachment_attach`, `--test socket root_local_attach` |
| Firestore declaration command and vector        | `--test deploy_declaration`, `--lib deploy_declaration`, then Remote's `-p tmt-remote --test deploy_vectors` mirror                                                  |

- Object-channel fixtures exercise the real private socket and neutral Bus with scripted
  extension outcomes; they do not prove the production Remote backend route. Production
  activation and the three-binary routed proof remain separate acceptance gates. Browser
  attachment checks are `test/attachment-channel.test.ts`, `test/attachment-read.test.ts`
  and `e2e/fold.spec.ts`; native reads require an already established channel. The
  composer and message surfaces are `test/attachment-{file,service,draft}.test.ts`,
  `e2e/attachments.spec.ts` and `acceptance/attachments.spec.ts`.
- Management subcommands precede operands: `tmt colab share link list <page>`,
  `tmt colab share mode <page> link --yes`.
- Full management includes `share member add/remove/role`, `share history`,
  `retention <page> [<days>|forever]`, `archive <page>` and `delete <page> --yes`.
  Member add is advanced/scripted raw-public-key use; invitation flows come with
  the Firestore stage. Widening and deletion require `--yes`. Explicit retries keep
  the operation ID, revision and selections, including after deletion.
- `--bin tmt-colab --test cli -- --test-threads=1` covers serving/stopped mutations,
  complete member assignments, confirmations/input refusal before IPC, original-head
  replay across restart, stale/conflicting requests and denied/uncertain socket replies.
- Tests that start the decoder use the shared `tests/support` test-only decoder
  configuration (60 s invocation, 30 s FIFO readiness); production keeps the
  two-second deadline and all caps. The decoder-timeout cases keep two seconds and
  must see exactly `Deadline`, confirmed cleanup and a recorded PID gone.
- Decoder load proof: compile first, repeat the affected decoder, transition and
  socket tests under at most two owned CPU burners (180 s per run, terminated and
  reaped on every exit), `CARGO_BUILD_JOBS=2` and that worktree’s `rust/target` (never an external
  `CARGO_TARGET_DIR`, per DEVELOPMENT); keep
  logs outside the repository; no Docker or release builds.
- On macOS the decoder reports `memory limit unavailable`; only Linux enforces the
  child address-space limit.
- The `yrs` 0.28.0 pin brings `smallstr` (RUSTSEC-2026-0215, unmaintained, no fix):
  accepted as maintenance debt with decoder containment and hostile-corpus gates.

## Firestore Rules emulator

The Colab admission Rules (`extensions/tmt-colab/firestore/admission.rules`) are proved against the
composed golden `contracts/vectors/deploy-declaration-v1.firestore.rules`, never the bare fragment.
After editing the Rules or declaration: run `contracts/vectors/deploy-declaration-reference.py --write`,
then regenerate the goldens into a staging directory with Remote's example (from `rust/`):

```sh
CARGO_BUILD_JOBS=2 cargo run --offline --locked -p tmt-remote --example compose_firestore -- ../extensions/tmt-colab/contracts/vectors/deploy-declaration-v1.json /absolute/staging
```

Review the three outputs, copy them over `deploy-declaration-v1.{firestore.rules,firestore.indexes.json,plan.json}`,
and run the oracle without `--write`, the Colab suites above and Remote's `deploy_vectors` test.

`tests/emulator/suite.mjs` has no npm dependency and no skip path: it runs only under
`firebase emulators:exec --only firestore` (firebase-tools 15.29.0, Java 21, pins identical to Remote's). Run it
locally in Docker in the booked slot: `scripts/dev-disk-check.sh` first (30 GiB free), one image tag per worktree,
remove the image after the run (DEVELOPMENT's disk section). CI runs selected changes in the hosted Unit tests lane ([selection](../../tmt-release/references/ci-selection.md)); Docker network-none/read-only isolation remains local qualification only.

```sh
tag=tmt-colab-rules:$(printf %s "$(basename "$PWD")" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9_.-' '-')
docker build -t "$tag" extensions/tmt-colab/rust/tmt-colab/tests/emulator
docker run --rm --init --network none -v "$PWD/extensions/tmt-colab:/c:ro" "$tag" firebase emulators:exec --only firestore --project demo-tmt-colab --config /c/rust/tmt-colab/tests/emulator/firebase.json --non-interactive 'node --test /c/rust/tmt-colab/tests/emulator/suite.mjs'
docker image rm "$tag"
```

Sensitivity: a negative case must fail for the intended guard only, so each case sits next to a
positive control that differs in that one condition. After changing a guard, remove it from a copy of the
composed golden and confirm the suite fails (the suite was checked this way for the epoch fence, open state,
retention bound, 365-day page ceiling, expiry-only refresh, expired-data cleanup, owner-only refresh,
writer role, contiguity, writer-uid binding of streams and objects, ID binding, link secret and role, membership reads, list page
expiry, space owner and owner-path rules).

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
`COLAB_SOCKET_PATH_TOO_LONG`. Browsers reach Colab at
`/r/<prefix>/x/colab/` through the Remote door, which `serve` attaches to or starts itself
(see the [colab-v1 contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#serve-and-the-remote-door-1584)).
Page source and export:

```bash
tmt colab page read <page-uuid> --json
tmt colab page write <page-uuid> --file page.html --expected-revision 'v1:<token-from-read>' --json
tmt colab export <page-uuid> --json          # or --dir /existing/export-parent
```

Use the exact revision from `read`; a stale base returns `COLAB_STALE_BASE` (exit 1)
and is never retried. A failed or uncertain serving IPC returns `COLAB_UNAVAILABLE`
without an offline fallback. Export needs an existing parent, creates a new UUID
directory with `page.html`, `conversations.json`, `conversations.md`, `manifest.json` and an
`attachments/` directory of the files it could read (never replacing output), and reports
`error.partialDirectory` on a failed publication. Attachments are read through the running serve,
so `tmt colab attachment read <page-uuid> --reference <manifest-row-reference.json> [--output <dir>]`
and the attachments of `export` need `tmt colab serve`; without it each is unavailable. `tmt colab attachment attach <page-uuid> <file>` also needs an established object channel (open the page once in a browser through `tmt remote`).

Serve and the door: the CLI suites in `tests/cli.rs` run a scripted `tmt remote ...` stand-in
(`Pilot::remote_core`: attach, start, not installed, door that dies, a wrapper that leaves a
grandchild, Ctrl-C while starting; `tmt colab stop` with started, attached and no serve, and a
forwarded-context refusal; page links with and without a door, `settings`, and auto-open through a
stub `open`/`xdg-open` in a private `bin` on PATH — never a real browser) with a private HOME and no real Remote. The real-binary case
is `acceptance/one-command.spec.ts` (needs `tmt`, `tmt-remote` and `tmt-colab` built; see
[acceptance.md](acceptance.md)); it links the extensions onto the world's PATH so the real core
resolves `tmt remote`.

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
- Cross-screen chrome also requires the socket fixture: build it with
  `cargo test --offline --locked -p tmt-colab --test socket --no-run --message-format=json`
  and set `COLAB_CHROME_FIXTURE_EXECUTABLE` to the executable from its compiler-artifact
  record. `e2e/chrome.spec.ts` runs its explicit ignored response producer, then checks
  the actual native fallback HTML/CSS alongside each React screen.
  `COLAB_CHROME_CAPTURE_DIR` selects its top and midpoint capture directory.
- `test:browser` runs in the advisory `colab-app` job of `colab-browser.yml` (no native
  fixtures there); run it locally too after changing app UI, harness fixtures or
  `src/fold.ts`. `e2e/cli.spec.ts`, `e2e/chrome.spec.ts` and `e2e/served.spec.ts` skip
  with a named reason when their native executables are absent (`COLAB_SERVE_EXECUTABLE`
  and `COLAB_PAGE_FIXTURE_EXECUTABLE`, `COLAB_CHROME_FIXTURE_EXECUTABLE`, or the built
  `rust/target/debug/tmt-colab`), so a run without them proves only the other specs.
  Design-review screenshots go to `COLAB_CAPTURE_DIR` (default
  `<tmpdir>/colab-app-captures`); never hard-code a host path in a spec.
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

Browser title checks are `test/title-cache.test.ts` (WebCrypto encryption, scope
binding and fallback), `test/live.test.ts` / `test/mounted.test.ts` (accepted fold
and registration fencing), `e2e/live.spec.ts` (real browser storage/reload and plain
text rendering), and `acceptance/titles.spec.ts` (native page creation, title
propagation and separate paired profiles). Run app gates and real-binary acceptance
when changing the fold publication/cache wiring; capture home/dialog light/dark and
mobile screenshots.

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
three-engine pass. The report retains source/toolchain/browser identity, frozen input
hashes, lifecycle timings, last started/completed checks and failure stacks. Progress
markers are synchronous test-only console messages; they do not replace assertions.
Cleanup is bounded separately from crypto execution and its failure refuses the gate.
For an explicitly approved Linux closure diagnostic, retain `DEBUG=pw:browser` stderr
alongside the report and exact runner image; a later pass does not explain an older
closure. Do not retry automatically or infer crash/OOM from a generic target error.
The advisory `Colab browser verification` workflow runs Chromium
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

The app's top-level files are declared once in
`extensions/tmt-colab/rust/tmt-colab/app-entries.txt`: the native inventory compiles it in and
the archive proof (`colab-runtime-proof.mjs`) reads it. Adding a top-level file means editing that
file; the Colab app CI job runs `node typescript/scripts/verify-colab-app-entries.mjs` on the real
build so a mismatch fails the pull request, not the release.

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
  --skills extensions/tmt-colab/skills \
  --notices /absolute/combined-notices.txt --license LICENSE
```

The extracted binary is copied to a fresh directory and must serve without
`--app-dir`, checkout output or pnpm on `PATH`; exact HTML, every asset and both
Rust and frontend notices must match.
