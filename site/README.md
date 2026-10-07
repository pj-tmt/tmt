## Local Colab embed preview

The opt-in development overlay annotates the live handbook DOM without copying it
into a Colab HTML page. Start from an isolated checkout:

```sh
VITE_COLAB_EMBED=1 TMT_EMBED_IDENTITY=<your-identity-uuid> pnpm dev --port 5185 --strictPort
```

Use the pointer icon or C to toggle Colab mode without opening a menu. The
shortcut is inactive while editing text, composing with an IME, or holding any
modifier key. The separate details icon opens Connection & threads, even when
Colab mode is off. Hover outlines a component; click it or select
text to open an anchored discussion. Clicking outside collapses it, and reopening
restores its draft. The composer stays at the bottom while messages scroll; new
turns and agent replies scroll the message area to the latest entry. Enter sends to the selected agent; Shift+Enter adds a line.
Comment stores a local-only turn. Resolve closes the bubble and hides its marker;
the discussion remains in the list for reopening. Comments and drafts stay in this browser's local storage
across reload/HMR. They are local review data, not encrypted/shared Colab records.
Missing anchors remain in the annotation list. The Connection view lists local
agents and opens the existing Remote entry for user-controlled device pairing.

Repeated activation and HTTP retries share one bounded server-session send
operation, retained before dispatch across browser reload/HMR. An uncertain failure
never redispatches that operation within the same server session. A temporary reply
lookup failure remains visible and does not stop subsequent automatic checks.
Editing an unacknowledged send cannot allocate a replacement operation; restore
its original draft and recipient to retry. Inspect TMT in the terminal before
restarting a preview with an unacknowledged operation.

Explicit Ask actions use the configured sender and installed TMT CLI through a
loopback-only, same-origin Vite middleware. Open discussions automatically check for replies every three seconds;
request receipts belong to the current server session. Restarting the server
requires checking old receipts with `tmt result` in the terminal. No pairing or
grants are performed by the overlay. The development middleware and FAB are
excluded from production builds, even when the embed flag is set.
