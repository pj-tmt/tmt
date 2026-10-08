# Ask agent real-binary acceptance (#1110)

`extensions/tmt-colab/typescript/app/acceptance/` drives the built `tmt`, `tmt-remote` and
`tmt-colab` with real Chromium devices. It needs no Docker and adds no production seam.

## Run

Build the app before the binaries so the native build embeds the current assets.
From the repository root, point `TMT_ACCEPTANCE_BIN_DIR` at the directory holding the
three binaries (default `rust/target/debug`):

```bash
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match build
CARGO_BUILD_JOBS=2 TMT_COLAB_APP_DIR="$PWD/extensions/tmt-colab/typescript/app/dist" \
  cargo build --locked --manifest-path rust/Cargo.toml -p tmt-cli -p tmt-remote -p tmt-colab --bins
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-app --fail-if-no-match test:acceptance
```

If also running [native Chromium fixtures](development.md#app-and-browser-client),
compile those fixtures before the final binary build: their test configuration can
replace the regular `tmt-colab` artifact in a shared target directory. Copy the final
three binaries into a dedicated execution directory, select it with
`TMT_ACCEPTANCE_BIN_DIR`, and record the tested head, binary and embedded asset hashes
before execution. Keep that directory unchanged throughout both acceptance runs.

Run it twice for lifecycle acceptance; it remains a recorded manual gate on the PR head.
The separate advisory `colab-browser.yml` acceptance job runs once weekly, manually,
or for a PR carrying `colab-acceptance`, using one app/native build and one Playwright
worker. PR runs test the default merge ref and record both the PR head and tested merge SHA
in `tested-head.txt`. It retains exact build hashes, test outcomes and bounded assertion locations/timeouts
without private error values, traces or profiles. An unexpected outcome also prints this
bounded summary even if the list reporter's detailed failure is absent from the job log;
selection and concurrency are owned by the [CI reference](../../tmt-release/references/ci-selection.md#advisory-browser-selection).
Set `TMT_ACCEPTANCE_KEEP=1` to keep a world's root (counter rows, `*.stderr`) after a run.
Use that setting only for diagnostics: the harness's injected-failure case requires root deletion,
so a retained-root run cannot satisfy the complete suite's cleanup gate. A Colab PR handoff
names the acceptance specs it ran and the exact tested executable/asset hashes.

## World

Each scenario gets one world (`harness/world.ts`) under a short `/tmp` root, because Unix
socket paths are limited to about 100 bytes:

- private HOME and XDG roots, and a private tmux server reached only through a `-L` wrapper
  on `PATH` (the tmux `TMUX` session value has no `$` prefix, or `tmt name` cannot find its
  pane);
- the real `tmt-remote` door with the real `tmt-colab` mounted, started by `startDoor`;
- one Chromium profile per paired device (`pairBrowser`: the real `pair --json` ceremony), so
  two viewers have separate keys, IndexedDB and cookies;
- a recipient pane (`harness/recipient.mjs`) that appends a durable `received` row before it
  replies through the real `tmt reply`. Count work from those rows, never from terminal echo;
  a duplicate wake is a second row for the same request;
- `TMT_EXECUTABLE` for Remote and Colab is a wrapper (`harness/core-barrier.mjs`) that
  forwards to the real core, records every launch (`coreCalls()`), and can park one
  `dispatch.create` before or after the core acts (`armBarrier`, `barrierEntered`,
  `releaseBarrier`) so a scenario kills and restarts a process there.

`withWorld` always disposes. Disposal stops every process group, closes the browsers, kills the
tmux server and fails the test if a process naming the root or tmux socket, or any other socket,
remains, including after a failed scenario. `harness.spec.ts` proves the harness, including the
sensitivity of that check.

## Pages

A page needs a real owner command: `tmt colab page create --title <title> [--file <path|->]
[--json]` (empty source by default; a file or stdin otherwise), which works while `serve`
runs. `--json` returns `{spaceId,pageId,title,path,operationId,membershipHead}`; open `path`
under the Remote door address (`createPage` and `openPage` in `harness/ask.ts` do this).
`tmt colab ls --json` and `tmt colab page read <page>` verify it. Create the page before the
browsers register: each paired device registers when it first opens the app.

## Cases

`agent-status.spec.ts` reads the real admitted directory through the Agents drawer,
checks its served asset hashes and CSP, and injects a labelled context-read refusal
without sending or reopening. By default it compares every served asset with the app's
current `dist` build. For a relocated frozen binary run, set `COLAB_STATUS_ASSET_MANIFEST`
to an independently retained build report containing all eleven `distAssets` paths and SHA256
digests, including `THIRD-PARTY-NOTICES.txt`; optional
`COLAB_STATUS_NATIVE_CAPTURE_DIR` saves 1440/390 light/dark originals. Its injected
refusal does not diagnose an existing browser's session failure.

`one-command.spec.ts` (#1584) starts only `tmt-colab serve` (`startServe`): Colab starts the
real `tmt-remote` door through the real core's public CLI, a paired device opens the printed
link and a created page, and stopping Colab closes the door it started (`doorAnswers`).
`world.linkExtensions()` puts `tmt-remote` and `tmt-colab` on the world's PATH, which is how
the core resolves `tmt remote`. `page create --json` prints the door's full `link` and `paired` (#1614), and the link opens as the
paired device. A second case drives `tmt colab stop` (#1594): the started door closes, a second
stop is `not-running`, and the pairing stays listed. A third attaches to a door started outside
Colab: stop and exit leave that door running.

`mention-send.spec.ts` creates a page from a real agent pane and verifies its removable creator mention,
one comment with three distinct Asks, live recipient bytes, durable offline inbox delivery,
and reload without replay. `createPage` links the extensions onto the isolated world's PATH
so native creator provenance resolves through the real core.

`discussion.spec.ts` covers two paired writers and comment-origin Ask. Its module
contracts and focused cases are described in [discussion.md](discussion.md).

`ask.spec.ts` holds the Ask cases: direct send (the recipient's received text is the oracle for the exact bytes) and a second viewer, browser
reload, Remote restart after the core accepted, Remote restart before dispatch, Colab restart,
device revocation and two tabs of one browser staying live at once. They
drive direct Chat (`composeChat`, `sendChat`, `askEntry`, `askState`) and run against the built binaries.
Remote stores sessions and door cookies in memory. The mounted owner can replace a verified
ended Session after a restart without a reload.
The cases admit that in-place recovery or the page's explicit Reconnect action before checking
the original Ask outcome. They observe a fresh successful mounted device registration after
Remote dies, then require live status and no stopped preview; an old iframe is not recovery.
The restored ask is observed read-only under its original operation ID: accepted, or uncertain
with abandon recorded as `MAY_HAVE_BEEN_DELIVERED`, never a second dispatch. An in-flight send
stays "dispatching" until the SDK deadline, so a case reconnects instead of waiting for it.
The held case waits for a Remote-provided hold fixture, with held behavior covered by unit
tests. Both Remote restart cases are active pass-required cases. A fetch failure during
replacement leaves explicit Reconnect available. They retain their original-operation,
uncertain, recheck, abandon, no-effect, accepted, one-dispatch and one-wake assertions.
Explicit-Reconnect draft cases keep the same Chat and anchored composer at 1440/390,
including a mid-text caret through a failed click and a verified in-place replacement.
They count the original Ask and the next explicit Send separately, verify a new
registration without main-frame navigation, and exercise a plain-comment Send before an anchored Ask.
Enable a case by making its body pass, never with
a stand-in. Assert the recipient's text equals the disclosed bytes captured on Enter, including the
`[remote: <device>]` line, and that no delivery state is shown (presence only).

`withWorld` always disposes, and `test.afterEach(disposeActiveWorlds)` does too after a test
timeout, so a timed-out case leaves no tmux server, process or root behind.

`reader.spec.ts` (#1545) drives the read-only share link: the page, `share link add`/`reset` and `page write`
are real owner commands, and each reader is an unpaired Chromium profile (`openReaderLink`) that
never paired with the door. It asserts that an unpaired browser gets no owner file, the fragment
leaves the address bar, the page shows read-only and live, an owner edit reaches it, no request
carries the seed, Reset ends the open reader ("Access ended") and the old link, and the
replacement link opens in a third profile.

`export.spec.ts` (#1574) has two paired writers discuss a selection, one asks the recipient
agent about the comment, then exports through the browser's Export panel (real downloads) and
the CLI `tmt colab export`. It asserts the page, both conversation files and the manifest
(except `exportedAtMs`) are byte-identical between the two paths, and that the export holds both
writers' comments, the accepted Ask and the stored reply.

`tabs.spec.ts` pins the Remote contract the Ask design depends on: several
sessions of one device can coexist, while session eviction at the configured
limit ends only the evicted tab's transports and reports the active limit.

`idle-tab.spec.ts` (#2170) leaves one live tab idle for 150 s, past the door's 120 s tunnel
limit, and counts the page's `/sync` sockets: none may close or reopen, because the server pings
every 30 s. It takes about 2.5 minutes, so run it only for changes to the sync keepalive, the
tunnel limits or the live reconnect path.

`chat.spec.ts` (#1645) covers one null-anchor thread per asking device, two paired
viewers, page-visible history, Comments exclusion, exact follow-up context, retained
drafts, current-app-dir native CSP privacy and 1440/390 light/dark captures. Its
reload/restart checks count actual recipient rows and core dispatches, never terminal
echo. The restart suite can arm the next dispatch before Enter generates its ID,
then verifies that the parked operation matches the admitted Ask record.
