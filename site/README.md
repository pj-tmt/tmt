## Colab landing

The English `/colab/` route is a landing design exploration within the site,
separate from Home and the retained handbook chapters. Four large feature
close-ups show annotation, agent conversation, live documents and work
organization, followed by a short installation guide. The UI close-ups are
illustrations. The route reuses the site theme and logo; the development embed
below annotates it directly.

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
Missing anchors remain in the annotation list. Open one and use Reattach this
discussion, then click a component or select text to move its anchor without
changing its turns or draft. The Connection view lists local agents and opens
the existing Remote entry for user-controlled device pairing.

Repeated activation and HTTP retries share one bounded server-session send
operation, retained before dispatch across browser reload/HMR. A definite
pre-dispatch error releases the hold. An uncertain failure never redispatches
that operation within the same server session: the composer shows the frozen
message and an explicit clear action. Check TMT before clearing the hold or
restarting the preview, because the request may have reached the agent. Editing
an unacknowledged send cannot allocate a replacement operation until it is
cleared. A temporary reply lookup failure remains visible and does not stop
subsequent automatic checks.

Explicit Ask actions use the configured sender and installed TMT CLI through a
loopback-only, same-origin Vite middleware. Open discussions automatically check
for replies every three seconds. Reply lookup uses the retained TMT exchange
owned by the configured sender, so old threads can read replies after a preview
restart while TMT retains that exchange. No pairing or grants are performed by
the overlay. The development middleware and FAB are excluded from production
builds, even when the embed flag is set.
