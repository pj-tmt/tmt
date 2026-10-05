# Ask agent real-binary acceptance (#1110)

`extensions/tmt-colab/typescript/app/acceptance/` drives the built `tmt`, `tmt-remote` and
`tmt-colab` with real Chromium devices. It needs no Docker and adds no production seam.

## Run

Build the binaries and the app, point `TMT_ACCEPTANCE_BIN_DIR` at the directory holding the
three binaries (default `rust/target/debug`), then run from `typescript/`:

```bash
(cd rust && CARGO_BUILD_JOBS=2 cargo build --locked -p tmt-cli -p tmt-remote -p tmt-colab --bins)
corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match build
corepack pnpm@10.33.0 --filter @tmt/colab-app --fail-if-no-match test:acceptance
```

Run it twice for lifecycle acceptance; it is a recorded manual gate on the PR head, not a CI
job. Set `TMT_ACCEPTANCE_KEEP=1` to keep a world's root (counter rows, `*.stderr`) after a run.

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

`one-command.spec.ts` (#1584) starts only `tmt-colab serve` (`startServe`): Colab starts the
real `tmt-remote` door through the real core's public CLI, a paired device opens the printed
link and a created page, and stopping Colab closes the door it started (`doorAnswers`).
`world.linkExtensions()` puts `tmt-remote` and `tmt-colab` on the world's PATH, which is how
the core resolves `tmt remote`. `page create --json` prints the door's full `link` and `paired` (#1614), and the link opens as the
paired device. A second case drives `tmt colab stop` (#1594): the started door closes, a second
stop is `not-running`, and the pairing stays listed. A third attaches to a door started outside
Colab: stop and exit leave that door running.

`discussion.spec.ts` covers two paired writers and comment-origin Ask. Its module
contracts and focused cases are described in [discussion.md](discussion.md).

`ask.spec.ts` holds the Ask cases: direct send (the recipient's received text is the oracle for the exact bytes) and a second viewer, browser
reload, Remote restart after the core accepted, Remote restart before dispatch, Colab restart,
device revocation and two tabs of one browser staying live at once. They
drive direct Chat (`composeChat`, `sendChat`, `askEntry`, `askState`) and run against the built binaries. A restarted Remote keeps sessions and door cookies in memory, so the page
shows "Sync disconnected" and the restart cases recover through its own Reconnect button
(`reconnect(page)`: the SDK reopens the paired session once, then the page reloads). The
restored ask is observed read-only under its original operation ID: accepted, or uncertain
with abandon recorded as `MAY_HAVE_BEEN_DELIVERED`, never a second dispatch. An in-flight send
stays "dispatching" until the SDK deadline, so a case reconnects instead of waiting for it.
The held case waits for a Remote-provided hold fixture, with held behavior covered by unit
tests. Enable a case by making its body pass, never with
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

`chat.spec.ts` (#1645) covers one null-anchor thread per asking device, two paired
viewers, page-visible history, Comments exclusion, exact follow-up context, retained
drafts, current-app-dir native CSP privacy and 1440/390 light/dark captures. Its
reload/restart checks count actual recipient rows and core dispatches, never terminal
echo. The restart suite can arm the next dispatch before Enter generates its ID,
then verifies that the parked operation matches the admitted Ask record.
