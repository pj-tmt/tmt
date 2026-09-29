# Development

The Rust workspace is the shipped CLI runtime. The optional Office SPA foundation
lives in `extensions/tmt-office/typescript/apps/office` and is not required to use the CLI. The nested
`typescript` pnpm workspace owns private developer tooling for Vitest, fixtures
and release verification. Nx orchestrates repository tasks without making the
tooling workspace an npm product or a CLI fallback. Repository policy is
in [AGENTS.md](AGENTS.md), architecture ownership in
[ARCHITECTURE.md](ARCHITECTURE.md), and style in [CONVENTIONS.md](CONVENTIONS.md).
Use this guide for reproducible commands and evidence.

## Setup

This section is for contributors, not end users. Rust/Cargo builds the product.
Node and pinned pnpm run Vitest, native-process and Docker orchestration, formatting,
type checks and release verification. `better-sqlite3` is an independent test
oracle; `tar` builds test archives. These are retained developer dependencies,
not reasons to install TMT through npm. npm is not a separate required workflow;
use the pinned pnpm lockfile rather than introducing another package manager.
Unless a command explicitly changes directories, pnpm commands in this guide run
from `typescript/`; Cargo, Nx and Docker commands run from the repository root.

Requirements are Node.js 22.12 or newer, the pinned pnpm toolchain, and the
Rust toolchain declared by `rust/rust-toolchain.toml`. The workspace MSRV is
Rust 1.88; CI also runs the current pinned release toolchain.
Shell completion tests require Bash and Zsh. Runtime proof uses the selected
macOS developer tools or Linux `readelf` (binutils); these are verifier tools,
not product runtime dependencies.

```bash
(cd typescript && corepack pnpm install --frozen-lockfile)
(cd rust && rustup show)
NX_DAEMON=false NX_INTERACTIVE=false ./nx show projects
```

The checked-in non-JavaScript Nx wrapper pins Nx 23.2.1 in `nx.json`. On first
use, the official wrapper uses npm only to populate ignored `.nx/installation`
runtime files; this isolated bootstrap is not a product workspace, a lockfile
owner, or an alternative to the nested pnpm commands above. Nx task caching is
disabled while this layout is established, and automated checks disable the Nx
daemon and interactive prompts.

Do not install the product globally while testing. Keep application state,
provider directories, temporary prefixes, sockets and child processes inside a
task-owned temporary root.

If `better-sqlite3` is used by retained tooling, it is a development-only
independent SQLite oracle. It is not the Rust runtime, a product dependency or
an excuse to reopen native schema state through Node.

## Run the workspace CLI

From this checkout's root:

```bash
(cd rust && cargo build --locked -p tmt-cli)
NX_DAEMON=false NX_INTERACTIVE=false ./nx run native:tmt -- --version
(cd typescript && corepack pnpm tmt office --help)
(cd rust && cargo build --locked -p tmt-office)
./rust/target/debug/tmt-office --help
(cd rust && cargo build --locked -p tmt-squad)
./rust/target/debug/tmt-squad --help
```

The `native:tmt` Nx target and nested `pnpm tmt` script both launch only this
checkout's `rust/target/debug/tmt`. They do not
build automatically, install anything, or fall back to a global binary. Rebuild
after changing Rust sources. For clean JSON stdout use
`(cd typescript && corepack pnpm --silent tmt ...)`.
The launcher preserves arguments, exit status and environment; normal CLI commands
still use the usual application data unless you explicitly select isolated settings.
It does not replace the installed Office companion or update a running Office UI.
The reserved `tmt office` facade and direct `tmt-office` command share the Office
parser and handlers. Direct commands resolve core lookups through
`TMT_EXECUTABLE` (when invoked by TMT) or an executable `tmt` on PATH; use an
isolated application home when testing writes.

Extensions can use the public [local process API](docs/extension-api.md) for
structured dispatch, history, conditional room writes and bounded notebook reads.
Its contract is owned by [architecture](ARCHITECTURE.md#local-extension-api-v1).

## Office SPA

Office native data contracts and codecs are in
`extensions/tmt-office/rust/tmt-office-model`, a member of the `rust/` workspace.
Use `cargo test --manifest-path rust/Cargo.toml -p tmt-office-model` for pure
admission/vector checks, and the affected adapter tests for file/storage behavior.
After changing these boundaries, run `cargo test --manifest-path rust/Cargo.toml
-p tmt-cli --test architecture`; the guard checks actual workspace dependencies
and the retained Office consumer inventory. Full isolated verification still
covers the runtime consumers; pure model tests do not replace it.

The optional app uses React, Vite, TanStack Router and Jotai. Read
[Office architecture](docs/office/architecture.md) before changing its boundaries.
From the repository root:

```sh
cd typescript
corepack pnpm install --frozen-lockfile
corepack pnpm office:dev
corepack pnpm office:check
corepack pnpm office:test
corepack pnpm office:build
```

The dev server binds loopback. Production output is `extensions/tmt-office/typescript/apps/office/dist`; preview
with `pnpm --filter @tmt/office preview`. A deployed SPA host must rewrite app
routes such as `/setup` to `index.html`; hosting and Firebase setup are not part
of the scaffold. Tests use jsdom and the real router, not a browser-layout proof.

`pnpm check` checks workspace tooling and Office. `pnpm check:tooling` retains the
native test/release tooling's independent quality gate. Tooling-workspace `test:run` still
selects only tooling tests; `office:test` explicitly selects app tests and fails
on empty discovery. Office uses Oxfmt; tooling and repository docs use the
`typescript/.prettierrc` Prettier configuration. Run
`pnpm --filter @tmt/office format` for app formatting, not the tooling formatter.
The distinct Vitest versions are lockfile-owned, not a claim that native tests
were migrated to the newer app runner.
Office wire-schema conformance is a nested tooling test. From `typescript`, run
`corepack pnpm exec vitest run test/tooling/office-contracts.test.ts`. See
[`extensions/tmt-office/contracts`](extensions/tmt-office/contracts/README.md) for its single source of truth,
versioning and limits. Design vectors are not executable authorization or crash
recovery evidence; downstream suites must prove those behaviors separately.
Root tooling runs at most two suite workers to avoid simultaneous subprocess
startup overwhelming the existing per-test budgets; assertion/time limits are
unchanged. Native process and tmux configurations keep their own execution rules.

For an Office-only clean checkout, use `pnpm office:install`. It installs from
the app directory against the same workspace lockfile, without workspace recursion.
With pinned pnpm 10.33, a plain Office `--filter` install still builds root
SQLite; the explicit isolated install avoids that unrelated dependency. Do not
create an app lockfile or remove `--frozen-lockfile` to work around a mismatch.
The script resolves the lockfile directory through an absolute path: pinned pnpm
can apply a relative path twice and place modules outside the checkout. The
non-root browser image verifies dependencies stay in its workspace. Firebase's
optional auto-configuration postinstall and protobufjs's version warning script
remain unapproved; explicit app configuration and bundled SDK code need neither.

The app's verification-only Dockerfile proves the isolated install has neither
the root SQLite oracle nor a native TMT executable. It runs quality, DOM tests
and a production build with networking disabled:

```sh
docker build -f extensions/tmt-office/typescript/apps/office/Dockerfile -t tmt-office-check:local .
docker run --rm --init --network none tmt-office-check:local
docker image rm tmt-office-check:local
```

Choose an unused task-owned image tag; remove only that verification image.
This is not a deployment image or a browser/Firebase E2E claim.

For native-only fixtures, `pnpm --filter tmux-team install --frozen-lockfile`
installs only tooling dependencies; Docker copies workspace/package metadata before
this step. Do not make native fixture containers install or execute Office.
The workspace explicitly permits lifecycle scripts only for the existing
`better-sqlite3` oracle and esbuild tooling. Fresh oracle builds need Python,
make and a C++ compiler; the tmux fixture image supplies these build tools.

CI always reports `Code quality` and `Native package matrix`. Selected Office
changes also run the required `Office SPA` job, which builds the existing
`browser-tests-base` target without executing Playwright. That target owns the
Office service check/test/build, Office type/lint/format/unit checks, browser-test
type checking and partition inventory, and local, preview, emulator and cloud SPA
builds. A conservative
diff selector skips expensive native jobs only for Office-only paths, and skips
Office for native-source/skill-only paths. Shared/unknown paths run both. Code
quality includes the selector's own focused tests even when native unit jobs are
unselected, and requires the selected Office check. The native aggregator rejects
failed, cancelled or unexpectedly skipped selected jobs. CI changes need positive
and negative selection/gate evidence before pushing; do not change branch
protection merely to get a newly skipped job accepted.

For the separate Office Auth/Firestore environment, follow
[`extensions/tmt-office/typescript/services/office/README.md`](extensions/tmt-office/typescript/services/office/README.md). It uses Docker-contained
Java and Firebase tooling with a demo project; no host Firebase login is required
for emulator tests. Local real-project mappings and credentials must remain
ignored by both Git and Docker. Never substitute this bootstrap smoke proof for
future membership rules, invitation or browser tests.

### Local browser sign-in

Start the emulators as described above, then explicitly opt in:

```sh
pnpm office:dev --mode emulator
```

Open the loopback URL printed by Vite. Choose **Sign in with Google (emulator)**,
add a local test account in the popup and observe its UID. No real Google login,
Firebase owner alias or credentials are needed. Reload signs out; another tab
does not inherit the session. Default `office:dev` / `office:build` stays a
disconnected preview. Login does not create a world or grant access.
Console-managed tester admission gates direct Firestore world creation/read.
For explicit real Google sign-in and owner-local configuration, see
[the limited cloud pilot](extensions/tmt-office/typescript/services/office/README.md#limited-cloud-pilot).
Do not point automated tests at a real project.

Run the real-browser suite with the same emulator owner:

```sh
docker build --target browser-tests -f extensions/tmt-office/typescript/services/office/Dockerfile -t tmt-office-browser:local .
docker run --rm --init --shm-size=256m tmt-office-browser:local
```

The image installs pinned Chromium, checks the app, runs DOM/session tests and
builds preview, emulator and unconfigured cloud variants. The container starts
disposable Auth/Firestore/Functions emulators and three strict-port preview servers, then
Playwright. The cloud build must fail closed without operator configuration;
it never contacts a real project during automated tests.
It uses one worker, no retries, bounded waits and independent browser contexts.
The complete local command above remains the acceptance entry point. CI schedules
the same standard browser identities as advisory diagnostics in twelve isolated
partitions to reduce the chance
that cold image builds and serial scenarios exhaust a per-job deadline:
emulator-backed contracts, three local Vite shards, and eight native-local shards.
Local partitions do not start Firebase, while native-local shards use the container
Secret Service and embedded companion without Vite or Firebase.
`test:browser:partitions` compares the exact Playwright identities from all twelve
partitions with the standard suite, rejects overlaps or omissions, keeps
`native-decoration.spec.ts` in the emulator partition, and separately proves that
the four opt-in capacity scenarios retain the full original inventory without
overlapping the standard browser inventory. Every partition keeps one worker, zero retries and the
existing scenario limits. Browser results do not gate merge aggregates; failed jobs
remain visible and retain their logs and artifacts. Native Rust CI still owns
formatting, linting, locked builds, embedded SPA service tests and process/parser
contracts. The container-native shards retain installed-browser diagnostics without
being rerun after compilation.
Browser matrices and Playwright commands continue after individual failures while the
runner remains active. A failed command uploads available Playwright error contexts
before a final fail-closed step records the advisory job failure. Native browser jobs have a
25-minute deadline so their cold image build leaves more time for each retained
4–9-test partition. That remains a best-effort diagnostic budget: several tests can
still consume their 120-second scenario limits after the build. A
deadline or runner termination can truncate a partition and prevent later artifact
steps despite their failure/cancellation predicate. Report completed and expected
counts together, including missing artifacts; do not claim a complete inventory from
the partition count alone.

### Office browser verification

A change affects Office when `typescript/scripts/ci-scope.mjs` selects
`native_office` for it: Office's own app, service, crates, contracts and skills;
every workspace crate under `rust/crates/` other than `tmt-cli`, except the modules
the script's verified denylist names; the CLI Office facade, API command and native install
commands; workspace build inputs; the shared test support and E2E harness the
image reads; and any path the script does not recognize. Squad, core skills, other
CLI code, prose outside Office, core-only test suites and E2E scenarios do not
affect it. `ci-scope.mjs` owns this mapping; its tests recompute what the Office
crates and the API module reach across the workspace crates so the denylist
cannot go stale.

- Run the local browser suite above before opening a PR for an Office-affecting
  change, and record the result in the PR.
- CI runs the Office browser partitions only when `office` is selected, and the
  native Office shards only when `native_office` is. Their results are advisory
  and never gate merge.
- If a remote browser job fails but the same tests pass reliably in the local
  suite, treat the failure as flaky: record the local pass in the PR and move on.
  Only a failure that also reproduces locally needs a fix.

Capacity diagnostics are preserved as explicit opt-in runs and are not required CI:

```bash
docker build --target browser-tests -f extensions/tmt-office/typescript/services/office/Dockerfile -t tmt-office-browser:capacity .
docker run --rm --init --shm-size=256m --network none \
  --env TMT_TEST_BROWSER_CHANNEL=chromium \
  tmt-office-browser:capacity \
  sh /workspace/extensions/tmt-office/typescript/services/office/with-test-keyring.sh \
  pnpm --filter @tmt/office test:browser:capacity
```

Only failed-transaction assertions use the Firestore fixture's 20-second budget:
the pinned SDK's five attempts can spend 12.1875 seconds in jittered backoff
alone. Keep the ordinary 10-second UI expectation and 30-second scenario limits.
Those scenarios assert intercepted transaction reads and independent durable
state before and after explicit retry; a timeout alone is not transport evidence.
Auth responses are real and local. The suite also exercises the real Firestore
SDK against Rules: immutable creation/retry, cross-user denial, self-grant denial,
shape validation, and browser grant/create/revocation. The separate agent-grant
Rules suite adds custom-principal tokens, assigned UUID blocks, claim/capability
isolation, malformed/expired grants and one-way owner revocation with unchanged
cached tokens. `pairing-service.spec.ts` adds actual HTTP approval/custom-token
issuance, concurrent claims and live revocation. Direct service composition with
an injected signer adds failure/recovery evidence against the same real database.
Those service scenarios alone are not evidence of protected native storage or
CLI-to-browser pairing. Service-only scenarios can run with `--network none`
by overriding the image command to select that spec. Operator fixtures
also independently inspect saved block layouts. Home-block scenarios cover
revision races, exact retries, Rules/client conformance vectors, bounded list
validation, owner/device isolation and browser placement/save/reopen/revocation.
Desktop and narrow editor screenshots are written to Playwright test results
for primary visual review, not treated as automatic visual acceptance.
Operator fixture writes
use a hard-wired loopback demo-project bypass, never production credentials.
Google's official popup transport still loads
public JavaScript from `apis.google.com`, even in emulator mode. The browser
allowlist permits only those script GETs and loopback, blocks optional upstream
CDN styling, and fails for any other destination. This suite requires Internet
access for that library; it is not a fully offline OAuth test. No host
credentials/volumes or published ports are used. A failed browser test must
propagate through `emulators:exec`; do not count a skipped or empty suite as proof.
The selected Office CI job runs this same target. Native tmux E2E remains
separate. Remove the task-owned verification image when no longer needed.

`pairing-browser.spec.ts` proves real browser consent -> issuer approval ->
original-proof claim -> scoped block write -> cached-token denial after owner
revocation, plus same-request reopening, non-owner denial, lost-response retry
and logout during a pending confirmation. Inspect the actual approval POST body
as well as durable state; URL-only capture is not proof of a secret-free body.
The originating proof is a fixture, not a native CLI. DOM/state tests separately
cover admission/route fencing and explicit consent. Browser and service decoders
consume the same literal pairing corpus; browser encoding tests use an independent
Node encoder. Review desktop/narrow screenshots rather than accepting their
existence as visual proof.

`native-pairing.spec.ts` installs the real compiled CLI/companion through a
synthetic verified archive, using the existing `typescript/test/support` process and artifact
owners. It resumes a protected proof across CLI processes after Chromium consent,
then independently checks the issued grant, OS-store record, scoped Firestore
read, token refresh, expired server/native lease renewal, simulated lost-response readback,
unchanged resources, corruption, revocation and same-name replacement.
The browser target therefore builds Rust and installs the root test-helper
dependencies with scripts disabled; native helpers never enter the SPA. The
standalone app image stays independent.
`with-test-keyring.sh` owns a disposable D-Bus session and real Linux Secret Service
with a fixture-only password and private container directories. The scenario
deletes and verifies absence of its protected entry; container teardown removes
the disposable store. No host Keychain, credentials or installed CLI is used.
This is Linux credential-store evidence, not macOS runtime or real release-archive
acceptance. Reuse the separate native artifact verifier for published artifacts.

`native-cancellation.spec.ts` verifies expired protected state, unknown-request
preservation, browser cancellation, secret-free receipts and same-identity fresh
pairing using the real CLI and isolated vault. `pairing-cancellation.spec.ts`
checks cancellation/approval races and denied unknown-request writes through the
real issuer. Neither replaces cached-token denial or retirement coverage.

`native-hooks.spec.ts` adds the causal retirement path using a private real tmux
server and the container's `sqlite3` tool as an independent state observer.
Verify local retirement plus pending notification before any remote cleanup,
then disabled pairing/grant, retained block and denial of a previously usable
cached token. A missing session bus or vault entry must retain pending work.
An unknown pending proof stays retryable and later cancels a known approval
without claiming credentials or creating a grant. A test-only SQLite
acknowledgment trigger separates remote/vault success from local completion;
retry must consume the protected revoked receipt without restoring credentials.
Neither hook registration unit tests nor service-only revocation proves this
chain. Run the Office browser target and tmux lifecycle suite twice after changes
to these cleanup boundaries, with verified private-server/vault cleanup.

For focused service checks use `pnpm office:service:check`,
`pnpm office:service:test` and `pnpm office:service:build`. `pnpm check` also runs
the combined `@tmt/office type:check:e2e`; standalone `office:check` deliberately
does not load service dependencies through E2E fixtures. The Docker browser
target runs both package checks and this explicit E2E type check on Node 22.

For in-progress native deployment discovery, run
`cargo test --locked -p tmt-office-pairing office_deployment` from `rust/`.
These tests verify literal
browser/native descriptor conformance, strict URL/JSON validation and bounded
unauthenticated HTTP; they do not prove native pairing or protected credentials.
`deployment.spec.ts` checks the actual built emulator descriptor and unavailable
preview/cloud variants. Issuer scenarios independently compare claim
`grantExpiresAt` with Firestore, including retries and a shortened grant; approval
and token expiry must not stand in for that value.

## Personal-office milestone acceptance

`retained-spaces.spec.ts` proves owner discovery of a revoked grant, opening and
editing its retained UUID block through the shared editor, independent stored
state, unchanged home layout, reopening and admission loss. The grant Rules
suite independently checks bounded owner pagination and denies unbounded,
oversized, foreign-owner, agent and revoked-admission queries. Owner editing
does not by itself prove native block mutation, reassignment or credential recovery.

Retained-block reassignment adds actual competing approval transactions in
`pairing-reassignment.spec.ts`: one source reservation, exact retries, abandoned
unclaimed recovery and denial of claimed ancestors. Browser approval selects
the source explicitly and preserves assignment across uncertain responses.
The native acceptance must resume the original protected proof, inspect the
same retained block and corroborate it in the browser and independent database;
seeded grants or a browser-only claim do not substitute for that chain. Keep
the former cached credential's denial and unchanged layout as separate assertions.

M1 acceptance is the offline local flow through the actual runtime owners: native
CLI and companion -> authenticated loopback browser -> shared SQLite -> service
restart. Browser approval, cloud credentials and Firebase emulators are not M1
prerequisites. Retained remote regression coverage separately uses the native CLI
and Office companion -> browser owner approval -> scoped credential use -> durable
resource change -> visible browser result, with demo-project Auth/Firestore
emulators; it is not an M1 prerequisite. Use deterministic mock agents, isolated
real tmux where relevant and Playwright Chromium. Extend the existing fixture owners
as each feature ships, not a parallel mock implementation of TMT. Independent
database observations must corroborate UI and command results. Separate green layer
suites are not proof of an integrated flow. The native pairing browser scenario
covers acquisition and scoped reads. `native-decoration.spec.ts` adds real remote
block show/apply, independent stored tokens/revisions, visible scene geometry, exact
retry without timestamp changes, invalid input, conflicting local callers and
read-only/revoked authority.
Local callers share fail-fast locks: pre-execution contention reports `OFFICE_BUSY`;
retrying the original intent after the winner must conflict without another write.
This is not proof of a Firestore precondition race. The existing browser transaction and Rules suites retain that
independent server-side coverage. Neither replaces the other's acceptance.

Automated acceptance must not call a paid model, use provider/host credentials,
contact production Firebase or require paid runners. Keep the native-only tmux
suite network-disabled. Office integration keeps service traffic inside its
isolated fixture; the browser popup's existing allowlisted Google script is an
explicit network exception, not permission for cloud data or model calls.
Use bounded readiness and polling, fail on empty/skipped acceptance, and verify
child/socket/buffer/state cleanup. Run lifecycle-sensitive local suites twice.

Cover approval, denial, expiry, duplicate/concurrent claims, wrong scope,
revocation with cached credentials, uncertain writes, reconnect, temporary
retirement and same-name replacement. Resource slices add revision conflicts,
profile/layout/notebook independence, board disclosure and installed-guidance
checks. Preserve real request/response and storage owners; no terminal-output
completion fallback or second memory store.

For local presentation-profile changes, run the shared profile vectors in Rust and
Office, SQLite create/no-op/retry/conflict plus retirement tests, companion/CLI and
protected loopback route tests, and the real local CLI→SQLite→browser→restart flow.
Capture desktop and narrow screenshots and inspect name, hair, clothing, mark and
offline-presence legibility. No cloud account or remote publication is part of this gate.

Direct-manipulation UI changes are covered by `local-office-direct-manipulation.spec.ts`
(click/drag/cancel, auto-apply, persisted Undo and room properties),
`native-local-catalog-drag.spec.ts` (catalog pointer previews, rejected/cancelled
drops, one-change auto-apply, exact history and native restart persistence),
`local-office-editor-hud.spec.ts` (desktop/narrow context controls) and
`native-local-skybridges.spec.ts` (native auto-apply and canvas meeting creation).
`world-yjs.test.ts` covers selective history, entity ordering, observation exclusion,
atomic gestures and lifecycle; `use-world-editor.test.tsx` covers the JSON/CAS queue,
acknowledgement races, explicit conflict recovery and refreshed native observations.
These are local history checks, not evidence of a deployed Yjs synchronization provider.
The remaining legacy browser scenarios below still contain explicit layout-mode
scripts and require migration before a release gate can claim full current-UI coverage.

`native-local-world.spec.ts` owns the installation-wide layout lifecycle: a lazy
furnished 2×2 Lobby plus four unassigned offices, explicit browser Save, one SQLite world revision, empty placements
remaining empty after restart, and no new per-identity block rows.
`typescript/test/support/office-world.ts` owns explicit legacy fixtures for older topology
scenarios; do not translate the evolving new-world preset to simulate old inputs.
`native-local-topology.spec.ts` owns personal-area removal, replacement Lobby,
retained identity content and real external-write conflicts using explicit legacy
inputs rather than retired terrain-authoring controls. Its error-state
desktop/short/narrow viewport checks require non-overlapping, operable HUD controls
without resizing the canvas. `native-local-walls.spec.ts` covers browser-authored
wall art, native exterior/support admission, retained rejected drafts, explicit
repair and restart. Shared read-only SQL and pointer helpers live in
`native-world-state.ts` and `world-editor-gesture.ts`; keep expected fixture extents
independent of production geometry and assertions inside their scenarios.
`native-local-composition.spec.ts` combines private-tmux saved/temporary identities,
furnished personal areas, a meeting set, editable wall objects and floating
Chat/Info/room-audience panels. Inspect its desktop/narrow renders after readiness;
it supplements, rather than replaces, the focused lifecycle and delivery tests.
`native-local-profile.spec.ts` independently verifies that profile edits, retries
and conflicts do not mutate the saved world or role definition, and retirement
retains profile and world bytes.
`native-local-meeting-modules.spec.ts` exercises the V4 in-world name entry,
independent canonical-room creation, layout Undo/Redo/Cancel, existing-room
reattachment, furnished Save, targeted membership and blocked spatial removal.
Independent SQLite reads distinguish room writes from layout writes.
`native-local-unified-areas.spec.ts` covers explicit legacy-to-v8 alignment,
acknowledged Undo/Redo, use changes at a stable floor pointer position, spatial
removal, canonical-room retention, a shared creation ghost and service restart.
Inspect its desktop and narrow screenshots for lamp/icon differentiation and
unchanged platform materials. Shared unified-area vectors prove Rust/TypeScript
floor and opening parity; projection tests cover occupancy-independent spacing
and inverse picking. Retained island vectors and strict freeform connectivity
remain separate compatibility tests.
The retained V4 scenario supplies 1536×1024 DPR-1 and narrow screenshots;
`native-local-central-grid.spec.ts` adds
sparse circulation, wall mounts and DPR-2 idle evidence. These fixtures do not
prove default-world conversion or final visual fidelity.
`native-local-agent-meeting.spec.ts` verifies agent Info → room selection → member
review → Save using the installed companion and independent SQLite/CLI reads.
It covers retained members, discard, draft protection across agent switching,
restart, unchanged world bytes and no dispatch. DOM dialog visibility is emulated
once in `src/test-setup.ts`; native browser tests own real modal/focus behavior.
Native browser navigation selects agents in the whole-world Directory; identities
without assigned areas do not acquire implicit rooms. Avatar reinstall/fallback
and inbox/reply scenarios use the same installed companion and isolated fixture.

`native-local-module-keyboard.spec.ts` verifies keyboard name entry and button
activation, Undo/Redo/Save, restart and narrow-screen object admission/repair,
with independent SQLite observations and no terrain-painting interface. Native
select values use Playwright selection; this does not prove OS-level popup
keyboard navigation, which requires a separate native-input accessibility check.

`native-local-prop.spec.ts` verifies whole-world placement, directional artwork,
per-object customization, restart and missing/corrupt-pack recovery without an
identity-owned room. Observe layout bytes independently of catalog mutations;
compare rendered pixels, not only object selectors. `native-local-prop-capacity.spec.ts`
covers full catalog admission, overflow immutability and actual visible artwork
from every installed pack. Keep pixel probes clear of architectural occlusion.
`native-local-workstation.spec.ts` verifies source-derived bundled furniture and wall props through
real catalog placement, authored chair views and native Save/reopen. Offline art
encoding and its source-review gates are documented in the
[modular visual package](docs/office/references/rooms-and-walls/modular-v1/README.md).
`native-local-furniture-rotation.spec.ts` verifies retained static furniture's
directional successor through corner gestures, precision rotation, exact
Undo/Redo and restart. Its runtime gallery captures all four views of the
[directional furniture](docs/office/references/furniture-rotation/README.md);
review those images as well as the state assertions when changing this art.
`native-local-furniture-base.spec.ts` owns full-art upper hit testing and frontmost
selection independently of shallow physical support. Its real drags distinguish
supported overhang from unsupported bases, retain atomic Undo/Redo and restart,
and exercise all directions plus the narrow-screen rotation control.
`native-local-room-materials.spec.ts` checks exact world-pixel Undo/Redo, retained
content and bounded finish textures. `captureWorldScene` excludes HUD presentation
for pixel comparisons; separate unmodified screenshots verify the visible HUD.
For the accepted platform style, review the same seeded scene at a fixed viewport,
DPR and browser version: a Lobby with four offices, both bridge axes, meeting
branches, selected objects, and valid/invalid drag previews. Geometry assertions
lock the connector constant and inverse picking; behavior assertions lock no-write
invalid/cancelled drops and one completed gesture per Undo/Redo step. Current
skybridge/direct-manipulation tests retain native lifecycle evidence. The separate
opt-in `playwright.visual.config.ts` owns two focused visual scenarios under
`e2e/visual/*.visual.ts`, using the existing local HTTP fixture and real scene/HUD.
They protect platform lighting, supported overhang, held rotation/selection, and
desktop/narrow inspector and creation spacing. They do not prove native admission
or persistence, and do not join the standard browser CI partitions.

From `typescript`, run `pnpm --filter @tmt/office test:visual`. The config starts
only the offline Vite server, uses the lockfile-pinned Playwright Chromium at DPR 1,
fixed viewport/locale/color scheme and reduced decorative motion, and refuses to
create missing baselines by default. Install the matching browser with
`pnpm --filter @tmt/office exec playwright install chromium` when necessary.
PNG names include the host platform; a missing platform baseline is not permission
to copy another platform's pixels or claim cross-platform equivalence. Failed
comparisons preserve expected/actual/diff images in Playwright's test-results.

After an intentional design change, explicitly run
`pnpm --filter @tmt/office test:visual --update-snapshots=all`, inspect every changed
PNG against the previous baseline and the approved design, then rerun without the
update flag. Never regenerate a baseline merely to make a failing comparison pass.
Keep screenshot tolerances strict and verify representative defect sensitivity;
portable geometry/behavior assertions remain with their existing unit/native owners.
`native-local-world-capacity.spec.ts` exercises dense tile-budget and connected
sparse worlds through native admission and the browser. `scene-observation.ts`
observes actual WebGL submissions and texture lifetimes across zoom, pan, revisit
and replacement; the idle window must submit no new draws. Keep measured startup,
CPU and heap evidence separate from portable assertions: texture counts are not
GPU bytes, and JS heap is not total browser memory.

For local discussion-board changes, extend `native-local-discussion-admin.spec.ts`: use
one-shot CLI mutations while the service is stopped, the rendered browser through
the real loopback API, a fresh CLI read and independent SQLite observations. Cover
category isolation, inert text and attribution, board-specific stale revisions with
unchanged storage, owner moderation tombstones, CLI replies visible after browser
refresh, restart durability, loopback-only traffic and an
asserted stopped service. No browser route interception or cloud approval is part of
this local gate.

`native-local-service.spec.ts` owns installed-service faults and cleanup: separate
browser/control authority, live-session reuse, bounded incomplete-write shutdown,
rotated credentials, version drift, dead-child recovery, occupied ports, early
companion exit and uncertain receipts. Use the shared native Office fixture;
do not recreate its installer, process sandbox or executable selection in scripts.

`extensions/tmt-office/typescript/apps/office/e2e/native-local-discussion.spec.ts` owns spatial presentation checks:
opening and closing over the same panned canvas, unsent draft retention, explicit
post/reply persistence without task dispatch, mobile navigation and focus return.
It uses the shared native Office fixture, not a second service harness.

Whiteboard drawing and snapshot/reference scenarios share that fixture. The
snapshot scenario paints in a real browser, checks frozen JSON/PNG against SQLite,
copies a token-free reference, stops the web service, then verifies native reads
and no-clobber PNG export. It saves the exported image for visual inspection;
structured text alone is not image-access evidence. The Send scenario additionally
checks no enqueue before confirmation, frozen recipient UUIDs across retirement
and same-name recreation, exact-envelope replay, native inbox/reply/result and PNG
access. Mounted-state tests cover uncertain transport and exact Retry send input;
the native scenario does not intercept routes to fabricate a successful send.
After the sequential SPA and
native builds below, run these without Vite servers or Firebase:

```bash
pnpm --filter @tmt/office test:browser:native native-local-whiteboard
```

The native browser config reuses the normal one-worker/no-retry policy. Fixtures
own and stop their isolated services. `TMT_TEST_BROWSER_CHANNEL` selects the local
browser (default `chrome`).

`native-local-conversation.spec.ts` verifies direct chat against that same native
fixture: draft retention, target-switch confirmation, inbox delivery, real CLI
reply, browser display, and reload recovery after the host accepts a send but its
HTTP response is dropped. Independent SQLite counts prove recovery creates no
duplicate requests and reading the chat does not acknowledge incoming work.
`native-local-direct-wake.spec.ts` uses an isolated real tmux server and installed
companion to prove a new direct request sends only a request-ID and explicit
recipient-UUID instruction to
the verified pane, replay sends no second notification even after uncertain
paste, and an offline recipient keeps durable inbox acceptance without pane input.
The transport-fault case forwards to the real host before aborting the browser
response; it never fabricates acceptance. It captures desktop/narrow screenshots
and asserts service shutdown. State tests separately cover bounded page/body
reads, observation deadlines, hidden-tab cancellation and failed-read recovery.

`native-local-status.spec.ts` adds private-tmux actors and real CLI self-reports:
one canonical record reaches the directory and Info, browser-only time advancement
expires the rendered cue without another read or stored mutation, and a real
request/reply takes visual priority without changing status. It checks saved and
Contractor presentation, restart persistence and desktop/narrow rendering. Unit
tests own exact deadline/rollback scheduling and appearance/status read races.

Build the local SPA from the repository root:

```bash
NX_DAEMON=false NX_INTERACTIVE=false ./nx run office-spa:build-local
```

After the SPA build exits successfully, run the pinned toolchain from `rust/`,
where `rust-toolchain.toml` applies. Never overlap these producer/consumer steps:
Cargo can otherwise reuse the old embedded assets before Vite replaces them.

```bash
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo clippy --locked -p tmt-office --features local-service -- -D warnings)
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-office --features local-service)
(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" CARGO_PROFILE_DEV_DEBUG=0 cargo test --locked -p tmt-office --features local-service)
(cd rust && CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-cli)
```

Finally, return to the repository root and exercise the real installed-style flow:

```bash
(cd typescript && corepack pnpm office:local:e2e)
```

Run the complete applicable local gates before pushing the reviewed commit.
Record commands, results, exact commit, limitations and cleanup in its PR/issue.
Do not use repeated CI pushes for local debugging. Existing required CI gates
remain authoritative until a reviewed cost-policy change provides replacement
evidence and merge requirements; unselected jobs are not passing selected tests.
Do not activate paid runners, model APIs, billing or production deployment.

Real Google login, IAM, deployment and live-agent usability are separate owner-
authorized pilot checks. Request owner assistance when those checks are actually
ready. Emulator success does not prove production IAM or device credential
protection, and a live-agent demo does not replace deterministic regression tests.

## Rust checks

The companion package lives at `extensions/tmt-office/rust/tmt-office`, but remains
in the `rust/Cargo.toml` workspace. Run the same package commands from `rust/`;
the shared lockfile, toolchain and `rust/target` artifact paths are unchanged.
Docker build contexts must include both `rust/` and `extensions/tmt-office/rust/`.

Office storage migration tests live in `tmt-office-storage`
(`cargo test --locked -p tmt-office-storage`) and build their source databases
under temporary roots through the public core storage entry point. For manual
diagnostics, the companion's hidden
`tmt-office __tmt-office-storage 1 <status|prepare|copy|verify|switch|recover> --global-dir <absolute directory>`
requires an explicit disposable root and never uses configuration discovery.
Never point it at a real installation. Real migration requires the separately
consented user path.

Run from `rust/` for a normal native change:

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
cargo build --locked --example storage-probe
cargo build --locked --example tmux-probe
cargo +1.88.0 build --locked
```

Before native installation/process tests, build the two product fixtures
independently, after workspace checks:

```sh
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-office
CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p tmt-cli
```

Workspace builds unify adapter features, so their debug CLI can include Office
dependency debug information. Package-scoped builds exercise the ordinary CLI
product without Office features. These fixtures omit debug symbols, not debug
assertions or runtime checks; installation tests should hash/package executable
behavior rather than large DWARF payloads. Keep the existing process deadlines
and assertions. Both binaries remain in `target/debug`, using the established
selectors. This does not replace optimized release-archive verification.

The same unification applies to any combined build: `cargo build -p tmt-cli -p
tmt-squad` (or a workspace build) may compile shared dependencies with features
that only another package enables, so its `tmt` can differ from the product.
Product identity proofs are package-scoped, matching per-product release builds:
compare `cargo build --locked --release -p tmt-cli` alone, at the same checkout
path, before and after a change.

The architecture guard is included in `cargo test`. A focused offline run is:

```bash
cargo test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
cargo +1.88.0 test --offline --locked --manifest-path /absolute/checkout/rust/Cargo.toml --test architecture
```

When changing a guard, exercise a real positive and negative source/dependency
fixture and restore the checkout exactly. A stale lockfile or an unexecuted
test is not evidence that the guard worked. Core must remain free of concrete
I/O; adapters own SQLite/files/processes; CLI owns grammar and composition.

## Native process and shared tests

### Selecting the CLI under test

The maintained JavaScript suites live under `typescript/test/native/`, `typescript/test/e2e/`,
`typescript/test/tooling/` and `typescript/test/support/`. Rust tests stay beside the owner in
`rust/crates/*` or the companion package under `extensions/tmt-office/rust/`.
The native process selector resolves the repository build at
`rust/target/debug/tmt` by default and fails if it is absent. An explicit
descriptor may select another absolute native executable; it must be
executable, and neither an installed host command nor Node is an allowed
fallback. Paths and argv are passed as data, never through shell fragments.

Suites that start the local Office service (such as `office-storage`) need a
companion built with the embedded SPA, as CI does:
`(cd typescript && corepack pnpm office:build:local)`, then
`(cd rust && TMT_OFFICE_SPA_DIR="$PWD/../target/office-spa" cargo build --locked -p tmt-office --features local-service)`.
A later plain workspace `cargo build` or `cargo test` replaces
`rust/target/debug/tmt-office` without the service, and those suites then fail
with `OFFICE_SERVICE_UNAVAILABLE`; rebuild the companion before rerunning them.

Native Rust CI explicitly selects the same-checkout release CLI for process
contracts, so installed-companion integrity checks run with production compiler
optimization rather than debug hashing cost. The independent Office companion
and storage probe remain debug fixtures. Rust debug tests, Clippy, MSRV builds
and embedded service tests remain separate required checks; process deadlines
and assertions are unchanged. Local selection still defaults to the debug CLI.

Build first, then explicitly select the test-only storage probe. The product CLI
uses its repository-native default; the probe is never an installed SQL command:

```bash
cargo build --locked --manifest-path rust/Cargo.toml
cargo build --locked --manifest-path rust/Cargo.toml --example storage-probe
TMT_TEST_STORAGE_PROBE='{"executable":"/absolute/checkout/rust/target/debug/examples/storage-probe","args":[]}' \
  pnpm test:native
```

The maximum-body, 50-reply installed-companion page is an explicit load diagnostic,
not required process acceptance:

```bash
pnpm test:stress:native
```

For a deliberate explicit selection or a task-owned moved binary:

```bash
TMT_TEST_CLI='{"executable":"/absolute/checkout/rust/target/debug/tmt","args":[]}' \
TMT_TEST_STORAGE_PROBE='{"executable":"/absolute/checkout/rust/target/debug/examples/storage-probe","args":[]}' \
  pnpm test:native
```

The suite covers grammar, configuration-before-effects, identity metadata and
binding lifecycle, role/preamble, response/receipts, exchanges/attention, inbox listening, talk,
local Office board grammar/persistence, managed skills and native installation. It uses bounded process budgets,
task-owned files and independent SQL/schema oracles. Frozen migration fixtures
and provenance under `typescript/test/fixtures/storage-history/` are immutable evidence;
do not generate expected data with the implementation under test.
For schema changes, update the independent native schema expectation in
`typescript/test/native/storage-fixture.ts` and the explicit migration/table assertions,
then run this complete process suite before pushing. Rust storage tests do not
replace process-level migration and future-version rejection tests.

Runtime resume mappings have deterministic adapter tests and a separate opt-in
provider parser contract. With the explicitly selected Claude Code 2.1.283 and
Codex CLI 0.157.1 executables already installed, run:

```bash
cargo run --locked --manifest-path rust/Cargo.toml -p tmt-adapters \
  --example runtime-contract -- /absolute/claude /absolute/codex
```

This invokes only `--version` and the driver's generated resume arguments with
`--help`, under bounded process ownership. It neither installs providers nor
starts a model conversation. A different version fails rather than silently
expanding support; review provider behavior before updating the pinned contract.
Help-parser acceptance does not prove that a session can actually resume: the
manual lifecycle evidence in issue #321 owns that distinction, including the
Codex cross-mode limitation. Normal `tmt run` does not execute this developer
check or enforce these version pins on user commands.

`tmt whoami --context [--json]` is the read-only rehydration entry point. It reports
the verified caller identity and lifetime, up to 500 characters of role text,
an existing saved notebook path (not its contents), and unacknowledged originated
and incoming X counts with explicit-identity inspect commands. The complete output
is limited to 4 KiB, with a truncation marker when role/path content is shortened.
Only a verified empty pane receives the binding hint. Unavailable or ambiguous
evidence returns empty human output or JSON `status: "unavailable"`, successfully,
without initializing configuration, storage or tmux metadata.
Ordinary `whoami` keeps its existing human output and
adds `interfaceKind` and `sessionState` to its JSON projection.

The `identity-context` and `identity-context-requests` Docker scenarios own bound
context acceptance. Their independent SQLite snapshots include verification
timestamps, retention and both participants' attention state: context reads must
not change them. `typescript/test/native/context.test.ts` owns unbound/missing
configuration no-side-effect checks; Rust storage/formatter tests own retained
counts, role limits, escaping and the complete serialized byte bound.

`typescript/test/native/inbox.test.ts` owns the real no-tmux queue -> bounded listen ->
detail/receipt -> reply -> result path. It uses isolated SQLite, verifies compact
listen output excludes receipts and bodies, preserves participant-scoped
acknowledgment, and starts no Office process. Rust request/storage tests own route
validity, revision CAS, migration and indexed observation. Fake-clock unit tests
own debounce/deadline timing without a real 15-minute wait.

The tooling unit suite is independent developer-tool coverage. Its denominator
must contain no deleted TypeScript source and must not present Rust as a
cross-language percentage. Run focused tooling tests with:

```bash
(cd typescript && corepack pnpm exec vitest run test/tooling)
```

Native process tests must prove the missing-native negative control and selected
native positive control. Child processes are finite, are stopped and reaped
before fixture deletion, and receive signals only when they are task-owned.
Tests never use host tmux, global provider state, or process-wide environment
mutation as setup.

Use `withSandbox` for callback-owned native fixtures. Its descriptor clones
share active runs; disposal stops outstanding commands before deleting files
and rejects later launches. Each run has its execution deadline plus at most
one second to confirm direct close and process-group exit. Unconfirmed cleanup
fails and reports the retained fixture path instead of deleting potentially
live state. Focused lifecycle regressions live in `typescript/test/tooling/cli-process.test.ts`;
they use explicit Node fixtures, not a product-runtime fallback.

### Squad extension

`typescript/test/native/squad.test.ts` runs `rust/target/debug/tmt-squad`, or an
absolute path in `TMT_TEST_SQUAD`, through real `tmt` dispatch. It uses a
sandbox PATH holding the `tmt-squad` and `tmt-sq` links, with no installed-copy
fallback. It observes rooms and metadata through an independent SQLite reader.
A workspace `cargo build --locked` produces the default executable. Squad unit
tests run with `cargo test --locked -p tmt-squad`. For dependency changes,
compare `cargo tree -p tmt-cli -e normal,build -f '{p} {f}'` with `main` and the
package-scoped release `tmt` (see Rust checks) to prove the CLI is unchanged.

### Provider setup and lifecycle verification

Setup planning/publication tests use disposable settings files and preserve user
hook/permission bytes, exact reruns, recovery copies and changed-input refusal.
`test/native/setup.test.ts` owns CLI consent, stable-launcher repair/removal and
the always-zero bounded hook failure contract, including custom `CODEX_HOME`
without trust/config mutation. Runtime adapter tests own Claude/Codex payload
mapping, pane-ancestor evidence, continuation transitions and stale-end rejection;
core/storage tests retain session CAS, unique exact-thread lookup and transaction
ownership. Docker E2E owns real pane/process integration, including Codex's
independent-to-shared transition and unmapped shared rejection. Fixture hook
execution proves TMT integration, not provider-version compatibility or trust UI.
Manual provider acceptance must use a disposable identity/window and isolated
provider settings; installing hooks into the user's real global settings needs explicit
consent. No test invokes setup against the user's actual provider directory.

## Docker E2E

Run the full private tmux/caller lifecycle harness twice for lifecycle,
transport, identity, talk, or cleanup changes:

```bash
(cd typescript && corepack pnpm test:e2e)
(cd typescript && corepack pnpm test:e2e)
```

The harness builds its pinned image, uses `--network none`, private tmux
sockets and deterministic mock agents, and selects the Docker-built native
executable. It must not connect to a host tmux server or a real agent. The
wrapper's `--init` behavior is part of orphan-child cleanup evidence.
The image puts its selected dev/release binary at `rust/target/debug/tmt` inside
the fixture and leaves the shared selector unset. This exercises the same
default path as a developer checkout, including nested replies; explicit test
descriptors still override it. There is no second default executable owner.

### Scenario ownership

Name Docker scenarios for the behavior or adapter they verify, not a retired
implementation language. All public-command scenarios already use Rust.
Keep these boundaries when choosing where a regression belongs:

| E2E file(s)                                                    | Distinct evidence                                                                                                                     |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `binding-reconciliation`                                       | Publication races, uncertain endpoint evidence, scoped/global discovery, binding transitions and live listing presentation            |
| `marked-binding`                                               | Explicit marked-pane selection, frozen target evidence, server isolation, mark preservation and binding lifecycle reuse               |
| `durable-identity`, `identity-lifecycle`, `identity-retention` | Stored identity/profile continuity, pane/server lifecycle and retirement consequences                                                 |
| `check-routing` versus `capture-limits`                        | Real target/server resolution and capture failures versus numeric/configuration bounds                                                |
| `profile-binding` versus `role-lifecycle`                      | Verified caller/retired-name ownership versus restart, file input, validation and concurrent writes                                   |
| `talk-completion` versus `durable-talk`                        | Observer interruption, output/attribution and uncertain delivery versus inline input, size bounds, concurrency and submission retries |
| `exchange-watermarks` versus `exchange-attention`              | Gated revision/watermark state and exact late finals versus config isolation, redaction and rebind access                             |
| `tmux-adapter`, `transport-adapter`                            | Explicit adapter-probe evidence: caller/inventory/markers and delivery/capture stages; not public CLI success                         |
| `pane-badge`                                                   | Default-off behavior, opt-in updates, theme preservation, rendering, conflicts and cleanup                                            |
| `executable-selection`, `smoke`                                | Harness selection, causal nested replies, startup and cleanup controls                                                                |

Similar commands do not imply duplicate evidence: native-process tests inspect
the executable's public contracts and independent stored state, while Docker
adds real tmux/process ordering. Consolidate only after mapping setup, causal
action, assertions and failure/cleanup observations. The redundant basic badge
case formerly in the binding suite is owned by the stronger `pane-badge` cases;
do not add another copy there.

`typescript/test/e2e/cli-assertions.ts` owns `expectJsonResult` for the E2E result shape.
It checks the success envelope and returns the same parsed value; it does not
validate domain fields. Scenarios retain their exact/partial payload assertions
and independent SQL oracles. Helpers with a different stderr or parse contract
remain local. Do not combine partial identity views into a permissive shared
schema or import product types to manufacture expected results.

Shared cross-suite utilities belong in `typescript/test/support/`; suite-only harness,
assertions and observers stay with their suite. Focused helper tests belong in
`typescript/test/tooling/` and must prove rejection as well as positive behavior.

For ordinary developer checks, run:

```bash
(cd typescript && corepack pnpm check)
(cd typescript && corepack pnpm test:run)
```

The nested `pnpm check` script is the common quality entrypoint for all retained
tooling, including E2E. For focused work use `pnpm type:check`, `pnpm lint` or
`pnpm format:check` from `typescript/`; the old duplicate `e2e:*` quality aliases
are removed. `pnpm test:e2e` remains the actual Docker scenario runner.

The first command checks TypeScript types and lint/format for retained tooling
and docs; the second runs tooling behavior and source-boundary tests. Neither
replace the Rust commands, native process suite or Docker runs.

## Runtime smoke matrix

The six required native smoke environments are:

- macOS x64 and macOS arm64;
- Linux glibc x64 and Linux glibc arm64;
- Linux musl x64 and Linux musl arm64.

CI builds four raw targets once (the two macOS targets and two static Linux musl
targets) and reuses the matching static musl executable for both Linux smoke
environments. Preserve the `Packed install (<environment>)` check names and
`Native package matrix` final aggregator; those names retain historical CI
compatibility, while their
steps are real native runtime checks rather than npm package checks. Each smoke
runs outside the checkout with isolated HOME/state, no Node or Rust on the
product PATH, and checks version/help, exact embedded skill bytes, managed skill installation and
SQLite reopen/persistence. Cross-compilation alone is never claimed as runtime
evidence.

The same distinction applies to release artifacts: raw PR executables prove
source-runtime behavior only. They do not prove archive inventory, notices,
checksums or bootstrap behavior. Linkage is checked in both raw and archive proof.

## Native release verification

For archive, installer, upgrade, bootstrap or publication changes, read the
complete [native release verification guide](docs/native-release-verification.md)
before acting. It owns the exact commands, actual-archive evidence and authorized
publication gates. Ordinary changes do not need to load release-only procedures.
The runtime smoke matrix above remains part of native PR verification; raw
executables are not proof of release archives or public installation.

## Installed guidance source ownership

`skills/tmux-team/SKILL.md`, `skills/tmt-inbox/SKILL.md`, and the optional
`extensions/tmt-office/skills/tmt-office/SKILL.md`, `extensions/tmt-office/skills/tmt-prop-create/SKILL.md`, and
`extensions/tmt-office/skills/tmt-avatar-create/SKILL.md` are the five
canonical guidance sources in one versioned bundle. Core install exposes only
the first two; explicit Office install or upgrade manages all three Office siblings
in detected and already-managed custom roots. Verify exact embedded bytes,
core-only preservation, sibling
managed links, repeat no-op, backup/conflict and partial-failure behavior, lock
ownership, refresh without resurrection, and no effects on SQLite or tmux.
Follow `USER-GUIDE.md` and `skills/README.md` for provider/custom-root usage; do
not add provider-specific skill copies. The squad lead skill
(`extensions/tmt-squad/skills/tmt-squad/SKILL.md`) is deliberately outside this
bundle: the squad executable embeds it, and its native test checks the
documented status row shape against real output. Extension-owned skills
(`skills.install`, owned by an extension rather than this bundle) are covered by
`skill_installation::owned_tests` with isolated provider roots: publish, repeat
no-op, core and cross-owner claims, force backup, Office adoption, removal by
owner and drift. Runtime/linkage proof shared by archive
and raw verification lives in `typescript/scripts/native-runtime-proof.mjs`.

## Review and evidence

Before handoff, report the changed owner and run the smallest relevant focused
tests, then the complete gates required by the issue:

1. `pnpm check` and retained tooling tests;
2. Rust fmt, clippy, locked tests, build and MSRV build;
3. the complete native process suite, plus the tooling
   selector's missing-native negative and selected-native positive controls;
4. two complete Docker E2E runs for lifecycle/transport changes;
5. available real-host smoke environments, with remaining architecture coverage
   left to the required CI matrix;
6. matching-host artifact/bootstrap proofs for release changes.

Do not turn a filtered, skipped, cross-compiled, or failed subprocess into a
success claim. Preserve exact bytes and independent oracles. Keep returned
errors distinct from crash recovery, and retain uncertainty, transaction,
retention, acknowledgment, lifetime and cleanup evidence when changing those
boundaries.

Test design and causal positive/negative controls belong to
[CONVENTIONS](CONVENTIONS.md#tests-and-review); isolation and evidence ownership
belong to [ARCHITECTURE](ARCHITECTURE.md#testing-and-evidence-boundaries).
Compare exact file bytes, not symlink-directory snapshots or enumerated binary
objects. Use structured output or a focused formatter test, not mocked
`console.log`. Apply the [architecture maintenance contract](ARCHITECTURE.md#maintenance-contract)
when changing an owner, boundary or verification procedure.
