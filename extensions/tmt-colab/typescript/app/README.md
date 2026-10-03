# Colab local page preview

A private React/Vite app for space home and HTML page views. The in-process
preview adapter supplies detached sample snapshots. The paired
mount uses Remote's session/key certification, Colab registration and verified
bootstrap, with a bounded Worker and live content binding. Comments and agent
operations remain later slices.

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
Only source and render metadata enter that frame. A frame can navigate itself and
leak a request before teardown; the app does not promise complete exfiltration
prevention.

[DEVELOPMENT.md](../../../../DEVELOPMENT.md#colab-browser-verification) owns the
install, dev, build and test commands. `test:browser` requires a built native `tmt-colab`
and runs the Chromium isolation and real-socket static app scenarios, separately from the client primitive conformance harness.
