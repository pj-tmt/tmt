# Colab local page preview

A private React/Vite app for space home and HTML page views. The in-process
preview adapter supplies detached sample snapshots. The paired
mount uses Remote's session/key certification, Colab registration and verified
bootstrap, with a bounded Worker and live content binding. The general discussion
UI remains deferred; Ask operations use the verified Remote port owned by the
browser controller.

The executable serves an immutable app inventory at startup. Build with
`TMT_COLAB_APP_DIR=/absolute/path/to/dist` to embed the complete Vite output,
including its dependency notices; the resulting binary needs no app directory at
runtime. An invalid supplied build fails compilation. `serve --app-dir` overrides
embedded bytes; without either, source builds try the checkout's `dist`, then show
the build hint. Rebuilding embedded assets requires rebuilding the binary.
Vite emits relative asset URLs for the nested Remote mount. The app uses system
font fallbacks and makes no third-party asset requests. The static route, fallback
and CSP contract is owned by
[colab-v1](../../contracts/colab-v1.md#implemented-mounted-browser-assets-1253).

Trusted chrome uses the shared design tokens. Page HTML runs in an opaque frame
under the [renderer contract](../../contracts/colab-v1.md#renderer-and-live-anchors).
The frame loads the same-mount build-owned `renderer.html` with its own response
CSP and accepts bounded source/render metadata once from its parent. Its policy
also sandboxes direct opening. Trusted mounted chrome disallows inline scripts
and styles; the loopback Vite dev parent has the documented hot-reload exception.
A frame can navigate itself and
leak a request before teardown; the app does not promise complete exfiltration
prevention.

[DEVELOPMENT.md](../../../../DEVELOPMENT.md#colab-browser-verification) owns the
install, dev, build and test commands. `test:browser` requires a built native `tmt-colab`
and runs the Chromium isolation and real-socket static app scenarios, separately from the client primitive conformance harness.

The trusted page chrome captures bounded selection text from the current opaque
renderer. Frame messages carry text only; the parent binds them to the active
frame and render ID. Ask agent opens a parent-drawn picker and freezes the quote,
question and destination before the preview is sent. Closing a preview does not
cancel agent work. The page Ask panel displays admitted Ask state and attributed
replies separately from the deferred general discussion UI. Names are publisher
asserted display labels; routing UUIDs remain available in Details. Historical
asks retain the signing keys captured from authenticated, cut-admitted own
envelopes, including revoked writers, without granting action rights. The picker
shows presence; Remote supplies no delivery evidence in v1. The preview includes
Remote's verified device-name prefix, while the transport excludes it because
Remote adds it. Re-check and the visible-page observer read the original operation;
uncertain asks may be abandoned without retrying dispatch.

Registration retains the verified Remote session. Tunnel disconnects and resyncs
reopen sync using that same session and Remote adapter, rebuilding only the Ask
controller. An explicit Remote session fault triggers coalesced mounted session
replacement, verifies the same device and space owner, then rebuilds the
Remote adapter and Ask controller before opening sync. Closing the old controller
invalidates its preview actions; the new controller observes unresolved original
IDs after catchup. Normal context, agent-list, send and result calls never reopen
the session.

Browser tests may set `COLAB_APP_TEST_PORT` to isolate their loopback Vite server;
the default remains 4179. The deterministic Page/Remote doubles live only in
`test/ask-page-browser.tsx` and `test/ask-browser-attempt.ts` and never enter the
production build. Chromium tests
use stable `ask-*` test IDs and `data-operation-id` on each preview and Ask entry.
