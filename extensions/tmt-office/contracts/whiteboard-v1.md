# Whiteboard v1

Local work in progress, not a released feature. Scene admission, SQLite documents,
their local HTTP/browser port and an editor with spatial/direct entry are implemented.
Structured immutable snapshot/PNG storage, native HTTP/CLI access, image export
and local references are implemented locally, including explicit-identity request
composition and revision-fenced meeting-room selection.
See [functional extensions](functional-props.md)
for the complete behavior and [extension records](extension-v1.md) for placement.

## Document

`{formatVersion: 1, width: 1600, height: 1000, background, elements: [...]}`
describes one finite drawing surface. Coordinates and sizes are integer document
pixels, independent of browser zoom. The background and colors are lowercase
six-digit hex colors. Elements are ordered back to front and have unique canonical
UUIDs. Unknown fields, versions and kinds reject; missing values and null are not
defaults. Empty scenes are valid.

| Kind                   | Fields in addition to `id` and `kind`                            |
| ---------------------- | ---------------------------------------------------------------- |
| `stroke`               | `points: [[x,y], ...]`, `color`, `strokeWidth`                   |
| `arrow`                | Exactly two `points`, `color`, `strokeWidth`                     |
| `rectangle`, `ellipse` | `x`, `y`, `width`, `height`, `color`, `fill`, `strokeWidth`      |
| `text`, `note`         | `x`, `y`, `width`, `height`, `text`, `color`, `fill`, `fontSize` |

Points and boxes stay inside the surface; boxes have positive dimensions.
Stroke widths are 1–16 pixels; font sizes are 12–72 pixels. `fill` is a color or
`none`. A stroke has at least two points. Text allows line feeds and tabs, but not
other control characters or unpaired surrogates. Empty text is valid while editing.
Text is always inert: literal markup, links and instructions never become HTML,
remote resources or host operations. There are no embedded files, fonts, SVG or URLs.

Admission budgets are 2 MiB of UTF-8 JSON, 2,048 elements, 65,536 total points,
16 KiB per text element and 64 KiB total text. These are bounded document-work
budgets, not per-message limits. The browser and native boundary share the same
literal contract vectors. Raw native JSON additionally rejects duplicate fields.
Host route body limits must admit the complete document envelope before enabling
save; do not route a valid scene through the existing 64 KiB metadata ceiling.

## Editing and resource ownership

The editor owns selection, tool state and undo/redo, not storage or delivery.
A pointer gesture becomes one transaction on completion; cancellation makes no
change. Undo/redo restores element IDs and paint order. No-op transactions do not
add history. New edits after undo discard redo. Selection and camera are not saved
document content. Rendering and image export consume this same admitted scene.
Undo retains up to 64 scene entries within a 16 MiB serialized-history budget,
evicting the oldest entries first; the live document is never evicted. This bounds
retained document work, not exact JavaScript heap usage.

Physical whiteboards and their accessible entries share one lazy-mounted resource
session. Closing and reopening the same document keeps its draft and returns focus.
Switching documents requires explicit discard of unsaved drawing, inspector text
or snapshot/review composition. Pending or unconfirmed saves, captures and sends
must be resolved in the existing session before switching; a target change cannot
silently abandon their retry intent. Opening does not create a resource or dispatch
a request. The local preview route
`/local/whiteboards/<id>` hosts the same editor against the same saved resource,
not a shared in-memory draft across routes. Both require an authorized local Office
browser session. Direct-route navigation uses the same aggregate leave state:
draft departure requires confirmation and unresolved operations block departure;
browser unload receives the standard unsaved-work warning. Explicitly restarting
an unconfirmed snapshot requires a separate discard-retry confirmation and never
claims to delete a snapshot that may already have been saved.
It supports pen, arrows, rectangle/ellipse shapes, notes/text, selection,
movement, deletion and explicit Save. The element list exposes selection and text
editing outside the canvas. Review snapshot switches to a preview workspace without
discarding the drawing draft. A stored snapshot with an admitted PNG enables Ask agents.
Unconfirmed saves keep the original operation intent and freeze document edits
until that save is retried successfully or the user explicitly discards and reloads.
A failed reload leaves the draft intact. Disposal aborts pending transport and
prevents late results from updating the closed editor.

The 2D painter draws admitted scenes, not HTML or SVG. Display rasters follow the
viewport with device-pixel ratio capped at 2, while document coordinates remain
fixed. Live pen input owns a mutable gesture buffer and paints only new segments;
it does not repeatedly copy or repaint the entire growing stroke. Completion
passes one scene through admission/history. Resizing redraws the current buffer
once, and cancellation restores the unchanged scene. There is no idle render loop.

The local resource service owns stable document IDs and monotonically increasing
revisions. Saves carry an operation ID and expected revision; retries cannot
duplicate writes and conflicts preserve the draft. Agent edits are proposals for
human acceptance, never unconditional replacement of an open human draft.
Moving or deleting an extension instance does not remove a document.

### Local resource API

`GET /api/v1/local/whiteboards/<id>` returns
`{id,revision,scene,updatedAtMs}`. IDs are `lobby` or canonical UUIDs. Any admitted
absent document reads as a blank cream (`#fff7e7`) revision-0 document with timestamp
0 and the requested ID, without creating a world or document. This same lazy
resource model serves the Lobby and meeting whiteboards. Snapshot capture still
requires a saved document; invalid IDs do not acquire a default.

`PUT` at the same path accepts `{expectedRevision,operationId,scene}` and returns
`{documentId,operationId,revision,changed,updatedAtMs}`. The browser bearer is
required for both operations; writes also require the exact local Origin and
JSON content type. The request cannot choose an author or execute an agent action.
Its body budget is the 2 MiB scene plus 16 KiB of envelope space; the scene keeps
its independent admission budget. Unknown and duplicate JSON fields reject before
storage, including duplicates inside the nested scene.

Revision 0 creates the first saved document at revision 1. Existing documents
require the exact expected revision, even when the proposed scene matches.
An unchanged scene keeps its revision and timestamp. Changed scenes increment
the revision; revisions and timestamps remain JavaScript-safe integers.

Operation IDs are installation-world scoped. Document, canonical scene and expected
revision form the saved intent. Retrying that intent with the same operation ID
returns its original receipt, even after newer edits; reusing the ID for another
intent conflicts. A new operation with a stale expected revision also conflicts.
World creation, conditional document write and operation receipt commit in one
SQLite transaction. A failed receipt insert rolls the document back as well.
The browser port does not automatically retry, reload or acknowledge conflicts.

Errors are `WHITEBOARD_INVALID` (400), `WHITEBOARD_NOT_FOUND` (404),
`WHITEBOARD_REVISION_CONFLICT`, `WHITEBOARD_REVISION_EXHAUSTED`,
`WHITEBOARD_IDEMPOTENCY_CONFLICT` (409), or `STORAGE_UNAVAILABLE` (500), in
addition to the shared HTTP authentication, Origin and routing errors.

## References and explicit sending

### Structured snapshot storage

Native capture policy and SQLite storage are implemented. The capture envelope is
`{expectedRevision, operationId, selectedElementIds, annotation}` for a document ID.
It requires an existing saved revision (at least 1). The host copies that exact
current revision under one transaction; callers cannot submit scene replacement,
images, an actor or a command. Stale revisions fail, never falling back to current
content or implicitly saving a browser draft.

Selection IDs must be unique supported UUIDs in the captured scene. Empty means
the whole scene; otherwise selection is a set, stored in canonical sorted order.
The full source scene remains available for context. Annotation uses the scene's
inert 16 KiB text policy. The 128 KiB input ceiling accommodates all 2,048 selected
IDs and annotation without forcing them through a 64 KiB metadata limit.

The operation UUID is also the immutable snapshot ID. An exact retry returns the
original capture even after newer edits and restart; changed document, revision,
selection or annotation under that ID conflicts. Reordering the same selected IDs
does not change intent. The snapshot stores document ID/revision, scene, selection,
annotation and creation time; it never updates the source or dispatches a request.
There is no snapshot update or cleanup API. Removing live document content must not
erase captured content. Missing snapshots fail explicitly.

### Snapshot image storage

The native image adapter and SQLite attachment are implemented locally. An image
is a static, opaque 1600×1000 8-bit RGB/RGBA PNG, with at most 8 MiB of encoded
input. The host bounds decoder allocation, checks dimensions before allocating
pixel storage, validates the complete image and rejects animation and transparent
pixels. It re-encodes decoded pixels without text, ICC or other source metadata.
The pure-Rust `png` dependency avoids requiring a browser, Node or system image
tools on native installations.

An existing immutable capture can receive one normalized image. The image row
references that capture, not its mutable source document. Its SHA-256 identity
covers opaque RGBA pixels, so RGB/RGBA or compression differences do not make an
equivalent retry conflict. Equal pixels return the original retained PNG bytes;
different pixels conflict without overwriting. Reads validate pixels and their
digest without re-encoding. A failed image write leaves the structured capture
available for retry. There is no implicit render, latest-scene fallback, replace
or cleanup operation.

The browser renders the already captured scene using the existing
whiteboard painter and shows the actual stored image. PNG admission
proves format and pixel integrity, not that an authorized uploader drew the scene
faithfully. The retained structured scene, selection and annotation remain the
authoritative drawing data. Image upload belongs to its own typed boundary, never
the capture metadata envelope.

### Snapshot HTTP access

All routes require the existing local browser bearer. Writes additionally require
the exact local Origin and the declared content type. No route accepts an agent
actor, arbitrary filesystem path, remote URL or command.

| Method and path                                             | Input                               | Result                         |
| ----------------------------------------------------------- | ----------------------------------- | ------------------------------ |
| `POST /api/v1/local/whiteboards/<documentId>/snapshots`     | Capture JSON above, at most 128 KiB | Retained snapshot JSON         |
| `GET /api/v1/local/whiteboard-snapshots/<snapshotId>`       | None                                | Retained snapshot JSON         |
| `PUT /api/v1/local/whiteboard-snapshots/<snapshotId>/image` | Raw `image/png`, at most 8 MiB      | Original stored normalized PNG |
| `GET /api/v1/local/whiteboard-snapshots/<snapshotId>/image` | None                                | Original stored normalized PNG |

Snapshot JSON is `{id, documentId, documentRevision, scene, selectedElementIds,
annotation, createdAtMs}`. The capture UUID identifies both JSON and image. A
missing image returns `WHITEBOARD_NOT_FOUND` even when the structured capture
exists; it never creates an image. Attach returns the actual retained image so
the browser can preview what subsequent reads/export will receive. All successful
operations return HTTP 200; errors reuse the document codes above. Responses
retain the local service's no-store and nosniff policy. Resource-specific body
budgets apply only to the exact method/path, not adjacent routes or other APIs.

### Browser snapshot review

Review snapshot keeps the editor mounted and exposes a separate annotation and
multi-element highlight selection. An empty selection means the whole board;
selected elements receive the same teal dashed outline used in the drawing editor.
The complete surrounding scene stays visible. Unsaved or unconfirmed document
changes must be saved before capture; preview never implicitly saves them.

`snapshot-state` retains one capture intent through uncertain completion. Once
capture succeeds, `snapshot-image` renders that returned scene to an opaque sRGB
1600×1000 PNG with the shared painter. Upload failure retains the exact PNG for
retry, without recapturing or rerendering. Preview displays the host's returned
normalized image, not the producer's input. Annotation stays inert adjacent text.
Fit board shows the full retained image; Actual size exposes its original pixels
inside a touch- and keyboard-scrollable viewport without modifying or recapturing it.
Later live edits do not change the preview. Start new preview explicitly releases
its browser image URL and resets the form's capture operation, without deleting
the retained resource. Closing the spatial panel preserves the visited preview;
editor disposal aborts transport, fences late results and releases the image URL.

The local JSON and binary ports share authentication, cancellation and the same
five-second transport deadline through response-body consumption. Binary reads
are bounded before materializing the image, independent of Content-Length.
Browser PNG decoding is presentation; authoritative pixel admission stays native.

### Local snapshot access

CLI snapshot access, image export and copy reference address the same retained
resource. Explicit request composition below sends its reference through the local
inbox. Exporting alone does not prove an agent's image tool actually inspected the
result.

The local reference format is `tmt:whiteboard:snapshot:<uuid>` using the canonical
lowercase snapshot UUID. It identifies the retained resource in the recipient's
current TMT data directory, not a network location or an access credential. No
query, fragment, path, encoded ID or latest-revision alias is accepted. A different
machine without that resource reports it missing; remote transfer is not implied.

Native access is `tmt office whiteboard snapshot show <id-or-reference>`
for structured JSON, and `tmt office whiteboard snapshot export <id-or-reference>
--output <path>` for the stored PNG. Both call the verified Office companion and
the existing snapshot repository without starting the web service. Only the CLI
receives the explicit output path. Export must not overwrite an existing file or
leave a partial file after failure. The companion returns bounded bytes, never
filesystem instructions; PNG limits remain operation-specific, not global IPC limits.
Browser Copy reference appears after the image is stored and shows the same local
reference in a read-only field. Denied clipboard access selects it for manual copy.
Copying itself dispatches nothing.

The internal companion operations `whiteboard-snapshot-show` and
`whiteboard-snapshot-image` accept only `{ "snapshotId": "<uuid>" }` within 4096
bytes. Show returns the strict retained snapshot envelope within the scene's 2 MiB
budget plus 128 KiB metadata and 1024 bytes framing. Image returns raw PNG within
8 MiB. Errors are exact single-field JSON with `WHITEBOARD_INVALID`,
`WHITEBOARD_NOT_FOUND` or `STORAGE_UNAVAILABLE`; unknown shapes fail closed.
The parent verifies scene target/structure or PNG pixels without re-encoding.
Export stages a private file beside the requested destination and publishes with
no-clobber linking. It never sends paths to the companion or starts the web service.

### Explicit request composition

A snapshot records an immutable saved revision, the scene, selected element IDs,
annotation and a derived image. Selected IDs must exist in that captured revision.
Changing the live scene cannot change a snapshot. Copy reference has no delivery
effect and contains no browser token. Missing references fail explicitly rather
than silently loading the latest revision. Retention must preserve live request
references; do not add independent cleanup that strands an active request.

Ask agents loads the existing identity directory, including offline agents. Select
identities and enter a question, then Review request shows the exact audience and
message (question, immutable reference, native read/export instructions and a
text-only fallback that requires disclosing when the image was not inspected). Only
Send request invokes the [dispatch capability](dispatch-v1.md). Display names do
not route work: the preview freezes identity UUIDs and labels, and never expands
to new identities or substitutes a same-name replacement.

`snapshot-send-state` owns composition and the operation ID, separately from
snapshot capture/image retries. Before sending, Edit request returns to the draft.
After any unconfirmed send, editing is locked and Retry send reuses the exact
operation. No automatic retry occurs. Discard requires confirmation and explicitly
does not cancel potentially queued work. Starting a new snapshot is disabled while
an unsent/unconfirmed request exists. Closing/reopening the spatial panel or
returning to drawing retains the mounted draft; browser unload warns before loss.
Drafts are not persisted across navigation or page reload.

The receipt shows per-identity queued/unavailable acceptance and canonical request
IDs, not read/completed status. Receivers use the normal inbox inspection/reply
flow; the owner can read `tmt result <request-id>`. Snapshot access uses native
commands as well as the browser; text-only agents must distinguish structured
inspection from actually viewing the exported PNG.

Meeting room mode uses the [explicit stored roster](meeting-room-v1.md). Use this
roster freezes its revision and member UUIDs; Review request shows its name,
version and exact audience. A changed-room rejection preserves the question and
requires explicit refresh/adoption/re-preview. Directory or room refresh cannot
expand an already reviewed audience.
