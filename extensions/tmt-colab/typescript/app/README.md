# Colab local page preview

A private React/Vite app for space home and HTML page views. `PageTransport` is
read-only; the in-process adapter supplies detached sample snapshots. Remote
sign-in, live sync, editing, comments and agent operations are later slices.
The executable does not embed this build yet.

Trusted chrome uses the shared design tokens. Page HTML runs in an opaque frame
under the [renderer contract](../../contracts/colab-v1.md#renderer-and-live-anchors).
Only source and render metadata enter that frame. A frame can navigate itself and
leak a request before teardown; the app does not promise complete exfiltration
prevention.

[DEVELOPMENT.md](../../../../DEVELOPMENT.md#colab-browser-verification) owns the
install, dev, build and test commands. `test:browser` runs the app's Chromium
isolation tests, separately from the client primitive conformance harness.
