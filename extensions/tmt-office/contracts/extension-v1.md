# World extension records v1

Local implementation: bundled discussion-board, whiteboard, broadcaster, notebook and web-link bindings, not an external plugin
installer, general native command endpoint or published compatibility promise.
The [functional-extension design](functional-props.md) owns the larger target.

## Records

The literal [definition](discussion-extension-v1.json) and
[instance](lobby-extension-v1.json), and the whiteboard
[definition](whiteboard-extension-v1.json) / [instance](lobby-whiteboard-v1.json), and broadcaster
[definition](broadcaster-extension-v1.json) / [instance](lobby-broadcaster-v1.json)
are the local default composition. They are
separate from immutable artwork packs, editable furniture layouts and discussion
content. Reading them creates no database row. These instance records define
bootstrap placement and the existing preflight input, not a second mutable store.
The [whole-world layout](world-v1.md) now persists a stable placement UUID,
appearance/surface and `{definition,binding}` attachment in one revision. Its
bootstrap reuses these records; subsequent reads do not recreate removed entries.
The UI reads persisted placements; no static instance list is rendered beside them.

| Record              | Required fields                                                                                            |
| ------------------- | ---------------------------------------------------------------------------------------------------------- |
| Definition          | `formatVersion: 1`, `worldApiVersion: 1`, `id`, `label`, `appearance`, `action`                            |
| Appearance          | Immutable `prop` digest/key and unrotated `footprint: {width,height}`                                      |
| Action              | `id: "open"`, `label`, and one capability/resource pair below                                              |
| Instance            | `formatVersion: 1`, `id`, `definition`, `space: "commons"`, `x`, `y`, `rotation`, `binding`                |
| Discussion binding  | `kind: "office-board"`, optional `roomId` — General or a canonical non-nil room UUID in the existing board |
| Whiteboard binding  | `kind: "whiteboard", documentId` — `lobby` or a canonical supported UUID                                   |
| Broadcaster binding | `kind: "office-broadcast"` — the current installation's announcement composer                              |
| Web-link binding    | `kind: "external-link", url` — an inert HTTP(S) destination, reviewed before navigation                    |
| Notebook binding    | `kind: "notebook", identityId` — an active saved identity's existing owner-local Markdown                  |

The admitted action pairs are `discussion.open` / `office-board`,
`whiteboard.open` / `whiteboard`, `broadcast.open` / `office-broadcast`, and
`link.open` / `external-link`, and `notebook.open` / `notebook`.
Notebook identity references are canonical non-nil UUIDs; structural admission
does not grant saved eligibility. The [notebook reader](notebook-v1.md) revalidates
that eligibility on every open/refresh. Attachment and removal change only world
references, never note files. There is no default notebook placement.
Crossed pairs reject. Whiteboard IDs reuse the
[document contract](whiteboard-v1.md), not filesystem paths or URLs. A discussion
or broadcaster binding cannot carry a document ID. The default board and whiteboard occupy
`(1,3)` and `(14,3)`, both `12×8`; the broadcaster occupies `(4,17)`, `8×8`.
These placements leave existing room geometry unchanged.

An absent discussion `roomId` means General; null, display names and nil UUIDs
reject. Room scope is an explicit resource reference, never inferred from object
position. Moving an object or rebinding/removing its area preserves the reference.
Opening another scope uses the retained board's leave gate: unsaved drafts require
confirmation; unresolved writes must be retried or explicitly discarded first.
Closing the panel preserves drafts. Neither opening nor placement creates content.

IDs use the existing Office key grammar `[a-z][a-z0-9-]{0,31}`. Labels are
non-control UTF-8 text, 1–80 bytes. Appearance and combined placement reuse the
existing local prop-placement constraints: footprint sides 1–16, integer tile
coordinates 0–31, direction 0–3, rotated footprint within the 32-tile grid.
These bounds apply to the bootstrap/preflight instance envelope. Persisted world
objects instead use the signed floor/wall coordinates in [world v1](world-v1.md).

### Web destinations

The [link definition](link-extension-v1.json) is bundled but is not automatically
placed in the Lobby. Any ordinary admitted artwork may carry its attachment.
The editor can attach/update/remove a link in the whole-world draft; it does not
silently replace an existing board/whiteboard/broadcast resource binding.

URLs are at most 2,048 UTF-8 bytes, absolute HTTP(S), with a nonempty host and no
credentials (including empty userinfo), raw whitespace/control characters,
backslashes or directional-override/isolate characters. Query and fragment are
allowed. Input bytes are retained; the review displays the parsed canonical
origin and full URL. [Shared vectors](external-link-vectors.json) cover native
and browser admission. `tmt-office-model::office_extension` owns native validity using
the existing workspace URL parser; adapters do not fetch a destination.

`link.open` opens only the host-owned destination review. A separate explicit
user click on its anchor opens HTTP(S) in a new tab with `noopener noreferrer`.
Rendering, saving, importing, deleting and merely opening the review never fetch
or navigate. Invalid bindings and unavailable handlers remain inert. No URL
becomes a command, trusted prompt, iframe, image source or permission grant.

Unknown fields, versions, World API versions, actions, capabilities and binding
kinds reject. Null is not absence. Native decoders consume raw JSON, reject
duplicate fields and use the existing 64 KiB local document input ceiling;
this is metadata, not a limit on the bound resource's contents. Browser admission
checks already-parsed objects. [Shared vectors](extension-vectors.json) exercise
both decoders; native raw-input tests additionally cover duplicate keys and bytes.

## Authoring preflight

With a compatible installed CLI/Office pair:

```sh
tmt office extension validate --file definition.json --instance instance.json --json
```

Both inputs must be regular UTF-8 files, at most 64 KiB each; symlinks and special
files reject. The verified companion owns decoding and paired validation. Raw
documents cross the bounded protocol without discarding duplicate keys.
Checks include matching definition identity, capability/resource compatibility
and the combined rotated footprint. Native and browser composition share
[pair vectors](extension-pair-vectors.json).

Success exits 0 and returns
`{"status":"valid","definition":"board","instance":"lobby-board","scope":"structureOnly"}`.
Invalid structure exits 1 with `OFFICE_EXTENSION_INVALID` and a diagnostic;
file acquisition failures use `OFFICE_EXTENSION_INPUT_INVALID`. Missing companion
uses the standard `OFFICE_NOT_INSTALLED` flow. Neither tmux, an identity, a browser
session nor a running Office service is required.

This is not installation, aesthetic lint or runtime authorization. A structurally
valid artwork reference may still be absent from the installed catalog. Runtime
binding must still resolve the exact artwork/footprint and a registered host
handler. Preflight writes no catalog, layout, content or request and executes no
extension code. Use prop/avatar validation and visual preview for artwork;
there is no general extension installer or arbitrary host command capability.

## Binding and dispatch

`extension-binding.ts` resolves a definition, checks the combined placement and
admitted appearance, then looks up the exact capability in host-owned handlers.
World objects use their stored appearance and placement UUID through
`bindWorldExtension`; the bundled bootstrap uses `bindExtension`. Both call the
same binding/handler composition and activation owner. The production canvas and
accessible object controls use world placement IDs.
The local composition root registers `discussion.open`, `whiteboard.open`, `broadcast.open`, `notebook.open` and `link.open`,
opening the existing resource views. Definitions declare a requirement; importing or rendering one never
registers a handler, authorizes a request or executes a command.

Unknown definitions, incompatible/missing artwork or unavailable handlers remain
inert with a visible reason. Keep the original instance and resource binding so
restoring a dependency can restore the presentation. Removing or relocating an
instance is not a content operation. Activation only mounts/opens the resource
view; it never dispatches work or writes content. Resource actions
use their existing authenticated typed APIs and revision/receipt rules.
The broadcaster uses the [dispatch contract](dispatch-v1.md) with `kind: "announcement"`;
opening it never selects recipients or sends automatically.

The renderer consumes world-projected bounds, display label and availability,
and emits an instance ID. Canvas and accessible controls use the same guarded
dispatcher and modal/draft owner. `useExtensionPanel` owns the shared native dialog
lifecycle: first open mounts content, close keeps the visited draft, and the browser
restores trigger focus. Decorations and functional instances share
prop resolution, directional frames and scene texture ownership. No capability
name, shell text or board data enters the pixel renderer.

Interaction plaques retain a readable minimum screen size at fitted zoom. Static
teal corner brackets and an action badge contrast with the warm pixel furniture,
making available actions apparent before hover. Pure decorations have no action
cue; unavailable objects have a distinct marker and explicit HUD explanation.
Pointer hover and keyboard focus strengthen the same cue and reveal its action
label. Geometry used for drawing also determines activation targets. Drag and
pointer cancellation do not activate. No idle animation loop is introduced.
