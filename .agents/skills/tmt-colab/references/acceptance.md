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

`ask.spec.ts` holds the Ask cases: direct send with exact bytes and a second viewer, browser
reload, Remote restart after the core accepted, Remote restart before dispatch, Colab restart,
device revocation and two tabs of one browser (one active tab, "Use here" takes it back). They
drive the real Ask UI (`selectInRenderer`, `previewAsk`, `send`, `askEntry`, `askState`) and run against the built binaries. A restarted Remote keeps sessions and door cookies in memory, so the page
shows "Sync disconnected" and the restart cases recover through its own Reconnect button
(`reconnect(page)`: the SDK reopens the paired session once, then the page reloads). The
restored ask is observed read-only under its original operation ID: accepted, or uncertain
with abandon recorded as `MAY_HAVE_BEEN_DELIVERED`, never a second dispatch. An in-flight send
stays "dispatching" until the SDK deadline, so a case reconnects instead of waiting for it.
The held case waits for a Remote-provided hold fixture, with held behavior covered by unit
tests. Enable a case by making its body pass, never with
a stand-in. Assert the recipient's text equals the previewed text, including the
`[remote: <device>]` line, and that no delivery state is shown (presence only).

`withWorld` always disposes, and `test.afterEach(disposeActiveWorlds)` does too after a test
timeout, so a timed-out case leaves no tmux server, process or root behind.

`reader.spec.ts` (#1545) drives the read-only share link: the page, `share link add`/`reset` and `page write`
are real owner commands, and each reader is an unpaired Chromium profile (`openReaderLink`) that
never paired with the door. It asserts that an unpaired browser gets no owner file, the fragment
leaves the address bar, the page shows read-only and live, an owner edit reaches it, no request
carries the seed, Reset ends the open reader ("Access ended") and the old link, and the
replacement link opens in a third profile.

`tabs.spec.ts` pins a Remote contract the Ask design depends on: Remote keeps one session per
device, so a newer `session.open` ends the older session and its tunnels. Two tabs of one paired
browser are one device, so v1 allows one active tab with explicit takeover.
