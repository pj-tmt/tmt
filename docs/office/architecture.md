# Office architecture

Current browser and data ownership is defined here. [Design](design.md),
[planned commands](commands.md) and [contracts](../../extensions/tmt-office/contracts/README.md)
separate proposed capabilities from implemented behavior.

## Current implementation

Office is an optional React SPA under `extensions/tmt-office/typescript/apps/office`. Its local shell has a home
route, setup explanation, unknown-route recovery and provider-local presentation
state. Default preview does not initialize Firebase. Explicit `emulator` mode
on loopback enables local Google-provider popup sign-in, UID display and logout
through the Auth Emulator, not real Google. Explicit `cloud` mode requires
owner-local Firebase web configuration and uses the same Google/session adapter.
This Firebase preview/cloud shell does not load local identities, install an
extension, open a connector listener or dispatch work. The native local path is
separate below; login alone does not grant world access. A
Console-managed tester gate controls direct client create/read of owner-only worlds;
Firestore Rules enforce both gates, immutable fields and default-deny paths.
This is not an invitation, presence or connected-agent implementation.

Rules also implement the [agent grant boundary](../../extensions/tmt-office/contracts/agent-grant-v1.md):
trusted per-agent principals can access only an assigned UUID block with matching
installation/identity claims, live capability/lease and current owner admission.
Direct client grant writes are owner-only revocation; clients cannot issue or
enlarge one. The trusted issuer also accepts scoped agent/proof retirement
cancellation through its [pairing contract](../../extensions/tmt-office/contracts/pairing-v1.md#retirement-cancellation). This
supports explicit retained-block reassignment during owner approval. Native
credential consumption is owned
by the optional companion described below. Retained UUID blocks
use the existing layout validator; no parallel layout/ownership document is introduced.

The browser owner edits `home` or a UUID block selected from its grant inventory.
`src/blocks/block-contract.ts` owns client values, catalog and footprint policy;
`firebase-blocks.ts` implements its port using the existing initialized SDK.
`block-state.ts` owns the server-confirmed projection and separate unsaved draft.
Watch observations and save confirmations share one nondecreasing revision
projection. A failed watch is terminal until the block is reopened; late
callbacks cannot restore its content. Repeated unchanged admission leaves the
active world subscription and editor draft intact.
It starts in the mounted `BlockPanel` effect, is keyed by world, and disposes on
route/admission loss. Late observations and saves cannot restore disposed data.
`block-scene.tsx` renders remote block views; `block-view.tsx` owns
selection and controls and exposes a controlled scene slot. Local views use the PixiJS scene described below,
without a generic game framework, new global store or remote-state copy. Pointer selection/tile placement and
equivalent numeric/keyboard controls edit locally; explicit Save uses the
revision-checked Firestore transaction. The native companion implements
revision-safe block show/apply for its scoped remote assignment, while local
one-shot commands and the loopback service use the same SQLite repository for
installation-owned blocks. The contract and shared validation vectors live in
`extensions/tmt-office/contracts`.
Save confirmation and conflict inspection use one-shot transaction reads in the
same adapter, independent of watch-channel recovery. Watch snapshots remain
the ongoing projection, not an acknowledgment channel for explicit saves.
The pure contract's sole codec converts readable furniture maps to four-character
storage tokens, allowing Rules to validate all 16 objects and exact rotated
bounds within their expression budget.

`src/auth/firebase-session.ts` is the only Firebase initialization/composition owner.
`firebase-config.ts` validates explicit activation before SDK initialization.
`src/worlds/firebase-worlds.ts` owns direct SDK operations; `world-state.ts`
depends on the Firebase-free `world-contract.ts` port and value types, not the
concrete adapter. The contract owns shared client validation and the adapter
implements it; the block adapter and world form reuse its world-ID contract.
Rules retain independent validation as the authority boundary, not a generated
client-only guard. There is no second domain model or request layer. `world-state.ts`
owns one admission listener and at most one selected-world listener. Session and
admission generations fence late creates; route generations also fence world
listener callbacks. Creation success navigation is local to the mounted create
form and rendered through the router; unmounted forms cannot redirect a later
view. Leaving does not cancel the write or discard its session-owned retry draft.
React subscribes
directly, with no parallel Jotai/Query copy. Firestore cache-only snapshots never
grant access or display world data; only server-confirmed observations do.
The document contract is in `extensions/tmt-office/contracts/private-world.md`; Rules are the
untrusted-write enforcement boundary. The client name check is feedback only.
The entry point creates one uniquely named app outside React rendering and
disposes it on hot replacement. `session.ts` owns one observer, bounded public
identity projection, action serialization, sanitized errors and teardown.
`session-view.tsx` subscribes directly with React's external-store interface;
Jotai and Query do not mirror identity. Only the observer changes identity;
failed actions retain the observed state, and late completions cannot revive
a disposed session. Auth uses explicit in-memory persistence from initialization:
reload signs out and other tabs/contexts do not inherit the identity. No tokens
are exposed in view snapshots. This memory policy is not a substitute for Rules
authorization. Removing tester admission clears private UI and denies subsequent
server operations; previously disclosed content cannot be recalled. No production
Firebase setup is implied.

| Owner                                              | Responsibility                                                 | Forbidden dependency                                                    |
| -------------------------------------------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `extensions/tmt-office/typescript/apps/office`     | Browser routes, accessible views, UI state, app tests          | Local SQLite, filesystem/process APIs, Rust source or test helpers      |
| `extensions/tmt-office/typescript/services/office` | Emulator bootstrap, Rules and trusted scoped pairing issuance  | Local execution, browser imports or implicit agent authority            |
| `extensions/tmt-office/contracts`                  | Versioned design schema and structural conformance fixtures    | Browser rendering, Firebase effects or duplicate domain policy          |
| `rust/`                                            | Existing local CLI, domain and concrete adapters               | Office assets, Node or a Firebase account required by ordinary commands |
| `extensions/tmt-office/rust/tmt-office`            | Companion executable, embedded SPA and local HTTP service      | A second implementation of core identity, messaging or storage policy   |
| `extensions/tmt-office/rust/tmt-office-storage`    | Office database, schema and resumable migration of Office rows | Core storage internals, copied identity/room authority or core writes   |
| `docs/office`                                      | Definitions, scenarios and operational guidance                | Describing planned behavior as shipped                                  |

### Offline local service

The offline local path is separate from the Firebase preview/cloud runtime.
`tmt office layout show/apply` and the loopback HTTP service call the same
whole-world access boundary in `tmt-adapters`; the service serves the embedded SPA
without mirroring state into it. Browser and control tokens are distinct, and
the adapter accepts only exact IPv4 loopback requests. This is a local browser
adapter, not a work connector: it does not receive or dispatch remote work and
does not execute CLI work.

The local overview (`/local`) renders the atomic whole-world snapshot. Identity
creation never adds floor or a personal room. Identity deep links select an agent
in this same world; they do not mount a second block editor. `use-local-office`
loads world, profiles, meetings and artwork with one cancellable mounted lifetime.
Profiles include saved/temporary lifetime and the native endpoint observation
(`active`, `offline`, `unknown`), never a lossy online boolean. The browser's
`identities/presence` owns labels and validation, not a second presence probe.
Offline and unknown identities remain in the complete directory; online Contractors and unassigned
saved identities project into the primary Lobby.

The directory includes nullable `selfReportedStatus` from the canonical identity
repository, batched independently of appearance revisions. `identities/identity-status`
owns strict browser decoding and stale projection; `use-status-clock` schedules one
nearest expiry and resamples on visibility, without network or per-actor polling.
`office-scene-model` projects fresh cues across home/meeting representations and
suppresses them for an identity with an open conversation HUD. Info retains exact
text and update/expiry timestamps, independently of endpoint presence. Avatar saves
and directory refreshes never use profile revision to order or overwrite status.
The [identity status contract](../../contracts/identity-status-v1.md) owns semantics.

`world-map/world-yjs` owns the mounted layout's Y.Doc and selective Y.UndoManager.
Stable object/module/area UUIDs are separate map entries; an entity's placement or
module value stays atomic. Retained freeform floor/door topology is one value.
Local gestures are tracked; bootstrap and confirmed native observations are not.
An observed replacement of the same entity takes precedence over an older local
inverse. Removed entities use explicit null tombstones: undoing an earlier move
must not resurrect an externally deleted entity. Order metadata cannot create phantom entities or delete another writer's
insertion. Y.Array order preserves restored paint positions. Each completed gesture
is one history step; an explicitly grouped name input coalesces within 500 ms and
ends on blur. The existing JSON decoder
admits input and history projections; an unrenderable inverse is reverted.
External changes invalidate pending format-conversion history, with a visible notice;
undo must not hide newer-format entities by reverting to an older representation.
The history is session-local, survives save acknowledgements and is discarded on
explicit reload or disposal. A 16 MiB accumulated Yjs-update budget checkpoints
the current projection before the next edit, clears older history and reports it
in the HUD. This is an update-byte budget, not a measured JavaScript heap limit.
`use-world-editor` owns the saved revision, migration fence and write queue.
Topology, occupancy, furniture and resource attachments share this selective Undo/Redo;
undo results auto-apply, but the history itself is not persisted.
Completed gestures and property changes auto-apply through a 300 ms coalescing queue.
One revision-fenced write runs at a time; acknowledgements advance the saved base
without replacing newer local edits or clearing history. A failed or uncertain
write pauses the queue and retains local changes; retry or reload is explicit,
never an automatic overwrite or rebase. Explicit reload discards local changes only
after a successful read. Native validation owns commit admission and affected
object diagnostics; the browser may display an invalid intermediate draft.

`WorldYjsDocument` encapsulates raw shared maps and order arrays. Its typed cell
union preserves atomic placement/module values; callers cannot write arbitrary keys
or mutate stored JSON through input/output aliases. Whole-domain decoding and delta
preparation precede a single Yjs transaction. This provides coherent observation,
not exception rollback or native geometry admission. UI snapshots retain their
render identity between history changes; SDKs and presentation do not own shared types.

Yjs is an MIT-licensed browser dependency, not another database or authorization
boundary. SQLite and the existing whole-world JSON/CAS port remain authoritative.
Clean refreshed native snapshots enter the Y.Doc as untracked observations without
clearing local history; dirty/conflicted drafts are not silently rebased. No Yjs
network provider, awareness channel, binary-update endpoint or cross-client CRDT
persistence is shipped here. Simultaneous native/agent writers still use the
revision fence; selective browser history alone does not implement live collaboration.

Future providers (planned, not implemented) must adapt the same Yjs update/state-vector
protocol without importing Firestore or Cloudflare SDKs into layout policy, history
or rendering. Provider transport, durable update/checkpoint storage and authorization
are separate responsibilities. Incoming updates must have an untracked origin;
neither an origin nor CRDT convergence is authorization or native geometry admission.
Local durable authority remains the user's SSOT. A provider implementation must define
document identity, bootstrap, offline retention, acknowledgement, deduplication,
reconnect and deletion semantics, and replace the standalone session's document-reset
checkpoint with coordinated compaction. Independently seeding each client from JSON
or replacing a live synchronized Y.Doc is not a valid provider integration. The current
JSON/CAS bridge is not a persisted CRDT log; adding a provider requires that explicit
storage/protocol slice rather than silently making remote storage authoritative.

`office-population` owns the read-only home and meeting-membership projection for
both the directory and `office-scene-model`. Meeting instances retain the same
identity UUID and appearance; selecting one carries an explicit area context,
not an encoded identity string or a second inbox. Complete searchable rosters
retain offline and unavailable members independently of finite online previews.
`rendering/world-geometry` indexes sparse floor runs, derived boundaries, area
anchors and ordered placements once per world change. It caches at most six
avatar slots per occupied area, wholly on that area's floor and clear of floor
furniture. Tiny or crowded areas can have fewer or no visual slots without
hiding roster members. Actor slots and additive meeting authoring share the sparse
`world-map/free-floor` rectangle intersection/subtraction helpers. Sparse ownership
queries use `world-map/floor-index` in both map projection and object discovery;
no second object-area relationship is stored. Visible chunks coalesce
floor primitives; panning reuses a viewport margin, and rebuilding evicts unused
art textures. Geometry does not synthesize furniture, rooms or surrounding floor.
`rendering/world-projection` owns the cutaway transform. In v4, public circulation
uses the unexpanded lattice; each module reserves 16 display units inside its
north edge for the rear wall. Its ground coordinates map into the remaining
interior height. Ownership comes from the existing admitted floor index and
module bounds, not a second layout. Upright art retains its aspect ratio and
anchors to its footprint. V5 projects vertical ground distances at 7/8 scale,
retains the 16-unit wall rise and rigid artwork, and anchors upright furniture
at its footprint center. The projection owns that anchor offset for both painting
and drag inversion. An empty outside wall face remains explicitly unowned rather
than falling back to the room behind it. Wall elevation is not floor depth. Painting, culling,
object hit bounds and drag inversion share this projection. Floor queries and
actor-slot admission retain native coordinates; the camera and HUD anchors use
projected scene coordinates. Fit adds framing margin without resizing the canvas
or changing the saved world. V1/v2 retain 5/8 floor depth. In v3, inter-row gaps project to 20 scene units so
the next 16-unit back wall leaves four visible corridor units. Interior sidewalls
use four-unit thickness, leaving the same clear width along vertical lanes;
exterior sidewalls retain six-unit thickness. The piecewise transform
and its inverse also serve ghosts, anchors and dragging across rows. No stored
room or object coordinates change. V4 no longer expands those gaps: its wall
and mounted-object projection use the derived room owner, including the inverse
for side-wall dragging. Side faces stay inside their owning cells. Whole-cell
ghosts, their measured HUD target and Fit share the same wall-reserve bounds.
Public circulation paints below compressed interior floors and wall art. Interior
wood is inset within module silhouettes so transparent atlas corners reveal sky.
Only door openings receive underlays through the wall reserve; rectangular wood
foundations must not be painted beneath entire rooms. These are paint bounds,
not occupancy or placement coordinates.
These dimensions remain under visual review.
Exterior common-floor edges render as low circulation rails, derived from the
same floor ownership. Horizontal rails reuse the atlas's metal crown rather than
compressing a room facade. Front and rear room walls share the same full
rise, even when visual references show a lower foreground wall; use those
references for materials and junctions, not wall-height differences.
Older layouts retain their portal artwork. Atlas frames are owned by
`architecture-art`, not per-room styles. Threshold decking belongs to the floor
pass below wall faces.
These cutaway rules describe retained pre-v6 layouts only. The approved v6
platform model supersedes equal-height walls: its single floor projection has no
wall reserve, closed boundaries render low slab edges, and open boundaries have
no doorway art. Skybridges and platforms share the same floor plane. Construction
ghosts use that same flat footprint. V6 authoring no longer offers the Walls
library or mounting action; historical mounted content still requires explicit
conversion, not silent deletion. See the owning
[platform contract](../../extensions/tmt-office/contracts/modules-v2.md#version-6-cosmic-platforms).
This describes the current implementation. The
[modular-cell target](../../extensions/tmt-office/contracts/rooms-and-walls.md#user-built-world-and-area-lifecycle)
derives circulation and boundaries from fixed slots, replacing freeform authoring
without independently editable module/floor graphs.
Native [versioned modular topology](../../extensions/tmt-office/contracts/modules-v2.md) projects module
slots into the same map validator and preserves the source in the world codec.
Browser `world-map/map-source` owns the v1–v8 source union and read-only projection
cache. Rendering, population and object discovery consume that geometry; world
history and Save keep the source, including material changes. Modular area/object
edits share those owners and reject direct floor writes. `module-authoring`
offers cardinal office neighbors through the shared bounds/reserved-wing rules;
adding a named, unassigned module uses the same draft/Undo/Save flow. Canvas
holograms and the anchored accessible name form only select an eligible slot
until Add office commits it to the draft. `module-authoring` also constructs a
source-only removal candidate; `world-object-placement` supplies spatial support
checks shared with wall placement suggestions. The removal preview includes
unsupported objects anywhere in the candidate, not only anchors inside the
selected room. The central Lobby is protected. Removing an empty module retains
ordered objects, identities and canonical meeting membership, using normal
Undo/Save; native Save rejects disconnected or otherwise invalid candidates.
`module-removal-dialog` computes the impact only after an explicit review request;
its native modal owns focus/Escape, with scrollable details and fixed confirmation
actions. Ordinary area-name or furniture edits do not run removal admission previews.
Central-grid meeting expansion appends the next stable wing slot through `meeting-module`,
sharing saved and preview circulation from `module-geometry`. Canonical room
creation precedes the spatial draft, so Undo do not delete that room.
V5 uses necessary public branches and adjacent meeting slots; retained v4 geometry
is unchanged. `module-upgrade` offers an explicit compact draft preview and moves
room-owned floor and wall objects through the shared relocation function. It
rejects ambiguous support rather than dropping objects. Undo restore the
source, and native whole-world Save remains the admission authority.
V6 connects cardinal office/Lobby neighbors with centered bridges; distant
offices require intervening platforms rather than a perimeter bypass.
Meeting pods have a gap from their public spine
and separate entrance branches; missing slots do not add branches. The existing
native module projection and browser counterpart own openings and floor together.
The explicit platform preview reuses the same object relocation and draft history;
it never silently reinterprets or overwrites a stored v4/v5 layout.
V7 retains v6 personal-office paths and fixed meeting positions, but omits the
meeting spine, branches and openings. The native modular admission permits only
these meeting components to be independent; ordinary Office connectivity remains
required. This retained format is readable but is no longer a conversion target.
Reading old worlds never performs conversion.
The reserved meeting wing uses fixed visual slots with the standard 24-unit
inter-platform gap. Empty slots retain their space; adding or removing a meeting
does not change the main campus projection or move another meeting's scene anchor.
V8 is the unified-area layout: all non-Lobby slots use one grid and their binding
selects Office or Meeting room use without altering bounds or bridges. The fixed
display lattice is independent of occupied rows/columns. Meeting lamps share the
existing mechanical housing and only recolor its inset; a nameplate icon provides
a non-color cue. `AreaCreationForm` owns the use choice for a shared prospective
slot. `MeetingCreationForm` retains the canonical room receipt if attachment fails;
the same form links a selected existing area without replacing furniture or
retargeting any resource. Clearing an area binding never deletes an identity or
canonical room. Yjs still owns only the layout change, not canonical-room creation.
New installations receive the native furnished v8 platform preset described in
[root architecture](../../ARCHITECTURE.md#typescript-workspace-boundary). Existing-world
conversion uses the explicit module-upgrade preview described in root architecture;
retained layouts keep object editing and explicit area removal to repair rejected inputs.
A new preset never replaces saved content.
Material variants share geometry, picking and placement admission; textures do
not define topology or executable extension behavior.
`office-scene` paints the controlled world with shared floor/wall materials,
indexed artwork and host-owned interaction cues. Windows/lights project separate
world objects. The backdrop is static space, not another material-covered floor.
Wall direction, indoor face, elevation and edge coordinates edit the same object
and preserve its resource reference. Dragging inverse-projects the painted origin;
preview and commit use the same snapped position. The wall collection supplies
windows, lamps, posters, editable signs and link plaques through the existing prop
catalog. Its creation recipes suggest available walls within the selected area;
they do not own geometry or Save admission. Custom art can choose the same object
kinds. Coordinate forms apply one complete bounded edit, rather than admitting
partial numeric input. Full wall-face visual acceptance remains in progress.
Web destinations use the
existing extension binding and guarded review path, as defined in
[extension v1](../../extensions/tmt-office/contracts/extension-v1.md#web-destinations).

`scene-application` and `scene-frames` retain renderer initialization, resize,
teardown and invalidation-only scheduling; hidden views do not render.
`office-canvas` coalesces hidden input changes and exposes an accessible
directory fallback on failure. `scene-textures` owns mount-local art textures;
`scene-materials` decodes one bundled architecture source and lazily caches at
most three finish atlases per scene. `architecture-material` applies the bundled
surface palette within the reviewed frames, preserving alpha and relief; it
does not introduce a theme loader or geometry. Module material in the existing
world draft selects each owned face; public connectors remain Workshop. Style
edits use the same history and native Save as other module edits. Disposing a
scene releases all frame views before their shared sources. `scene-wall`
nine-slices those frames over `world-geometry`'s cutaway bounds. Room side infill
and crown are separate atlas views: only a physical north endpoint gets a crown,
not each boundary/portal split. At equal ground depth, front faces paint after
side bodies to retain their terminal posts. Overlapping atlas crops are recolored
once per source pixel. The same geometry
owns culling extents and wall/mount paint depth; artwork does not define topology.
V6 mechanical edges use `platform-art`'s explicitly reviewed source frames and
silhouette clips, decoded once into nearest-neighbor textures. `scene-platform`
repeats straight sections and fixed-scale lights/brackets over the same derived
boundary runs; Lobby size never scales up the hardware. The metal kit is shared
across rooms while floor finishes remain independent. `office-scene` paints the
v6 shell before upright furniture and actors, retaining depth order within each
pass; thin platform rims never occlude supported overhanging art. Pre-v6 cutaway
walls remain interleaved with content. Selection uses a beveled
floor contour, not filled floor-run rectangles. No per-frame image processing,
blur filters, interactive sprite nodes or additional stored geometry are added.
Module passage descriptors come from `world-map/module-geometry`, shared with
floor projection. A v2 short passage paints its east/south portal only; its other threshold
remains physically open without a duplicate arch. Standalone and meeting openings
retain their frames. V3 grid corridors retain both room thresholds separated by
visible common floor. V4 uses a 2×2 Lobby and independently generated grid gaps;
office creation previews derive their new circulation from that same projection,
not the old paired-room bridge formula. Old sources remain readable. Preview modular
layout converts eligible v1/v2/v3 inputs in the existing draft; native Save must
accept every retained placement before persistence. The screen-stable star field uses bounded static geometry,
not animation or per-star scene nodes.
Floor furniture retains its stored compositing order. In v4 its stack paints
after the owning rear face and before foreground cutaways; footprint-bottom
sorting must not cover desks with their rugs.
Source frames are defined in `architecture-art`, not inferred from sheet cells.
No user URL or executable artwork is admitted by these rendering owners. This
architecture slice is under visual verification; it does not imply that the
modular editor or the remaining source sheets have been admitted.

HUD panels overlay the entire camera viewport. Directory and canonical room
management live in a collapsed Office menu; the scene retains meeting creation
and area/member entry points. There is no layout editing mode or Save/Cancel bar.
The right-side context panel shows room properties when its floor is selected,
object properties when a placement is selected, agent Info/Chat when an identity is
selected, and the visual catalog otherwise. `WorldTools` owns the shared inspector
slot; `useAgentConversation` retains the session, not its screen position.
The agent inspector fills the available height below the shared header. Its compact
header owns accessible Chat/Info icons and a context-specific refresh control;
chat refresh is portaled from the retained conversation owner, not a second request
controller. Info scrolls within the panel, while chat history uses the remaining
space above its composer. Layout inspectors retain their content-driven height.
Changes apply automatically; compact Undo/Redo and pending/error status remain
visible for layout selection and hide during agent interaction. Escape or a
background click clears selection. Room removal still uses
an explicit impact confirmation.
Expansion holograms, plus marks and labels belong to the canvas, not HTML hit
overlays. Eligible slots reveal a hologram on hover; clicking pins the preview
and opens its name form. Directory entries provide keyboard access to the same
slots. Holograms share pan/pinch and click-versus-drag handling.
Whole-office creation uses a compact measured card. The renderer supplies the
entire hologram's screen bounds; `selection-anchor`
selects a non-overlapping side when space permits and clamps tight layouts to
the HUD-safe viewport. `use-anchored-panel` measures the actual wrapped build
context controls and the lower viewport boundary through shared DOM refs; it does not assume fixed
header heights. Creation forms do not dispose the inspector's state. The office
name form has no location dropdown: select a different canvas slot or use the
directory's accessible slot list. Resize observation changes presentation only and ends when
the card unmounts; it does not schedule an idle rendering loop.
The retained agent Chat/Info HUD is suspended while browsing the directory/area
roster or inspecting an object; switching recipients/room contexts guards unsent drafts.
It occupies the same right-hand inspector as layout selection, without a floating
window or minimized bubble. Its mounted session preserves a draft when switching
to furniture/floor selection or clearing selection. Profile drafts and resource dialogs retain
their existing owners. Refreshing the overview preserves these mounted owners,
including on read failure, instead of discarding open work. The geometry
editor commits a completed object drag once; pointer cancellation does not enter
history. Props drag directly after a screen-space movement threshold; floor and
background drags pan. Ordinary wheel/two-finger scrolling and Shift/middle drag pan. Browser
control-wheel pinch zooms around the gesture anchor; camera limits remain shared
with the zoom buttons. Object library cards reuse the admitted indexed-art renderer
and mount previews only while the library is open. Legacy pixel basics are collapsed
after the current art collections; catalog search retains access and existing prop
digests remain valid. Selected furniture exposes Delete, guarded Delete/Backspace
shortcuts, and corner-drag rotation for artwork with distinct directional views.
Reviewed static furniture has immutable directional successors selected by
`props/furniture-upgrades`; a completed turn writes the successor reference with
the rotation in one history entry. Reading, selecting and cancelling never
replace saved art. Unsupported repeated-frame artwork retains an explanatory,
disabled precision control. The scene previews snapped quarter
turns and validates placement before one editor transaction on release; Escape,
capture loss, and blur discard the preview. The held rotation replaces the source
object visually; cancellation restores its original layer without a world edit.
Floor support can use an explicit shallow base while art projection and picking
retain the complete upright envelope. The art's feet anchor at the base front;
rotations preserve its physical center. Authoring recipes adopt a base only on
an explicit placement, move or turn on v6+ platform maps, never while reading a
world or editing an earlier map projection. The
[world contract](../../extensions/tmt-office/contracts/world-v1.md) owns its bounds and admission.
A precision rotation action remains
available for keyboard users and uses the same centered rotation and placement
validator. Grid rounding is relative to the canonical footprint, so odd-sized
objects do not drift after four turns. Room settings belong to floor selection, not the
furniture inspector. `use-catalog-drag` owns transient
catalog pointer capture and cancellation; the canvas lends a `CatalogPlacement`
projection port, not a layout writer. The scene uses its existing camera inverse,
object projection and placement validator for an artwork/footprint ghost. Drops
over UI overlays are rejected by hit-testing the canvas, and cancelled gestures
release capture and preview textures without history or writes. A valid release
is revalidated against the current draft and enters the existing world history
once. Touch scrolling and click-to-add remain available without a drag mode.
Choosing or dropping art selects the new placement and closes the library.
The inspector shows room settings or the selected
object, not both forms together; its object preview uses the same admitted art.
Library search filters existing pack/prop labels without another catalog or draft.
Selection owns catalog visibility, including Pixel workshop additions. There
is no separate placement tool; unavailable objects remain selectable repair targets.
Modular room plaques anchor to the projected rear wall header; actor/floor anchors
remain unchanged. The shared nameplate painter keeps text legible across zoom levels.
Area and exact-instance agent selection is painted on the platform, below all upright
props, actors and nameplates; object outlines and rotation handles retain their separate
foreground layer. Area accents follow usage: teal personal offices, purple meeting
spaces and warm gold lobbies. An agent's room context does not select its floor.
Background conversation refresh preserves the feed/composer geometry: its existing
header refresh control reports busy state without inserting a transient toolbar row.
Object room context derives from `object-area`, shared with extension discovery;
choosing a room clears object selection, and removing a placement returns to that room.
Object coordinates and surface controls are collapsed under Precise placement;
the automatic wall-placement action lives there too, not beside every floor item.
Optional notebook/link bindings live under Object action. Rotate/remove remain directly
available. Expanding the controls does not create another draft or bypass admission.
Meeting rebinding is similarly collapsed under Meeting link; furnishing remains
directly available. Rebinding does not retarget existing resources or move members.
Room finish choices show static wall/floor samples from the same reviewed atlas
and recoloring function as the scene. Native radios preserve keyboard selection;
preview failure keeps named choices usable. Preview cards allocate no WebGL scene.
One-time layout conversion controls follow the room/object inspector; their
explanations are expandable. Conversions create one undoable change and use the same auto-apply queue.
Named module placement and object coordinate
forms provide keyboard editing through the same draft and history. Freeform
painting, zoning, terrain-coordinate forms and manual door gestures are retired.
Legacy read geometry and explicit area-removal repair remain; their normalization
is also used by the module projection. There is no independent map-only history
or alternate floor writer in the browser.
`world-map/meeting-preset` is an explicit additive recipe, not a replacement for
the native initial-Lobby preset. It references the admitted `bundled-extensions`
definitions and existing furniture packs. Table/chairs use one clear floor
footprint; functional boards use the shared occupied-wall placement suggestion,
not a separate meeting wall algorithm. Missing floor or wall space rejects the
whole recipe without changing the draft. The recipe enters one history step.
The meeting UUID identifies its lazy whiteboard resource and
discussion category in the existing board store; broadcast still requires an
explicit audience. Rebinding or removing
an area preserves existing object references. `RoomPicker` also serves the
standalone room manager: conditional room writes are immediate, explicitly separate
from the layout auto-apply queue, and feed revision-checked room observations back to the
mounted Office. Closing that manager retains its draft without changing a roster.
`RoomEditor` is shared by that manager and the compact world-anchored creation
card. An uncertain write retains its UUID and draft; explicit readback and review
can adopt the stored room without rewriting it. A failed spatial attachment
retains the confirmed room and area ID for placement retry. Existing unplaced
rooms can enter the same path without a room write. Selected meeting areas open
the same targeted member editor; changing its target cannot discard an open draft.
The obsolete generated-room geometry, static public envelope, private block
canvas adapter and local block transport have been removed. `office block`
remains a remote-only command; local writes have one whole-world owner.
`extensions/world-extension-groups` derives discovery groups from floor placements
or mounted wall faces, not resource IDs. `WorldObjectActions` presents the selected
area (or identity home), common-floor tools, then expandable other-area groups.
Coordinate descriptions distinguish repeated objects. This is navigation, not
authorization: spatial activation and every listed entry use the same guarded
binding callback, with no implicit resource creation, retargeting or dispatch.
`scene-components.ts` consumes world-projected bounds and inert host action metadata,
not floor placements or an independent wall transform. It owns the shared static
interaction overlay for floor and wall entries. Idle badges sit inside the
object's lower-right corner; only the hovered/focused action label expands above
it, retaining the right edge and sizing to wrapped text. Its measured text bounds
and ordered display groups also supply picking, so neighboring
objects cannot capture clicks through the visible expanded label. No extra frame
loop or extension-specific hit policy is introduced.
The independent `whiteboard/scene-contract.ts` admits inert document elements,
not World sprites or arbitrary SVG. Native policy and wire admission live in the
corresponding `office_whiteboard` core/adapter modules and share literal vectors.
Document persistence uses SQLite conditional saves and operation receipts in one
transaction. `whiteboard/document-contract.ts` admits resource envelopes;
`LocalRuntime.whiteboards` reuses session authentication, cancellation and timeout
ownership without another fetch loop. It verifies target and operation receipts
and preserves explicit conflicts. `whiteboard/editor-state.ts` owns the mounted
draft, explicit saves and exact uncertain-operation retry; `history.ts` owns
bounded undo/redo. `editor-canvas.tsx` owns transient gestures and responsive raster
lifecycle, and `drawing.ts` owns the admitted scene painter. Pen previews append
and paint incremental segments, not full-scene state on every pointer event.
The editor is shared by a direct local route and a lazy-mounted spatial panel;
snapshot capture and request delivery retain separate state owners.
[Whiteboard v1](../../extensions/tmt-office/contracts/whiteboard-v1.md) owns
document fields, work budgets, resource behavior and the immutable reference contract.
Native `office_whiteboard::snapshot` now owns capture metadata and exact saved-revision
policy; `storage/office_whiteboard/snapshot` appends the retained scene, selected IDs
and annotation. The capture ID doubles as its operation receipt, so replay needs no
second operation table. It reads and validates independently from the live document
after capture. Its `snapshot/image` storage child attaches one normalized PNG using
the native `office_whiteboard::image` decoder. One pixel digest defines attachment
replay; the retained PNG bytes survive reads and retries without re-encoding.
The browser producer reuses `drawing.ts` on the frozen scene, not a native
second renderer. The native HTTP adapter exposes capture, retained JSON and raw PNG
through the existing bearer/Origin/response owners; one route parser also selects
its body limits. `snapshot-contract` admits owned metadata/resources;
`snapshot-state` retains capture and PNG retry stages independently from document
edits. `snapshot-review` displays the returned stored PNG with a mount-owned URL.
The local runtime shares one fetch/deadline/disposal owner for JSON and PNG;
`transport/response-body` owns bounded byte acquisition reused by pairing and PNG
transport, while MIME checks and error mapping stay with each protocol.
Native `office_whiteboard::access` exposes read-only snapshots through the existing
companion protocol, calling the same retained repository as HTTP. Its parent-side
`office_companion::whiteboard` decoder validates target/shape or PNG pixels over the
shared bounded process owner. CLI file export stays outside the companion and uses
the adapter's private-staging/no-clobber publisher. Local reference projections
share conformance vectors; copying references submits no request. The
[dispatch capability](../../extensions/tmt-office/contracts/dispatch-v1.md)
now composes explicit UUID recipients over the native request service, with one
transaction for inbox writes and its immutable operation receipt. `LocalRuntime.dispatch`
admits input and checks returned operation/audience over shared auth and cancellation.
For a new single-recipient request, native dispatch separately claims one
advisory pane wake and checks the active binding and tmux endpoint immediately
before sending a request-ID and explicit-recipient-UUID `x show --incoming`
instruction. The inbox receipt remains authoritative; an absent, failed or
uncertain wake does not change acceptance or trigger an automatic retry.
Announcements and roster sends remain inbox-only.
`local/dispatch-composer-state` owns frozen message/identity intent and explicit
retries for requests and announcements. `local/dispatch-composer` owns audience
selection, review and acceptance display without mirroring request state.
`whiteboard/snapshot-send-state` adds only immutable reference/message formatting;
`local/board-share` uses the same composer for a live thread UUID and native reader
instruction, retaining the underlying board while explicitly guarding request
draft disposal. It adds no reference grammar, snapshot or dispatch store.
`local/broadcaster` selects no-reply semantics. Capture/image retries remain separate.
Meeting membership has its own [stored resource](../../extensions/tmt-office/contracts/meeting-room-v1.md).
Full-roster dispatch checks revision and effective UUID audience within the enqueue transaction;
replay of accepted operations does not consult later membership. `RoomPicker`
edits/adopts rosters through the local port, sharing the controlled
`IdentityChecklist` with individual requests. Refresh cannot alter reviewed intent.
Room retirement uses the same port and revision-checked repository transition.
The manager confirms retention of spatial/content resources before retiring;
active observations drop that room and fence older in-flight list responses.
The browser must not infer membership from presence. The host dispatch capability
also accepts explicit announcements through the same request service and receipt;
its no-response policy is core-owned.
`LocalRuntime.requests` exposes owner history, exact request detail and acceptance
receipt recovery through the same authenticated, abortable transport. The strict
`local/request-history-contract` checks scope, keyset order, exact text bytes and
state tags. History reads canonical attempts, including acknowledged and unknown-
sender work; it neither mirrors chat messages nor acknowledges them. Bounded
response reads reuse `transport/response-body`. `local/agent-conversation` composes
the shared request composer with `conversation-state`: visible-only, cancellable
three-second observation pauses after fifteen minutes or a list failure; refresh
or reopening starts a new bounded window. The chat window holds ten exchanges
and uses at most two concurrent detail reads. Full bodies are fetched on first
display, explicit refresh or summary changes; changing pages releases old bodies.
Closing, Info-only browsing, layout selection and document
visibility pause reads without cancelling accepted work. Reopening the agent shows
the same request history. A view-local seen marker is not an inbox acknowledgment.
`dispatch-journal` stores only frozen unconfirmed intent in origin/tab
session storage, with separate direct-recipient/context and room-roster scopes.
`local/room-message` owns one retained room composition: its fixed-target
`RoomPicker` requires explicit adoption and the shared composer reviews the exact
roster before sending. Missing rooms never substitute a global audience.
Uncertain recovery retains the original roster/revision; a definitive stale-roster
rejection clears that pending operation before a newly reviewed send.
Opening a member from
a meeting scopes both history and direct intent to that room; it does not select
the roster. `RequestService` checks that member in the same enqueue transaction,
without rejecting a private message because another member changed. Switching
room context is a conversation switch, including its draft-disposal guard.
The shared composer can recover an existing receipt without
resending; explicit retries retain the operation. Direct chat freezes intent on
Send without a separate review screen; multi-recipient composers retain review.
Accepted messages remain solely in native storage. No bearer token is persisted,
so page reload still requires the authenticated session URL.
The revision-aware draft owner does not move into PixiJS. The former per-block
HTTP/private transport and browser port are removed. Retained schema-19 layouts distinguish identity UUIDs from
the installation lobby through `LocalBlockTarget`. Whole-world migration reads
them without creating rows, preserving saved empty overrides and resource bindings.
The [local target contract](../../extensions/tmt-office/contracts/local-block-targets.md) records
that retained source format, not the new local editing interface.
The [local service contract](../../extensions/tmt-office/contracts/local-service-v1.md) owns its
routes and bounded shared prop resolution.

The [map v1 foundation](../../extensions/tmt-office/contracts/map-v1.md) is the topology
owner, separate from resource contents and existing stored block layouts. Native
admission derives reachability and walls; browser `world-map` only decodes bounded
values, projects draft geometry and applies module or retained-area edits. It must
not authorize commits or silently repair invalidated placements. Whiteboard and pixel
drafts retain `editor/snapshot-history`; the production layout uses `world-yjs`. Production
UI and `office layout show/apply` use the whole-world API. Their HTTP and private
companion entrypoints share `office_world::access` and its close-before-publication
storage operation. The CLI requires an explicit revision and the read fingerprint
at revision zero; it never selects an identity or starts the service implicitly.
Legacy native/browser scenario fixture conversion is unfinished; production
local block transport has been removed.
The native [whole-world value](../../extensions/tmt-office/contracts/world-v1.md) now composes
map and prop admission, retaining ordered placements and reporting invalidated
object IDs. Wall/window/door rules are core-owned. Schema 28 and
`storage/office_world` now provide atomic revisioned saves and explicit legacy
cutover, including transaction-time identity, room and artwork checks. The
world contract owns those semantics. `local_service/world` and `LocalRuntime.world`
now expose protected whole-candidate reads/saves and preserve placement diagnostics.
`world-draft` contains pure layout mutations; `world-yjs` shares one history across
topology and dependent placements. Former
bundled functional objects use the existing typed extension bindings without
copying their resources. The production editor now consumes this world; no
independent browser placement policy authorizes a save.

Presentation profiles follow the same local-only composition without joining the block
model. `tmt-office-model::office_profile` owns the exact catalog, safe-text bounds and UUID-byte
default. Schema 15 stores one optional canonical override per immutable identity UUID;
reads do not materialize defaults. Immediate transactions implement create, no-op,
exact-retry and conflict semantics. The optional immutable avatar reference is admitted
against the installation-owned catalog in the same transaction as the profile write.
Removing or corrupting a selected pack never rewrites the profile: the SPA renders its
stored robot defaults, and exact reinstall restores the custom art at the same profile
revision. Retirement retains the row while active projections
exclude it, so a same-name replacement inherits nothing. The SPA profile port and native
commands both call this owner. Presence is a separate binding observation: offline saved
identities remain editable but are never drawn as present in the room.
`profiles/avatar-art.ts` admits the bundled 32×48 v2 robot pack, then projects
stored appearance choices through its declared shell, body and accessory slots.
`robot-materials.json` is bundled authoring metadata, not a second installed-pack
schema or mutable catalog. Palette tinting shares `indexed-art::tintPalette`
with furniture; fixed eyes, brass joints and the chest label retain their colors.
Accessories preserve the visor and antenna. Canvas and SVG consume the same
two-digit raster and share physical size/chest-mark geometry and sampled label contrast from
`profiles/avatar-layout.ts`. Artwork is static, without a lighting/animation loop.
Changing defaults neither rewrites profiles nor replaces immutable custom art.

The #238 source implementation adds schema 16 and the installation-owned local prop catalog defined by
`extensions/tmt-office/contracts/prop-pack-v1.md`. Embedded built-ins remain outside mutable rows and
quotas; installed exact bytes use one revisioned SQLite owner and request-scoped validation.
This source capability is not evidence that a separately installed companion release
contains it; use the verified release and native installation records for availability.
`tmt-office-model::codec::office_prop` owns the one byte/JSON/semantic validation path used by
CLI, SQLite revalidation, and preview; reusable placement, reference, footprint,
rotation, and room-bound rules remain in `tmt-office-model::office_block` without adding a
second codec or generic extension framework.
Local v2 block child requests and replies share the core-owned 64 KiB transport
ceiling; remote v1 and unrelated 4 KiB envelopes do not inherit it. Prop catalog
cursors use their own encoded-envelope bound. Preview responses must echo the exact
candidate digest and the exact private loopback preview ID, URL and browser token;
the client applies bounded connect, read and write deadlines.

Avatar authoring warnings run only on admitted packs and are appended by the
CLI validate command, like prop warnings. The indexed-art edge predicate is
shared; each domain owns its other advisory checks. Neither warnings nor visual
review change strict admission, source bytes, immutable digests or catalog writes.
Schema 17 adds the independent installation-owned avatar catalog defined by
`extensions/tmt-office/contracts/avatar-pack-v1.md` and `avatar-pack-v2.md`. V1 uses 16×24
one-digit rasters; v2 uses 32×48 two-digit rasters and a version-specific digest
domain, without increasing the 32 KiB file or 6,144-cell pack budget. Native
`avatar_format` owns versioned admission/summary geometry; browser `avatarRaster`
preserves the encoding through both catalog selection and preview. Packs retain
their own framed digest, revision singleton, cursor domain and 64-pack/256-avatar quotas.
Only narrow indexed-art predicates, replay recognition, cursor encoding and the typed
expiring preview lifecycle are shared with props. The catalog does not join furniture.
Profiles refer to an installed avatar by immutable digest/key, while catalog removal leaves
that profile value untouched. The authenticated SPA loads one bounded catalog projection at
startup, resolves the selected art before render and passes it to the same `Avatar`
composition and inert `IndexedRaster` used by retained robot art; there is no per-frame
storage or network lookup.

The local directional-prop implementation extends the same catalog with
[prop pack v2](../../extensions/tmt-office/contracts/prop-pack-v2.md). `office_prop` owns versioned
admission, framed identity and the derived prop-only input budgets; schema 18
widens only the prop BLOB constraint while preserving exact installed bytes.
`propFrame` owns browser orientation selection, and `rendering/indexed-art.ts`
owns interpretation of already-admitted one- or two-digit palette indices for
both SVG and canvas. V1 props retain raster rotation; v2 frames stay upright.
The local placement domain permits footprints up to 16 tiles per side so workshop
furniture scales naturally within the 32-tile room; v1 asset definitions retain
their 8-tile limit. Picking and rendering use the same admitted footprint.
Native and browser built-in registries retain frozen v1 references and add the
directional workshop and commons packs without catalog rows, quota usage or
revision changes. Commons noticeboard art is independently addressable; its
presence in the art catalog does not imply a functional binding.
Pack capabilities declare tint channels and directional text regions; placement
values belong to the existing layout draft and CAS transaction. Local layout v3
stores those values, while layouts without values canonicalize to v2. Schema 20
widens the same layout table's byte bound to 8 KiB without rewriting old JSON.
`resolvePlacedProp` owns shared browser definition/footprint/capability resolution;
`propFrame` applies tint and inert text for both renderers. Texture keys include
placement values, so instances share only identical art. The adapter's
`office_prop/quality` module inspects already-admitted packs for advisory authoring
issues; CLI validate appends these warnings without changing companion admission
or installation semantics. Visual review remains necessary.

Local pixel authoring uses the same native pack admission and
catalog CAS through `local_service::props`. Its paged list carries metadata only;
on-demand pack reads reuse the existing resolver. Browser `prop-catalog-contract`
checks exact-byte framed digests and revision receipts; `pixel-draft` reuses
bounded snapshot history and converts flat indexed work to ordinary v2 packs.
`pixel-canvas` owns the active pointer stroke, cancelling incomplete capture and
committing once to that history. Keyboard edits use the same paint function.
`use-pixel-catalog` freezes install bytes and revision until a confirmed result or
explicit release; it never rebases automatically. `pixel-workshop` keeps visited
drafts in the existing modal lifetime and reads only the selected library pack.
The world editor's single placement action accepts both built-in and saved art;
confirmed art is added to the mounted catalog observation, not a browser store.

The local editor's Add menu uses built-in and observed custom packs under the
whole-world draft/Save owner. The retained paired block editor uses its existing
block owner; it does not write the local world. Avatar admission independently
supports [v1 and v2](../../extensions/tmt-office/contracts/avatar-pack-v2.md), not prop-pack schemas.

The local discussion board follows the same companion boundary without sharing
the layout model. Lobby and room-bound entries mount one board view in a native
modal dialog over the same canvas; close retains the visited view and unsent
draft, and native dialog focus handling returns to the entry. The board uses
category buttons, recently active ordering, and an on-demand composer. Narrow
screens switch between the thread list and conversation without replacing the
board data owner. `board-navigation` guards category/thread/spatial-entry changes
using protection reported by the existing forms and dispatch composer. Forms
retain their own text; `use-board-mutation` retains frozen write/retry intent.
Refresh and ordering do not switch away from a selected thread. The standalone
route remains available.

`extensions/extension-contract.ts` and native `office_extension` admit separate
data-only definition/instance records against shared literal vectors. The
CLI `office extension validate` acquires two bounded no-follow files and transports
their raw text to the verified companion. `office_extension::validate_pair` owns
native structural composition; its typed preflight report claims structure only,
never installed artwork, handler authority or a content mutation. Native/browser
pair vectors cover binding mismatches and rotated edge bounds. The local
`use-world-extensions` composition root binds placed discussion, whiteboard, notebook, web-link and broadcaster
objects to existing resource views through `extension-binding`; admission alone grants no action.
Canvas and `ExtensionEntry` emit the same instance ID into guarded dispatch and
share each resource view's modal/draft owner. `useExtensionPanel` centralizes native
dialog focus/Escape behavior and keeps editable resource drafts mounted across closes.
The read-only notebook panel instead unmounts on close, aborting pending reads and
revalidating on reopen. `notebooks` owns the typed read port, inert text view and
explicit saved-identity attachment controls. It never owns note storage or writes;
the [existing notes owner](../../ARCHITECTURE.md#saved-identity-notes) resolves files.
The renderer receives inert `SceneComponent` values,
never capability names or board operations. `scene-components` owns action plaques,
hover/focus and matching hit geometry; `scene-props` shares admitted artwork
painting with ordinary furniture. Plaques retain readable screen size while the
world zooms; changes invalidate on demand, not through an animation loop.
`contracts/record.ts` owns generic exact-record/text checks reused by artwork and
extension admission; artwork-specific limits remain with their existing contracts.
The [v1 binding contract](../../extensions/tmt-office/contracts/extension-v1.md) defines current
scope. Editable persisted extension placement, general external loading and broader
host capabilities remain under the accepted
[extension design](../../extensions/tmt-office/contracts/functional-props.md).

Local service startup opens and closes the shared storage before publishing its
ready receipt or accepting browser workers. First-run migrations therefore finish
before the browser's concurrent resource reads; unusable storage fails startup
instead of advertising a ready service with failing resource endpoints.

The board's storage and transport follow their existing ownership independently of
the block model. `tmt-office-model::office_board` owns its bounded values, actors,
receipts and cursor policy; `storage::office_board` owns the single
board revision, exact-UUID/owner revalidation, soft deletion, retry receipts and
indexed pagination. CLI calls use the verified companion one-shot protocol and
remain independent of the running web service. Authenticated loopback routes use
the same operations with a fixed Owner actor. Category discovery projects one
synthetic general category and distinct stored root scopes;
it is not a registry and does not inspect Git from the browser.
Schema 30 extends the original schema-14 categories with canonical room UUIDs,
preserving entries, receipts and revisions. Category discovery includes stored
room and repository scopes; membership is not an ACL. New room posts validate
room existence, while retained thread reads and exact retries survive removal.
The original `TMT-OFFICE/1` version probe remains byte-for-byte compatible.
Board calls additionally require the separate exact, bounded version-1
capabilities probe to advertise `office_board_v1` before dispatch. An older,
malformed, duplicate or unknown-only capability response fails as incompatible
without attempting the mutation; capability support is never inferred from the
package version.

There is no work connector, deployed service or shared browser runtime package yet.
The independently versioned native `tmt-office` companion currently implements
the [typed local protocol](../../extensions/tmt-office/contracts/native-companion.md), including
pairing, local status, an authorized assigned-block existence check and
revision-safe block show/apply. Pure native scene validation lives in
`tmt-office-model::office_block`; readable JSON belongs to `tmt-office-model::codec::office_block`.
The companion's scoped remote adapter reads one block and commits with its
server update-time precondition (or nonexistence for creation), then rereads
canonical state. It shares pairing refresh/renewal and scope locks, not a generic
CRUD service. Exact retries preserve the timestamp; conflicting revisions never
rebase automatically. Browser and native codecs conform to the same literal
block vectors. There is no local scene cache or second grant registry.
Its optional adapter feature owns discovery, Auth exchange/refresh, invocation-owned
resource-lease renewal, identity retirement hook consumption and protected
scope records. The public alpha companion is distributed through the verified
native release path; local service and whole-world operation availability in a
published pair still depends on a
coordinated release. Follow [native installation guidance](../NATIVE-INSTALL.md)
for release status and compatibility rather than maintaining another version ledger
here. The CLI's explicit `office` subtree installs, inspects, updates and deactivates
it through existing native owners; other CLI operations do not execute or probe it.
Shared native protocol values remain in the existing core.
Create each only with its first concrete consumer and reviewed contract.
Do not relocate established Rust, test, script or canonical skill paths simply
to make the tree symmetric.

### Owner retained spaces

`src/spaces` owns a bounded one-page projection of existing `agentGrants`, not
a new assignment store. Its Firebase adapter performs server-only, document-ID
ordered queries; Rules allow at most 20 documents to the admitted world owner.
The mounted world owns its disposable page state. Refresh clears stale data,
late results are fenced after disposal, and no per-grant listener is created.
Read failures do not imply an empty collection or revoked authority.

The space view selects a target-bound `BlockPort` from the existing blocks adapter.
The same editor/state/codec handles home and UUID blocks; switching targets remounts
the editor and discards unsaved drafts. Submitted writes may still complete against
their original target, but cannot repopulate another editor. Layout mutations do
not edit grants, profiles or notebooks. Expiry labels are observations, never
presence or permission decisions. See the [grant contract](../../extensions/tmt-office/contracts/agent-grant-v1.md#owner-inventory)
for paging consistency and limitations.

### Trusted pairing issuer

`extensions/tmt-office/typescript/services/office/functions` is a separate Node 22 Functions package, not a CLI
runtime or SPA dependency. Official Admin/Functions SDKs own token verification,
signing, Firestore transactions and HTTP platform integration; no custom JWT or
database client is introduced. Its [pairing contract](../../extensions/tmt-office/contracts/pairing-v1.md)
owns the wire format and recovery policy.

`pairing-contract` owns bounded wire decoding; `pairing-record` owns strict
persisted pairing/grant decoding and timestamp shape. `pairing-store` owns transactional
approval/grant state, compare-expiry lease renewal and live owner admission.
`pairing-service` verifies human approval/revocation, bound-agent renewal or
reduction-only agent/proof cancellation
authentication and composes external signing, with a final authority recheck before token
delivery. `pairing-http` maps transport/error results; `index` alone initializes
SDKs and exposes the function. Admin bypasses Rules, so server validation is an
independent trust boundary, not a substitute for downstream Rules enforcement.
No public unauthenticated begin write, alternate block model or work queue exists.

The service defaults off outside the strictly configured demo emulators.
Production activation requires separate ingress/abuse, retention, IAM and cost
review; instance/concurrency limits are not a billing cap. Native secure storage
and the integrated CLI flow remain separate delivery gates.
Service unit checks use its own Vitest/Oxlint/Oxfmt configuration. The existing
browser Docker owner adds Functions and the real-token pairing scenarios; its
service-owned privileged fixture is test-only. Standalone SPA type checks exclude
E2E imports; `type:check:e2e` explicitly checks the combined fixture with both
packages installed. Browser production source never imports the Admin SDK.

### Browser owner approval

`src/pairing` adds explicit owner consent under `/worlds/$worldId/pair`, not an
agent connection or general assignment manager. `pairing-contract` decodes the
bounded public fragment and exact response echo; shared literal vectors under
`extensions/tmt-office/contracts` verify conformance with the independently authoritative issuer.
Browser platform base64 APIs and a fatal UTF-8 decoder own encoding primitives.

`pairing-transport` owns only the bounded JSON approval/revocation POST to the
operator-configured endpoint, with no redirects, cookies or implicit retries.
The Firebase composition supplies the current token source; tokens stay out of
view state. Preview has no pairing transport; cloud mode requires an explicit
HTTPS `VITE_OFFICE_PAIRING_URL`, while emulator mode uses the fixed demo endpoint.
The request fragment cannot select a service or an actor.

`pairing-state` serializes actions for one mounted immutable request and records
confirmed or uncertain results, not a second Firestore snapshot cache. The view
requires explicit recognition before approval. Existing session/admission and
selected-world owners gate the route; unmount fences late completions without
claiming to cancel submitted writes. Skip-to-content focuses the main landmark
without overwriting the request fragment. The browser cannot claim agent tokens.

The approval form reuses `SpacePort` and its disposable one-page state to select
a disabled grant's retained block. Selection is separate from the native link
and freezes after the first attempt; retries cannot silently change assignment.
The issuer records immutable source intent on the pairing and reserves the
disabled source grant in the same transaction. Its transfer receipt prevents
concurrent or ancestral reuse; only a verified abandoned unclaimed reservation
may be superseded. Grant validation remains shared with claim and renewal.
No block content, profile or notebook is copied. The exact transition belongs in
[pairing v1](../../extensions/tmt-office/contracts/pairing-v1.md#retained-block-reassignment).

### Public deployment discovery

`auth/firebase-config.ts` owns Firebase configuration and issuer selection.
Vite publishes only the allowlisted
[native deployment descriptor](../../extensions/tmt-office/contracts/native-pairing.md) at
`/.well-known/tmt-office.json`; private environment values are never spread into
it. Preview and unconfigured cloud publish no usable descriptor. Invalid
configured deployments fail the build. Browser transport consumes this same
configuration owner, not a second deployment registry or settings file.
Literal fixtures verify generator/native-decoder conformance. This metadata does
not prove world ownership, activate the issuer or complete native pairing.

## Frontend stack

- React + Vite SPA with TanStack Router; no Next.js, SSR or TanStack Start.
- Jotai owns shared cross-view presentation state; component-local forms and
  selection use React state. Each mounted app owns its store. No identity,
  authentication or durable task state is inferred from a UI atom.
- TanStack Query is the chosen future owner for non-streaming remote requests
  when needed. It is not installed without a consumer. Firestore streams need a
  single subscription/cache owner; do not mirror authoritative snapshots in both
  Query and Jotai or create a second request lifecycle.
- Use the VoidZero component tools: Vite (Rolldown), Vitest, Oxlint and Oxfmt.
  This does not require adopting Vite+'s runtime/package-manager management.
  Office uses its own current Vitest configuration; the established native and
  tooling suites keep their existing runner contract pending a scoped migration.
  One pnpm lockfile records both. Root tooling/docs retain Prettier, Office uses
  Oxfmt, and no file has competing formatter owners.
- Drawing dependencies are allowed. Compare a library's actual map/drag/board
  functionality, accessibility, bundle cost, maintenance and license before
  adding one. The scaffold needs no canvas engine, sprites or
  speculative universal scene abstraction.

## Trust boundaries for later design

[Sandbox design](sandbox.md) owns the planned data-only prop and exploration
boundary. It does not introduce a runtime/plugin SDK into the current app or
extend the fixed-asset block codec. Keep future native inputs and rendering
conformant to a versioned contract, not a second layout model.

The browser is an untrusted client of server-enforced membership rules. A local
connector must be explicitly installed, paired and running before it can receive
remote work for selected local agents. One-shot block operations are a separate
scoped authorization path, not a requirement to run a background process.
Visiting, chatting, board editing, requesting work and
executing locally are separate permissions. An optional receptionist can route
work but cannot replace authentication or authorization.

Browser presence, connector connectivity, individual agent availability and
durable work state are distinct observations. Proximity or user-supplied content
never grants tool access. Shared requests correlate with existing local exchanges;
they do not mirror the SQLite database or become an alternate task engine.

The planned protocol defines version negotiation, ownership/correlation,
deduplication, expiry/revocation and failure semantics separately from the SPA.
Firestore deployment means an owner's Firebase project, not self-hosted Firestore
or automatic federation. No cloud provisioning, billing or deployment occurs here.

## Foundation behavior

| Input                                 | Observable result                                             |
| ------------------------------------- | ------------------------------------------------------------- |
| Open `/`                              | Local preview and no world/agents connected; no cloud request |
| Follow Setup or open `/setup`         | Planned setup and permission boundaries, no fake login        |
| Open an unknown path                  | Not-found view with a working return link                     |
| Expand preview details, then navigate | Disclosure stays open in this mounted app                     |
| Unmount and start another app         | Disclosure resets; no global/persisted UI state               |

DOM tests use the real router, with focused session lifecycle tests beside the
owner. Playwright under `extensions/tmt-office/typescript/apps/office/e2e` proves real Chromium/SDK/Auth Emulator
popup flow, cancellation, transport failure, memory isolation and default-preview
network inactivity. Its opt-in Docker target extends the existing emulator image;
it does not duplicate emulator pins or use the native tmux harness. Upstream
emulator CDN presentation assets are blocked, not replaced with fake auth
responses. Google's public popup/iframe library is still required: browser
tests allow its script GETs on `apis.google.com`, keep all auth requests local,
and reject other destinations. This is not a fully offline flow. Direct-SDK
Rules scenarios additionally prove tester/owner isolation and immutable creation;
the browser world scenario proves grant/create/read/revocation through the UI.
`agent-grant-rules.spec.ts` separately proves scoped custom-principal access,
claim mismatches, malformed/expired grants, one-way revocation with cached tokens,
retained blocks and capability isolation. Its privileged fixture documents and
unsigned emulator-only custom tokens do not prove production credential issuance.
These suites do not prove connector lifecycle, real Google login or remote collaboration.

On tester revocation, subsequent server reads/writes are denied and the app's
admission stream clears the view and detaches its world listener. Do not claim instantaneous
server stream closure or recall of prior data. Real cloud revocation timing must
be verified during the explicitly authorized pilot.
