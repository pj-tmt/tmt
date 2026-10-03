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

## Cases

`ask.spec.ts` holds the seven Ask cases (direct send with exact bytes and a second viewer,
browser reload, Remote restart after and before the core acts, Colab restart, revocation,
held grant). Their bodies drive the real Ask UI (`harness/ask.ts`: `selectInRenderer`,
`previewAsk`, `send`, `askEntry`, `askState`). They are `test.fixme` for one reason: v1 has no
product way to create a page, so `createPage` fails visibly until `tmt colab page create`
exists; the suite never seeds a page with a test-only producer. The held case also needs a
hold grant for the device. Enable a case by removing `fixme`, never with a stand-in.
Assert the recipient's text equals the previewed text, including the `[remote: <device>]`
line, and that no delivery state is shown (presence only).

`tabs.spec.ts` pins a Remote contract the Ask design depends on: Remote keeps one session per
device, so a newer `session.open` ends the older session and its tunnels. Two tabs of one paired
browser are one device, so v1 allows one active tab with explicit takeover: a newer tab takes the
session, the older shows a notice and a "Use here" button and makes no Remote calls. The
two-tab Ask case in `ask.spec.ts` asserts that behavior.
