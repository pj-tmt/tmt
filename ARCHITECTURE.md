# Architecture

The shipped CLI runtime is the Rust workspace in `rust/`. An optional Office
SPA foundation lives in `extensions/tmt-office/typescript/apps/office`; it is not a CLI fallback or a
shipped connector. The nested `typescript` pnpm workspace owns Vitest, fixture
and release-verification tooling; the repository root has no Node package. Nx
orchestrates explicit Rust and TypeScript targets through the pinned non-JavaScript
wrapper, with caching disabled. Neither Nx nor the pnpm workspace is a second CLI
runtime, an npm product, or a source-install fallback. A native source checkout selects
`rust/target/debug/tmt` (or an explicitly supplied native executable); a missing
native build is an error. No test, script, or installer may silently execute an
installed host `tmt` or a retired TypeScript product implementation. Node may
run explicit developer fixtures and verifiers, never serve as a product fallback.

Published releases are immutable. Source changes do not publish replacements
or migrate application data.
TMT remains an invocation-owned local CLI, without a remote MCP server, identity
memory or a separate inbox service. The independently installed Office companion may
run one explicit loopback-only browser service; it does not execute CLI work or change
the CLI's invocation-owned storage policy.

Any retained `better-sqlite3` use belongs to private developer tooling as an
independent oracle. It is not a Rust runtime dependency or an alternate owner
of native schema and application state.

## TypeScript workspace boundary

The `typescript` pnpm workspace has one lockfile, retained Node tooling and tests,
the `@tmt/office` SPA, and the `@tmt/office-service` trusted pairing service.
The two Office packages live under `extensions/tmt-office/typescript` as
parent-relative members of that same workspace and lockfile. They resolve only
their declared dependencies, never root-hoisted tooling packages; Office browser
specs reach the tooling-owned SQLite oracle through `typescript/test/support`.
Rust, root shell launchers, shared contracts and canonical skills remain outside
that boundary. `contracts/` holds core contracts only; Office contracts, vectors
and the Office skill sources live under `extensions/tmt-office/`. The Nx task graph orders only the Office SPA producer, embedded
native companion and installed-browser acceptance chain; ordinary CLI targets
remain independent. Read
[Office architecture](docs/office/architecture.md) for current SPA ownership,
the chosen React/Vite/TanStack/Jotai stack and the
[Office design](docs/office/design.md) for planned trust/lifecycle semantics.
Office runtime code must not import local SQLite/process adapters or native test helpers.
The accepted [World extension design](extensions/tmt-office/contracts/functional-props.md)
separates spatial composition from concrete board/notebook/broadcast features.
Bundled features are not automatically World core. Reuse existing domain services;
extract capability interfaces from consumers rather than adding another command
runner or exchange engine. The first [data-only binding](extensions/tmt-office/contracts/extension-v1.md)
composes discussion and whiteboard resource views with physical instances through
a guarded host registry; native/browser admission is separate from capability
registration. Shared `useExtensionPanel` owns the native modal lifetime, while
each resource retains its own draft and persistence owner. Whiteboard
`useWhiteboardPanel` admits document switches from the editor's aggregate leave
state: unsaved content requires confirmation, while pending document/snapshot/send
operations retain their original owner until resolved. Closing a panel is not a
document switch or disposal.
The local [whiteboard scene contract](extensions/tmt-office/contracts/whiteboard-v1.md) keeps
drawing values separate from World placement, request delivery and resource storage.
Pure scene policy belongs to `tmt-office-model::office_whiteboard`, its strict JSON boundary
to `tmt-office-model::codec::office_whiteboard`, and the matching browser projection to
`whiteboard/scene-contract.ts`; literal vectors cover both projections.
Document revision policy lives in its core `document` module, envelope admission
in the adapter, and atomic document/operation persistence in
`tmt-office-storage::office_whiteboard`. The local HTTP adapter and browser document port
reuse the existing session transport; neither owns editor history or request
delivery. World singleton creation is shared by world layout, board ownership
and document persistence through `tmt-office-storage::office_world` inside caller transactions.
Whiteboard `snapshot` policy captures a specific saved revision and validates its
selection/annotation. Its adapter owns metadata admission; the storage child module
appends an immutable scene copy using the capture operation as its replay receipt.
It reuses the document read/transaction/error boundaries, not request or board receipt
tables. The adapter's `image` module admits bounded PNG pixels and normalizes uploads;
`snapshot/image` stores one immutable attachment with pixel-equivalent retries.
Reads revalidate pixels without re-encoding the original stored bytes. Image admission
does not attest scene semantics. The local HTTP module owns exact typed resource
routing shared with body-budget selection; image writes reuse the existing Origin
policy with a PNG content type. The browser snapshot state owns capture/image retry,
reusing the document painter and the local runtime's shared request lifetime;
the view owns only form state and disposable preview URLs. Native snapshot access
uses the same repository through `tmt-office-storage::access::whiteboard` in the verified
companion, with exact per-operation JSON/PNG limits. `office_companion::whiteboard`
validates replies over its parent's existing bounded process owner. The CLI owns
the explicit export path, while `office_whiteboard::export` publishes a private,
no-clobber file. The core snapshot module owns local reference identity; browser
formatting/resolution shares conformance vectors.
The local [request dispatch capability](extensions/tmt-office/contracts/dispatch-v1.md) composes
explicit recipients over `RequestService::enqueue`. Core `dispatch` owns
composition values, its adapter owns JSON/digests, and `storage::dispatch`
owns an immutable acceptance ledger in schema 23. `storage::requests` lends its
existing row adapter through `TransactionRequests` inside the caller's transaction;
there is no nested transaction or parallel request SQL/state. One operation and
all accepted/failed recipient attempts commit together. The HTTP adapter retains
the existing browser authority and settings/connection owners. `LocalRuntime.dispatch`
uses the shared authenticated transport and validates receipt operation/audience.
New single-recipient requests can claim one advisory wake on the canonical inbox
attempt after durable acceptance. The loopback adapter delegates to the shared
`tmt-adapters::delivery` composition used by talk and reply hints. Core routing
policy and runtime/host drivers verify recorded session state and endpoint evidence;
an Ended shell stays queued, while a verified replacement can recover through
the existing session CAS. It sends only a request-ID and accepted-recipient-UUID
instruction. The claim
prevents automatic replay after uncertain pane input; wake metadata never
changes the immutable receipt or queued delivery state. Roster sends and
announcements do not wake panes.
The shared `local/dispatch-composer-state` owns frozen message/audience intent and
explicit retries. Whiteboard `snapshot-send-state` adds immutable reference/message
formatting; the broadcaster selects announcement semantics. Capture/image state
does not own sends.
Direct `local/agent-conversation` composes that same state with canonical request
history. `use-agent-conversation` owns one retained non-modal Chat/Info HUD;
`conversation-cue` derives waiting/reply attention from canonical history with a
view-local seen marker, never a stored acknowledgment or execution state.
`conversation-state` owns bounded visible-only observation; the optional
scope-checked `dispatch-journal` keeps only unconfirmed intent in tab session
storage, separating direct recipient/context keys from room-roster keys.
`local/room-message` retains one room composer with explicit roster adoption,
review and guarded target switching; it uses the same composer and receipt recovery.
Accepted history and replies remain host-owned; credentials are never
written to the journal. Receipt lookup is read-only and retries preserve the
original operation and audience.
Discussion `local/board-share` supplies a live thread UUID and native reader
instruction to the same composer; it does not create snapshot storage or another
reference parser. The board retains its content/draft owner while the request
view freezes the selected thread and requires explicit discard before leaving.
The [meeting-room resource](extensions/tmt-office/contracts/meeting-room-v1.md) owns explicit local
rosters in schema 24. Fan-out reads its effective membership inside the existing
enqueue transaction and fences both room revision and UUID audience, after replay
lookup. Retirement filters active projections without changing core identity hooks.
Browser `RoomPicker` uses the shared local port and `IdentityChecklist`; refreshing
a list cannot expand frozen intent. Canonical `RequestKind` distinguishes replyable
requests from inbox-only announcements (schema 25); the request service owns
no-response policy, incoming attention and settlement. Dispatch includes kind in
intent comparison without changing existing request digests. The bundled broadcaster
opens the shared composer through the guarded extension binding; opening never
selects an audience or sends automatically. Physical meeting areas reference these
room UUIDs; `office-population` projects memberships without duplicating identities.
The same `RoomPicker` manages room definitions independently of a layout draft.
Its shared `RoomEditor` owns revision-fenced writes and explicit readback after
uncertain saves. Adopting a readback is an explicit action, never an automatic
overwrite or another room creation. The world-anchored `MeetingCreationForm`
reuses that editor, retaining a confirmed room UUID and proposed area ID across
placement failures. `meeting-module` attaches the room through the existing
world history and furniture recipe; layout Undo never deletes the room.
`world-map/meeting-preset` adds ordinary placements/resource bindings to that draft;
it does not create rooms, whiteboard content or requests. Browser authoring and
actor preview placement share the sparse `world-map/free-floor` interval owner.
Derived actor slots prefer clear views using `world-geometry`'s existing wall
projection and paint depth, then fall back to safe floor when an area is crowded.
This preference changes neither saved positions nor membership and is cached per geometry.
Core `office_whiteboard::document::empty_document` describes a virtual blank for
any admitted unsaved document ID. Storage reads do not materialize it; explicit
conditional Save remains the only content creation path. The Lobby has no special
storage branch, and snapshot capture still requires a persisted document.
CLI `room send` and `room broadcast` use that same atomic composition owner,
returning immutable per-recipient inbox acceptance without waiting for replies.
The trusted CLI adapter resolves optional sender provenance through the existing
identity context; HTTP admission still rejects caller-selected senders. Known
sender provenance participates in intent hashing and canonical request attention.
Unknown-sender HTTP digests and the historical ledger table name remain unchanged.
An explicit operation UUID permits identical-intent retry; a changed room roster
is a conflict, never permission to enqueue a new audience under the old UUID.
Core `operation` owns UUID generation for both board mutation and dispatch retry
identities. Direct `talk --room` selects only its named recipient; it never fans
out. The shared request service verifies effective membership within preparation's
transaction through `RequestRecords`, backed by the existing room reader. This
applies to inbox enqueue and pane preparation, before cadence or attention writes;
CLI preflight alone is not treated as an atomic membership fence.
Core `room::RoomRepository` is the shared CLI/HTTP roster boundary. Its resolver
accepts canonical UUIDs or unique exact labels and rejects ambiguous names. The
resolver depends only on `ActiveRoomReader`, which every repository provides and
which storage also provides inside a caller-owned read transaction.
Adapter `room` owns the wire projection used by both transports; storage table
names remain unchanged. CLI `room` creation/list/show/join/leave requires no Office
installation; explicit identity selection does not probe tmux. `ls --room` filters
the existing presence projection rather than introducing another presence owner.
Office preserves that projection's `active`, `offline`, and `unknown` states
through HTTP and UI; self-reported status and roster membership do not override it.
Atomic membership set changes reuse the same `storage::room` writer and
immediate transaction as conditional roster replacement; callers do not perform
an unlocked read-modify-write. Room retirement is a revision-checked transition
in that same row; the repository separates active selection from historical UUID
lookup. Dispatch and new spatial bindings use active selection, while committed
receipts, delivered requests and retained areas remain intact. CLI and HTTP reuse
the transition; no archive database or cascading content deletion is introduced.
Scoped delivery and spatial integration are defined
in [rooms and walls](extensions/tmt-office/contracts/rooms-and-walls.md), not implemented by roster
commands alone.
The local [map v1 foundation](extensions/tmt-office/contracts/map-v1.md) separates topology from
resource contents. `tmt-office-model::office_map` owns native admission and derived walls.
Its `modules` owner projects fixed room slots and circulation into that same
admission boundary. [Versioned modular topology](extensions/tmt-office/contracts/modules-v2.md) stores
the module source only; the admitted map's immutable floor/edge projection is
not another write model. The map codec preserves v1 values until explicit
conversion; v2 retains short links and v3 derives continuous grid
corridors. V4 adds a 2×2 Lobby and a bounded public lattice independent of paired
rooms. Its row-run generator excludes private interiors and the reserved meeting
wing; centered entrances are derived from adjacent public floor. Old geometry
remains readable. `world-map/module-upgrade` converts v2/v3 into V4 as a single
draft: offices south of the Lobby move one row with their interior contents,
the Lobby's south mounts follow its enlarged boundary, and meetings stay fixed.
Area IDs, assignments, materials and resource attachments remain unchanged.
Ambiguous corridor/exterior objects block conversion without mutation.
V6 adds an explicit platform preview through the same relocation boundary:
cardinal office/Lobby neighbors use centered links, with traversal through
intermediate platforms rather than perimeter bypasses. Meeting pods branch from
an independent spine. Both native
and browser module projection own the topology; rendering does not invent paths.
Stored v4/v5 geometry remains unchanged until explicit conversion.
V7 retains v6 room positions and personal-office bridges but omits all meeting
circulation. Only its module admission permits separate meeting components;
personal/common floor still requires Lobby reachability, and freeform admission
is unchanged. Explicit conversion checks retained placement support before the
existing history/auto-apply write, preserving IDs, bindings and object order.
V8 separates grid location from area use. Non-Lobby platforms share cardinal
neighbor connections regardless of personal/meeting binding; disconnected
platforms are allowed, but each area and its generated common floor remain
internally accessible. The historical `office` slot tag denotes a grid coordinate,
not a restriction to personal use. Explicit conversion aligns old meeting slots
with their room-relative objects; subsequent use changes touch only the binding.
The platform draft converts mounted objects into floor decorations while keeping
their IDs, artwork and resource bindings. Ownerless or oversized objects reject
the draft without mutating the source. Undo and Cancel retain the original value.
`world-map/freeform-upgrade` proposes v1 modules through the same eligible-slot
policy: one primary Lobby, personal areas near their previous relative positions,
and a separate ordered meeting wing. It preserves area IDs and bindings. Shared
object relocation checks complete source support, including holes, and rejects
contents that cannot fit the destination. Empty areas and extra Lobbies require
explicit resolution; no rooms, objects or attachments are silently discarded.
Upgrading is never a bare version toggle or an
implicit repacking operation. It is an explicit draft change validated by the normal Save,
not a read-time migration. Browser
`world-map/map-source` decodes that union and caches the read-only projection used
by rendering, population and discovery. Existing world history and revisioned
Save retain source modules; modular drafts cannot call the legacy floor writer.
`world-map/module-authoring` offers unoccupied cardinal office slots using those
same bounds/reserved-wing rules. Choosing a hologram only selects a slot; naming
and adding commits a module through the existing world history. The browser has
no edit-mode gate: selection drives the context inspector, object drags commit
once, and property changes enter the same serialized auto-apply queue. The queue
retains history and newer edits across acknowledgements, pauses on write failure,
and never rebases or retries an uncertain write implicitly. Native revision and
placement admission remain authoritative.
`rendering/scene-module-ghost` is disposable presentation of that slot, while
`office-expansion-form` supplies anchored text entry; the directory exposes the
same eligible slots for keyboard selection without a separate build mode.
The renderer projects the complete hologram bounds through `selection-anchor`;
the shared anchored-panel hook measures the form and actual context-panel
clearance, placing it beside that target within the HUD-safe viewport. Creation
forms retain the general inspector's state.
The shared projector still supplies existing point anchors
for actor and furniture controls. Panel measurement is disposable presentation,
not another camera or layout state. Its resize observer is released on unmount.
Module removal uses the same source/history owner. Its preview checks all retained
placements against candidate spatial support, including disappearing common floor
and partitions; blocked placements must be moved or explicitly removed first.
It never mutates canonical identities, membership or linked resources. Native Save
still owns full connectivity and content admission. V4 meeting expansion uses
the same wing descriptors for the saved topology and cyan construction preview;
room membership remains with the targeted room manager. The agent Info panel's
Add to meeting entry seeds that manager's existing `RoomEditor` draft with one
candidate; it does not write or dispatch. Conditional roster Save retains other
members, and an unsaved draft fences both room and candidate switching.
The native `office_world::starter` supplies a furnished v8 platform Lobby and four
unassigned offices only when neither a saved world layout nor retained blocks
exist. Stable placement IDs and bundled resource bindings remain read-only until
explicit Save; the existing revision-zero source fingerprint fences that Save.
Saved layouts never reseed. Explicit conversion is separate from this initializer;
legacy layouts retain object editing and explicit area removal for conversion
repair, but no floor painting, zoning or manual door authoring. The new-world
preset is not a migration of existing content. New objects use floor support;
the initializer does not create wall-mounted lights, windows or decorations.
`world-map/module-geometry` derives the shared connection descriptors used by
floor projection and portal presentation. Two physical thresholds remain in the
admitted map; v2 rendering paints one frame per short passage. V3 corridors
separate the physical thresholds and expose their shared floor.
`tmt-office-model::codec::office_map` owns its strict codec. Browser `world-map` owns bounded
draft editing and disposable rendering projection, not save authority. Literal
vectors cover shared geometry and intentional draft/admission differences.
Browser `rendering/world-projection` separates saved ground coordinates from
cutaway scene coordinates. Floors contract in depth, upright artwork keeps
its proportions, and object picking/dragging uses the same forward/inverse
transform. V3 retains expanded inter-row display gaps. V4 reserves rear-wall
space inside each derived module instead: public circulation stays unexpanded,
and the admitted floor index identifies the owner for room-floor and wall/mount
projection. Geometry owns this map-specific transform instance, including ghost
placement, Fit and HUD anchors. Upright artwork is never stretched with the floor.
It owns no persisted layout or placement state.
V6 removes the wall reserve: rooms and bridges share one projected floor plane.
Closed boundaries paint thin platform trim and downward front-edge thickness;
open boundaries have no door art. Flat construction ghosts use the same projection.
The v6 platform shell paints beneath upright content, allowing supported furniture
art to overhang a rim without being sliced by it. Content retains its existing
depth and saved stacking order; physical base admission is independent of paint.
The following cutaway wall rendering rules apply to retained pre-v6 layouts.
`world-map/floor-index` provides sparse row ownership queries for both boundary
projection and extension discovery; it does not allocate a second per-tile map or
persist object-area membership. Wall discovery and placement suggestions use the
same mounted-face interior tile convention.
`rendering/world-geometry` derives cutaway bounds and wall/mount paint depth from
those boundaries. Front corner posts derive from owned side-wall endpoints;
door splits never create duplicate posts. `world-projection` owns one room-wall
rise and a distinct circulation-rail rise shared by drawing and previews.
Room portals retain the same rise as their walls.
The existing editor lifetime controls wall translucency in `scene-wall`; entering
or leaving editing redraws presentation without changing geometry or hit testing.
The bundled architecture atlas supplies reviewed frame views
through `scene-materials`; front and rear share one straight-wall frame and
nine-slice crown/base definition, with cutaway height owned by geometry.
It cannot introduce independent topology or placement
state. Texture views share one mount-owned source and are disposed before it.
`editor/snapshot-history` supplies bounded undo/redo to whiteboard and pixel drafts.
The production layout uses `world-map/world-yjs`: one mounted Y.Doc, entity-keyed
values and a local-origin Y.UndoManager. `WorldYjsDocument` exclusively owns raw
shared types, typed cells, detached snapshots and prevalidated batch writes.
Yjs transactions batch observation, not rollback; native commit admission remains
separate. Confirmed clean native observations do not
enter the user's undo stack or erase it; an externally replaced entity is not
overwritten by its older local inverse. History is session-local, with an explicit
update-byte-budget checkpoint, not stored in SQLite. Domain decoders still admit
projections. `use-world-editor` retains the existing serialized JSON/CAS persistence
and pauses on conflicts; Yjs adds no provider, remote authority or second database.
See [Office architecture](docs/office/architecture.md) for history lifecycle and limits.
The [world value foundation](extensions/tmt-office/contracts/world-v1.md) composes that map with
stable placement IDs. `tmt-office-model::office_world` validates floor/wall support,
door clearance and window exclusions over the map index. Shared prop appearance
admission is independent of the legacy 32x32 bounds; signed positions support
world coordinates without loosening legacy block validity. The world adapter
composes the existing typed map and prop codecs. `tmt-office-storage::office_world` persists
the all-or-error candidate in the existing world row (schema 28), checking revision
and artwork within one immediate transaction after preflighting identity/room eligibility. Before
explicit cutover, retained blocks have a read-only deterministic projection fenced
by a source fingerprint. First Save retires those rows atomically; schema triggers
prevent renewed block writes. `office layout show/apply` and the world HTTP route
share `tmt-office-storage::access::world` for storage execution and public diagnostics; strict
companion decoding and bounded file acquisition remain adapter responsibilities.
The CLI has no local block alias. Old local block HTTP/private companion operations
and browser port are removed; legacy native/browser scenario fixtures still need
conversion, not a compatibility wrapper or competing layout writer. The world contract owns
the migration and uncertain-save behavior; resources keep their existing owners.
`office_extension::ResourceBinding` owns pure resource-reference validity; the
adapter reuses its codec for preflight and world attachments. Former bundled
functional entries become ordinary placements, never content copies.
External links extend that binding with an inert URL, not a new placement action
store. Core admission uses the workspace-pinned `url` parser for pure syntax and
credential checks; the reviewed core dependency policy permits parsing, not HTTP
clients. Browser admission shares literal vectors and uses its platform parser.
The guarded `link.open` handler opens a destination review, never a URL itself;
only the review's explicit no-opener anchor navigates. Neither storage nor rendering
fetches links, and artwork remains independent of the action.
`local_service/world` and the browser world port use the existing authenticated,
bounded transport. Browser `world-draft` supplies pure changes to the Yjs-owned
layout, whose admitted projection feeds the editor and renderer. Surface controls change the same placement,
not a second wall layout; resource bindings survive moves and unmounting.
The wall collection is an ordinary immutable indexed prop pack. Native and browser
catalogs admit the same bytes; windows, lights and decorations share art resolution
and missing-art fallback. A wall light adds a static renderer-owned glow, not a
shader/runtime capability. Browser placement suggestions inspect derived boundaries
and occupied silhouettes, but never authorize Save or move other objects. Numeric
coordinates and appearance text stay in local input forms until one complete edit
enters world history. Shared prop customization controls serve both editor callers.
Schema 26 retains an optional original room UUID on canonical request attempts.
Shared dispatch distinguishes single-recipient room context from reviewed full-roster
fan-out. Only fan-out checks the roster revision; canonical `RequestService` checks
recipient membership for both inside the enqueue transaction. The tagged mode is
part of immutable retry intent, not another delivery path. Dispatch copies its
room UUID into `PrepareRequest`; it does
not make the dispatch ledger a second context store. Shared row projection and
attention models carry that value through detail and incoming results. Room-scoped
listen filters the watermark and both incoming queries in the same request owner,
using participant/room indexes before pagination. The CLI resolves the room once;
subsequent roster changes do not hide already-delivered work. Unscoped requests
and JSON retain their existing behavior. Room lifecycle cannot cascade into
request retention, and transport adapters do not infer historical membership.
Schema 27 adds indexed keyset history over those same attempts, not chat storage.
`request::history` owns the owner-visible projection, and its service composes
retention and the existing attention final-state interpretation. Storage reuses
the canonical attempt/response row decoders; bounded UTF-8 previews preserve
embedded NUL without loading full message bodies into lists. The `request_history`
adapter admits/encodes the owner API without reply proofs or pane paths. HTTP
inspection requires the same bearer/Origin admission as dispatch. Operation lookup
and dispatch replay share the existing immutable ledger decoder; lookup cannot
resubmit. Browser `LocalRuntime.requests` owns only bounded typed transport and
response-scope checks, not another request cache or completion policy.
The [workshop references](docs/office/references/workshop/README.md)
own visual intent, not evidence that proposed extension APIs are implemented.
Its browser E2E may reuse the established test-only process and artifact owners.
The pairing issuer is implemented for local emulator verification and disabled
by default outside that environment; it is not deployed. `extensions/tmt-office/contracts`
owns the versioned work-handoff schema and fixtures; derived representations must
prove conformance there. Structural tests do not prove remote authorization or
delivery. Future connector dispatch reuses native request/storage ownership,
not CLI-output scraping or a competing exchange engine. Ordinary CLI operations
remain independent of Office.

Office has app-owned boundaries: `auth` initializes Firebase/session,
`worlds` owns admission and world access, and `blocks` owns the layout contract,
codec, adapter and editor lifecycle. `pairing` owns public-link decoding, explicit
owner approval/revocation and sanitized action state, reusing the selected-world
lifecycle and authenticated runtime composition. `spaces` projects bounded
owner-only grant pages and selects the existing block editor; it has no
assignment registry or permission mutation. Rules and the trusted issuer
enforce authority; views never grant it. Remote snapshots have one owner, separate
from unsaved drafts and ephemeral
presentation state. No stored markup executes and no parallel layout is stored.
Native decoration uses `tmt-office-model::office_block` for pure layout validation and
codec conformance, `tmt-office-model::codec::office_block` for readable JSON, and the existing
paired companion for authenticated conditional Firestore commits. Browser and
native implementations share the versioned block contract and literal vectors;
neither creates a second scene store. The Office command library owns Office
grammar and presentation, not credentials, grant renewal or Firestore transactions.
The offline local Office path is separate from the Firebase runtime. Both one-shot CLI
layout commands and the loopback HTTP service call the same whole-world access boundary in
`tmt-adapters`; neither mirrors state into the SPA. `tmt-office` embeds the Vite local
build at compile time, so the fixed native archive inventory does not gain mutable web
files. A private receipt coordinates one installation-wide process. Browser and control
tokens are distinct, status is token-free, and only exact IPv4 loopback Host/Origin
requests reach the bounded HTTP adapter. Manual area bindings use identity UUIDs;
retirement does not erase stored placements or linked content.
The local overview and identity deep links select the same whole-world loader,
editor and mount-owned Pixi renderer. React owns browse panels independently of
selection and the world draft. Selecting directory, area or object controls suspends
the retained agent session. Chat/Info share one recipient/context; closing or
selecting layout content pauses observation without cancelling work. `WorldTools`
owns the shared right-hand inspector slot, while `use-agent-conversation` owns
draft retention and request recovery, independent of camera position. There is
no floating or minimized agent window. Hidden details retain unsaved appearance edits;
changing panels never resizes the canvas. The HUD uses one viewport overlay
grid for the header and a right-hand inspector with auto-apply status and Undo/Redo.
There is no layout edit mode or manual Save/Cancel. Selection reveals contextual
controls; agent selection replaces layout controls with Info/Chat, and no selection
reveals the furniture library. Creation cards measure the inspector's viewport
boundary rather than reserving a bottom save bar. Directory and room management
use a collapsed Office menu. Camera controls remain
owned by the mounted canvas and portal into one stable top-line dock.
The header and camera wrap together without fixed-height offsets;
neither docking nor error feedback rebuilds the scene or reserves physical canvas space.
V6 platform shells use a shared fixed-scale mechanical sprite kit, owned by
`platform-art` and `scene-platform`. Repeated hardware and selection contours are
derived presentation; module topology, bridge openings and persistence remain
owned by the existing map geometry. The renderer separates ground-level area/actor
selection from foreground object handles so selection never repaints over upright
art or nameplates. See the Office architecture for texture lifetime and selection accents.
Bridge decking uses a fixed metal-panel scale, not the room floor's wood repeat;
`platform-projection` expands short empty bands to the single 24-unit connector
span while preserving room interiors and the Lobby origin. The invertible display
transform is shared by bounds, thresholds, ghosts, picking and dragging; it does
not change stored topology. V7 meeting islands use a separate fixed-slot transform
in their reserved wing: equal visible gaps include vacant slots, and adding or
removing an island cannot alter the campus transform or another island's position.
V8 replaces occupancy-dependent spacing with one fixed, invertible lattice for
all uses and empty slots. The Lobby spans two cells on each axis; its continuous
floor includes the intervening bands. Adding/removing a neighbor cannot shift
existing scene coordinates. Meeting use selects a violet lamp-inset texture and
a pixel nameplate icon, never a different platform geometry or selection color.
Longer routed circulation is not shortened. Blue-green
support bases paint below all bridge deck runs, before room floors and brass trim.
Brass rails are centered on each edge; the deck repeat excludes authored side
seams. Deterministic alloy tones, rivets and service grilles are baked into the
shared deck texture once at load. Brass threshold sprites cover both axes of real
openings, with static layered warm light spilling over the dock rather than hidden
behind its opaque artwork. Blue-green girders sit outboard and below the brass
rails, using long panels and platform-end attachment shoes rather than repeated
rail-like saddles. `bridge-pulse` owns one 20 Hz clock for visible threshold
glows: a five-second cycle changes only halo scale and opacity, not the lamp sprite.
It invalidates the shared frame scheduler without rebuilding scene geometry or
using blur filters. Hidden tabs, reduced-motion preferences, an empty visible-light
set and disposal stop the clock. This decorative activity means a visible
lit scene is no longer completely idle; camera and input still use demand-driven
frames. Drag feedback
uses `world-object-placement::placementProblem` against the existing geometry index.
Invalid drops are red and never enter history or the save queue. Native admission
remains authoritative; ordinary floor layering remains allowed.
Same-runtime refresh retains the mounted workspace and its drafts, reports read
failure in place, and fences late reads from replaced runtimes. The world editor adopts refreshed saved snapshots only
when clean and idle, without clearing selective undo history or rolling back a confirmed revision. The world
port distinguishes a confirmed revision rejection from an
unconfirmed write. Area-removal previews consume the same population projection
and physical object-anchor lookup as browsing, not separate ownership state.
Physical viewport changes preserve the viewed world center and relative zoom.
Remote block
views retain their separate SVG/editor path. Neither renderer owns persistence.
Sparse world geometry supplies floor/wall object bounds for artwork, selection,
culling and interaction; object dragging inverts that projection before editing
stored coordinates. The component overlay consumes those bounds and inert action
metadata, not a competing placement format. It owns measured action-label bounds
and matching paint/hit order. Rendering is invalidation-driven with bounded pixel
density and cancellation/teardown of browser and GPU resources.
Long wall faces retain their original bounds and material phase; repeated seam
and crown detail is generated only across the rendered viewport. Camera movement
must not create artificial wall ends or tessellate offscreen detail along an
otherwise visible wall run.
Fixed architectural materials share one decoded source per mount with bounded,
lazy finish variants owned by `scene-materials`; they do not enter the editable
prop catalog or occupy saved floor tiles. Module source selects the finish through
the existing world draft, without changing geometry or resource bindings.
The directional prop format extends the existing catalog and raster
projection, not the scene state owner; see the versioned
[prop contract](extensions/tmt-office/contracts/prop-pack-v2.md). Prop-specific byte budgets and
schema 18 do not change avatar admission or unrelated command envelopes.
Reviewed modular source art is encoded offline into the same immutable v2 prop
packs; both native and browser registries admit those exact contract bytes.
The optional `scripts/art` authoring tool is not a runtime decoder or validator.
Its source-hashed crop manifest and derivative policy live with the visual package.
`props/furniture-upgrades` maps reviewed static furniture to compatible directional
successors only during authoring. It is not a render-time alias: retained digests
resolve unchanged, and loading a layout never rewrites art. A completed rotation
changes the art reference and placement in one existing Yjs/CAS edit, so Undo
restores both. Native admission still validates the exact successor pack and
footprint. The library suppresses a superseded card only when its compatible
successor is present in the observed catalog.
The library excludes the retired `Legacy pixel basics` pack from authoring and
search. Its immutable resolver remains available for saved placements; opening
the library never migrates, deletes or replaces objects in a world.
World floor surfaces may carry a bounded physical `base` inside the unrotated
artwork envelope. Core world admission and browser `world-map/object-base` own
its quarter-turn geometry; `furniture-base` supplies authoring recipes only on
explicit edits. This is world placement data, not a rewrite of immutable art.
Scene projection, culling and picking keep the full artwork bounds. The scene
passes one complete placement candidate to the existing world editor so base,
position and rotation cannot commit as separate history entries. See the
[world contract](extensions/tmt-office/contracts/world-v1.md) for support and compatibility rules.
`world-object-placement` owns bundled wall-authoring hints used by both library
grouping and initial kind/mount selection. These hints grant no capability or
placement authority; arbitrary admitted artwork still uses the same world validation.
Legacy local blocks are retained migration input, not live HTTP write targets.
The whole-world draft owns per-placement tint/text and its CAS commit; pack
capabilities remain the source of allowed fields. Browser artwork authoring uses
the existing native prop validator and catalog install transaction through the
local props adapter. Exact source bytes determine immutable identity; artwork
Save never writes a world placement. The typed browser port verifies digest and
revision receipts and resolves saved packs on demand. `pixel-canvas` commits one
completed pointer stroke or keyboard edit into shared bounded snapshot history;
`use-pixel-catalog` owns catalog observations and frozen save/retry intent.
`pixel-workshop` composes drawing, existing indexed previews and on-demand library
selection. Both library art and built-in furniture enter the same world-draft
placement action; catalog Save and layout Save remain separate transactions.
Shared prop resolution and frame projection feed both renderers, with value-aware
texture keys. Authoring warnings inspect admitted packs without replacing strict
admission or granting executable capabilities.
Owner approval may select a revoked grant's retained block through the same
bounded space projection. The pairing transaction reserves that source grant
with a transfer receipt and records immutable approval intent; no second
assignment registry or resource copy is introduced.
The detailed lifecycle and verification map lives only in
[Office architecture](docs/office/architecture.md); exact persisted data belongs
in [Office contracts](extensions/tmt-office/contracts/README.md).

`extensions/tmt-office/typescript/services/office` owns isolated emulator infrastructure, Rules and the trusted
pairing issuer under `functions/`, not a deployed backend. Admin operations
bypass Rules: the issuer explicitly checks verified human authentication, live
admission, ownership and grant authority in its transaction owner. Signing stays
outside transactions. Rules enforce the issued grant using the existing UUID
block validator. The native companion consumes this issuer through its optional
Office adapter feature. See
[pairing v1](extensions/tmt-office/contracts/pairing-v1.md) for approval/retry semantics and the
agent-grant contract for resource leases. Owner-local configuration stays outside Git and Docker. Native tmux,
Office browser/Rules and bootstrap smoke proofs retain separate fixture owners.
Installation-local data-only prop and avatar packs are implemented under separate bounded
contracts below. They share only reviewed indexed-art, framed-digest, cursor and preview
mechanics; each retains typed validation, storage tables, revision/cursor domain and quotas.
Profiles may select admitted avatar art through immutable digest/key references; catalog
removal leaves the reference intact and falls back to the stored default appearance.
Avatar built-ins use the same validated native registry for list/show, profile
admission and the authenticated browser catalog. They do not seed database rows
or consume retained-pack quotas; custom catalog revisions remain storage-owned.
Community exchange and exploration remain a [sandbox plan](docs/office/sandbox.md), not a
runtime SDK, identity registry or alternate exchange engine.

`typescript/scripts/ci-scope.mjs` owns conservative affected-area selection and final gate
validation. Office-only source/docs avoid native matrices; native source/skill
changes avoid Office. Shared or unknown paths (including lockfiles, security,
contracts, workflows and test tooling) fan out. Empty diffs fail closed to both.
Diffs include deletions and both sides of renames. Existing required check names
remain; `Code quality` gates selected Office verification and `Native package
matrix` gates all selected native jobs. Selected skipped, cancelled or failed
jobs cannot satisfy either gate. No passing zero-test configuration is allowed.

## Runtime layers

The Rust crates have deliberately narrow responsibilities:

| Layer             | Owner                           | Responsibility                                                                                                                                                                                                  |
| ----------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure domain       | `rust/crates/tmt-core/src/`     | Identity, names, bindings, profiles, settings, retention, request state and native-install version policy. No filesystem, process, SQLite, tmux, network or CLI framework.                                      |
| Concrete adapters | `rust/crates/tmt-adapters/src/` | Config files, SQLite, bounded files and processes, signals, tmux evidence/transport, response input, HTTP acquisition, native release publication and managed skill files.                                      |
| Application/CLI   | `rust/crates/tmt-cli/src/`      | Core grammar, typed invocations, preflight and use-case composition, completion and the executable entry point. It chooses adapters; it does not duplicate their storage, file, installation or process policy. |

`rust/crates/tmt-command-output` owns shared command output/error values and
formatting. `extensions/tmt-office/rust/tmt-office-command` owns the public Office
grammar, typed requests and handlers. Core's reserved `tmt office` facade mounts
that grammar and calls the same handlers through an in-process `CoreAccess` port.
`tmt-cli/src/office_facade.rs` is the sole core registration and command-library
dependency; grammar, translation and invocation types are pure forwards through it;
direct `tmt-office` uses a bounded process implementation over public core JSON
commands. Both entry points keep the existing public Office command behavior.

`rust/crates/tmt-cli/tests/architecture.rs` is a test-only import and
dependency guard. It follows the actual Rust module tree, checks reviewed
layer edges and shared declaration ownership, and fails closed for unsupported
module remapping or incomplete discovery. It is a syntactic guard and never
replaces review of behavior or effects.

The optional `extensions/tmt-office/rust/tmt-office` executable remains a member
of the `rust/` Cargo workspace, with the same lockfile and `rust/target` output.
This package owns the companion entry point, embedded SPA and local HTTP service.
`extensions/tmt-office/rust/tmt-office-model` owns Office domain values, strict
codecs, immutable catalogs and data-only admission. It depends on core identity
syntax, numeric limits and content digests, never on Storage or runtime adapters.
Retained Storage imports that owner directly; there are no core Office re-exports.
Codecs own in-memory PNG processing; filesystem reads, publication, storage response
projection and process/config access remain adapters. Acquisition errors may retain
an `io::Error` value without giving the model an I/O operation.

`extensions/tmt-office/rust/tmt-office-storage` owns Office storage at
`<global>/office/office.db` (#353). Its schema v1 repeats the core schema 35
definitions of the 14 Office-owned tables, so migration copies raw cells, minus
the two `identities(id)` references that Office replaces with preflight. It adds
an Office-local retired-identity marker and a migration record; schema v2 adds the
activation marker written when `office.db` becomes authoritative, and v3 adds
the retired-room marker. Switched storage from an earlier Office schema upgrades
when opened. The crate reaches
core only through public owners: `config`, `file_lock`, `Storage::open` and the
`StorageError` type with its `classify` mapping. It owns the Office repositories
(worlds and legacy blocks, profiles, prop and avatar catalogs, the discussion board
and whiteboards) on `OfficeStore`, and the `access` operations shared by the
one-shot companion protocol and the local HTTP service. Production opens go
through `OfficeStore::open_configured(&StorageLayout)`, which reads core's cutover
receipt through `CoreReferences::storage_cutover`: without a receipt it opens the
shared core database file through `Storage::open`, so creation and migration are
unchanged; with one it opens `office.db`, finishing an interrupted activation
first and failing with the recovery error rather than recreating lost storage.
The single-file constructor is test-only, so no production path can bypass a
switched store. The CLI side keeps only file readers, reply
decoders and wire limits in `tmt-adapters`; the core `tmt office` facade still
reaches storage only by invoking the installed companion.

Office reads core-owned identities and rooms only through the UUID-keyed
`tmt-office-storage::core_references::CoreReferences` port (`identity`,
`active_identities`, `room`), separate from the CLI selector port `CoreAccess`.
Each write preflights its references and then commits in its own Office
transaction; no Office SQL names a core table outside the migration modules, and a
crate test enforces that. This accepts a window: a reference retired between
preflight and commit leaves exactly the state of the legal serial order "Office
commit, then retirement", because identities are never deleted, rooms are retired
rather than removed, and core retirement never changes Office rows. References
that are already retired or missing at preflight keep their existing errors.

Retirement reaches pairing through the `office_pairing::RetirementFence` port,
implemented by `tmt-office-storage::retirement` and injected by the companion. An
identity is fenced when Office's `office_retired_identities` marker in `office.db`
records it or core reports it retired (before the switch only core decides, and
marking changes nothing). The durable hook consumer lives in the companion
(`tmt-office::retirement_consumer`): it reads its pending deliveries, records each
attempt and acknowledges through the consumer-scoped `tmt api` hook operations,
and settles each delivery with `office_pairing::settle_scope` under the pairing
scope lock in order: mark, revoke, then acknowledge, so an interruption leaves the
hook pending and a retry repeats only idempotent steps; a revoked record stays as
the secret-free receipt, so a retry repeats no remote work. `office_pairing`
never opens core storage (a test enforces it): active-identity checks and hook
registration go through the `PairingCore` port, which the companion implements
with `tmt --json identity show -- <uuid>` (accepting only the exact UUID) and
`identityHooks.register`. Core launches one-shot companion operations with
`TMT_EXECUTABLE` set to itself, so the companion reaches the same `tmt`. Every write that grants or extends pairing authority (pair-begin,
pair-poll's claim reservation and completion, and the refresh and renewal on
inspect and block operations) passes the fence under the same lock; only unpair
and the consumer's own refresh, which reduce authority, are exempt, and a test
pins those sites. Known debt owned by #355: the in-process `CoreStore` behind the
Office repositories' `CoreReferences`, the migration coordinator's direct core
reads and receipt write, the `office_pairing` module still living in
`tmt-adapters`, and removing the retained Office rows and fences from core. An
independently versioned Office binary must eventually reach core only through
public commands and `tmt api`, never opening or migrating the core database itself.

Reconciliation v1 (`tmt-office-storage::reconciliation`) compares every identity
and room UUID Office stores (local profiles, legacy blocks, world personal and
meeting areas, board identity authors and room categories) with core through
`CoreReferences`, and records the ones core confirms retired in the monotonic
`office_retired_identities` and `office_retired_rooms` markers of `office.db`
(Office schema v3). It never erases or rewrites content: an unknown UUID is not
retired, and a lookup error stops the run before anything is written, so failure
means "retry", never "deleted". It runs at local-service start and before each
one-shot write command; reads never reconcile, and a failure never blocks the
command (the one-shot protocol keeps stderr empty, so only the service reports
it). Office transactions that write an identity or room reference (profile and
block apply, world save, board post and reply) check the markers inside the same
transaction, closing both orderings: a mark recorded after preflight blocks the
in-flight write, and a write that committed first is found by the next run.
Point-of-use reads apply the markers through `retirement::MarkedReferences`, so
a marked reference reads as retired, never missing, and history stays visible
without granting new authority. Before the switch the shared core file has no
markers and core stays authoritative. There is no queue or continuation: a full
run over 50 identities, 20 rooms and 2,000 board entries takes about 13 ms in a
release build. Add batching only if a run exceeds 100 ms or the inventory
exceeds about 10,000 stored references.

The migration coordinator is the one Office component that opens the core
database for its own reads outside the store: query-only,
without checkpoint-on-close, inside a single read snapshot. `prepare`, `copy` and
`verify` each hold `office/migration.lock` and commit atomically in a private
staging database. The copy preserves storage classes, TEXT and BLOB bytes and
rowids. Verification compares every typed cell against a fresh snapshot; the
manifest digest only detects a changed source. Rooms and the
dispatch ledger remain core-owned even though their tables are named `office_*`.

`migration::switch` makes a verified copy authoritative. Under the migration lock
it stops the local Office service through a `Quiesce` port and holds the service
lock, then writes a verified backup: `VACUUM INTO`
`<global>/backups/office-storage-<UTC stamp>/tmux-team.db` after a free-space check,
checked for integrity, migration history and the verified Office manifest, plus
copies of the global `config.json` and the protected top-level `office/` files
(never `runtime/`, locks or databases). It then opens one immediate core
transaction, recomputes the Office manifest, publishes staging (upgraded to the
current Office schema) as `office.db` with file and directory fsyncs, and inserts
core's `extension_storage_cutovers` receipt. That commit is the single decision
point, and the receipt is the one sanctioned Office write to the core database,
confined by the crate guard to `migration/switch.rs` and removed with the
in-process core link (#355). Core schema 36 triggers then reject every write to the
14 retained Office tables, including from already-open older connections; core
reads the receipt through `Storage::extension_storage_cutover`. Activation writes
the marker and moves `office.db` to WAL. Recovery derives the outcome from the
receipt and the marker: no receipt renames `office.db` back to staging, a receipt
without a marker finishes activation, and a receipt with missing, replaced or
unreadable storage reports `OFFICE_STORAGE_RECOVERY_REQUIRED` naming the newest
backup. Users reach the switch through `tmt office storage migrate`: the
companion's `storage-plan` operation reports the plan without taking locks or
creating files, and `storage-migrate` runs prepare, copy, verify and switch only
when the recomputed plan digest equals the one the user confirmed
(`OFFICE_STORAGE_PLAN_CHANGED` otherwise). The hidden diagnostic entry remains for
disposable roots.

The architecture guard freezes the exact remaining adapter consumer paths in
`office_consumer`; core grammar and parser no longer import the Office model.
New core-to-Office model consumers, command-library edges outside the facade,
dependency aliases, adapter re-exports and
reverse model dependencies are rejected. Office runtime adapters remain retained
extraction debt, not a second implementation or a storage migration.
`tmt_office_command::core_access::CoreAccess` limits Office handlers to identity selection
and historical room lookup. Its in-process implementation delegates verified
caller selection and room resolution to the existing core CLI owners; its direct
entry point invokes `whoami --json`, `identity show --json` and `room show --json`
through the bounded process owner. Handlers do not reopen core storage for these
lookups or duplicate caller policy. The process port preserves child errors except
for two exact-code mappings to existing Office semantics: `whoami`'s
`PANE_NOT_FOUND` becomes `IDENTITY_REQUIRED`, and explicit `identity show`'s
`INVALID_NAME` becomes `NAME_NOT_FOUND`. Neither mapping retries or changes targets.
The executable is independently versioned and
exposes the compatibility probe and typed one-shot pairing/status/inspect/sync operations.
It depends on core and the existing adapters, not the CLI. Its adapter `office`
feature owns validated deployment decoding, bounded HTTP, protected pairing
records and explicit platform credential stores; ordinary CLI builds do not enable
that feature. Serde derives reject duplicate/unknown descriptor fields; the URL
Standard library matches browser URL interpretation instead of introducing a
handwritten parser. Neither dependency enters core. The probe acquires no
credentials or network data. `tmt-office-model::office_protocol` owns the fixed typed
handshake; `tmt-adapters::office_companion` verifies active installation ownership
and starts the existing bounded subprocess under the installer lock, then waits
outside that lock and validates the version selected at launch.
Its contract is [native companion handshake](extensions/tmt-office/contracts/native-companion.md).
Local presentation profiles are a separate UUID-owned resource: `tmt-office-model::office_profile`
owns the literal default catalog, text bounds, optional immutable `avatarRef` grammar and
deterministic default; SQLite schema 15 owns only the canonical override and CAS revision.
Native commands and authenticated loopback HTTP reuse that owner. A profile change to a
different custom reference and its catalog admission are checked in one immediate SQLite
transaction. A retained reference remains editable when its pack is removed or corrupt,
and exact reinstall restores its art without rewriting the profile. Layout, role, notes,
identity and presence are never profile fields. Browser SVG and canvas share the default
`profiles/avatar-art` projection and `profiles/avatar-layout` display metrics; admitted
custom art keeps its immutable raster. The bundled v2 default uses named material
slots and the same palette-tint operation as furniture; these authoring slots do
not add a catalog or installed-pack schema. Avatar v1/v2 preserve their own versioned
digest domains and one-/two-digit encodings within the same file, cell and catalog
budgets. Native `office_avatar::avatar_format` owns versioned admission and summary
dimensions; browser `avatars/avatar-contract::avatarRaster` preserves encoding for
catalog resolution and previews. `rendering/indexed-raster` draws
inert pixels for both avatars and admitted props; avatar artwork never enters the prop
catalog. Identity and shirt text remain separate accessible text overlays, never executable
artwork.
The public `office` subtree composes installation, identity resolution and bounded
pairing observation, never credentials or HTTP. `office_pairing` separates wire
values, remote Auth/resource access, vault access, local installation metadata,
record transitions and one-shot composition. Resource access refreshes Auth and
performs bounded lease renewal separately; only an exact authenticated issuer
readback can replace the local expiry. No timer or background process renews
grants, and local status stays network-free. `office_http` shares bounded JSON
transport with deployment discovery. Existing ConfigPaths, identity storage,
file locks and process owners remain authoritative. One protected scope record
owns pending proof or credentials; SQLite hooks contain only its opaque scope
reference, never a duplicate credential or grant. OS random
bytes create proofs; explicit Keychain/Secret Service backends fail closed.
Background connection and resource editing remain unimplemented.

Explicit `office unpair` reuses the record's reduction-only revocation path and
scope lock. Only a confirmed revoked receipt can be replaced by a new explicit
pair request. Owner cancellation of an unknown public request uses the existing
issuer transaction to write a disabled pairing, never a second cancellation store.
Browser request snapshots share the existing contract decoder across state and
transport; uncertain cancellation prevents switching back to approval.

Identity retirement remains the existing binding transaction's responsibility.
`tmt-core::identity_hooks` owns typed subscriptions and delivery state;
`storage::identity_hooks` registers subscriptions and atomically queues retirement
notifications from that same transaction. It has no Office dependency. Consumers
run after commit: `office_pairing::hooks` uses the existing protected-record and
per-scope lock owners to revoke, retain a secret-free receipt, then acknowledge.
No SQL transaction spans remote work. Office pair/inspect and explicit `office
sync` consume bounded batches; ordinary identity commands only enqueue locally.
No background or punctual remote cleanup is implied. The
[native pairing lifecycle contract](extensions/tmt-office/contracts/native-pairing.md#identity-retirement-hooks)
owns delivery order, retries and compatibility limitations. This is a retirement
hook, not an arbitrary executable event bus.

Workspace quality checks cover the unified feature graph. Native process
fixtures build products separately to retain ordinary CLI feature isolation;
[Development](DEVELOPMENT.md#rust-checks) owns their symbol/profile selection.

## Public command boundary

`rust/crates/tmt-cli/src/grammar.rs` owns core syntax/help/completion and mounts
the Office subtree from `tmt-office-command::grammar`. Each owner's parser turns
its grammar into typed invocations; both publish through `tmt-command-output`.
Hidden commands are still
parsed for controlled internal workflows but are omitted from public help and
completion.

Help retains its selected public command path. The grammar-aware presentation scan
shares option-value boundaries with error-mode recovery, so `-h`/`--help` can bypass
required operands without interpreting payload data as flags. Public help and
completion use command-owned options, not inherited placement-only options. The
public projection preserves command-owned supplemental help instead of adding
per-command presentation branches. Help
for core commands never enters runtime dispatch or skill-drift inspection; JSON
core help remains unsupported.

### External command contract (v1)

An unknown root command named `[a-z0-9][a-z0-9-]*` resolves `tmt-<name>` on
PATH. The CLI reserves every grammar name and alias, including hidden commands;
the core always wins a collision. The adapter owns executable lookup and process
replacement. On Unix it uses `exec`, not the supervised child model used by
`run`: the extension inherits stdin, stdout, stderr, TTY and signal behavior,
and its exit status is the command's exit status. Exec failure is a normal error.
`TMT_EXECUTABLE` is the absolute invoking TMT executable path. There is no registry,
manifest, daemon, implicit install or extension state in core.

Only arguments after the extension name are passed verbatim, including non-UTF-8
arguments; TMT does not interpret their options. Root options before an extension
are rejected with an option-placement hint when the extension exists. A missing
executable retains the existing unknown-command diagnostic and presentation mode.
`tmt help <extension>` executes
`tmt-<extension> --help`. Root help lists discovered extensions and notes executable
names shadowed by core. PATH directory enumeration happens only for root help,
root completion and unknown-command suggestions; exact extension dispatch probes
only the requested filename. Ordinary core commands do not enumerate PATH.

Extensions may ignore completion v1. When offered, TMT calls
`tmt-<name> __complete -- <words after the name, including the current word>`
with `TMT_EXECUTABLE`, a one-second deadline and a 64 KiB combined output bound.
Successful stdout is newline-separated literal UTF-8 candidates, with no tags,
descriptions or version field. Empty output, nonzero exit, invalid UTF-8/NUL,
timeout or excess output falls back to file completion. Shell adapters quote the
literal candidates; they never evaluate them. Extension completion uses the shared
bounded process owner, not the unbounded interactive exec path.

This generic dispatch does not extract the reserved `office` command; Office
domain/storage extraction is tracked separately in #328.

### Local extension API (v1)

`tmt api` is the public, same-user process port for machine-shaped gaps in the
ordinary CLI. One invocation reads one versioned JSON request from stdin and
returns one JSON resource or structured error on stdout. It is neither an
authentication boundary nor a daemon, batch processor or streaming connection.
Extensions use `TMT_EXECUTABLE` rather than assuming an installed binary path.

The CLI owns bounded stdin acquisition (EOF within five seconds), JSON publication
and exit status. `tmt-adapters::api` owns envelope admission and composition;
identity, room, request history, dispatch and notes retain their existing domain,
transaction and resource encoders. The same one-shot delivery helper serves
Office and API dispatch. Explicit identity selects write attribution, not privilege.
Capabilities and unsupported-version discovery never open application storage.
Input and output bounds are advertised in capabilities; canonical content limits
still apply independently of JSON escaping.

Protocol major 1 accepts additive operations and response fields; clients ignore
unknown response fields. Removing operations or incompatible semantics requires
a new major. Unsupported majors return `API_VERSION_UNSUPPORTED` with the supported
range. Human-shaped identity, presence, room inspection/retirement, reply/result
and attention operations remain their ordinary JSON commands, not duplicate API
implementations. This port does not move Office storage or remove its core facade.

`rooms.roster` composes a room's effective members with their prefix-filtered
metadata and self-reported status from one deferred SQLite read snapshot
(`storage::room_roster`): selection, membership, metadata and status cannot
disagree within a response. The read itself performs no writes or acknowledgment;
opening storage follows the same policy as every other API operation.
Presence is deliberately excluded because it requires host observation and
binding reconciliation owned by `ls`; consumers join `ls --room --json`.
Adapter `identity_projection` owns the identity summary and metadata map shared
by CLI JSON and this operation, so both transports emit identical bytes.

`identityHooks.register|pending|attempt|ack` expose core's durable
identity-retirement subscriptions to their consumer. Every operation is scoped to
the named consumer and keeps the storage semantics: registration after retirement
is pending at once, delivered is terminal, and attempts or acknowledgments on
another consumer's hooks return `HOOK_NOT_FOUND`. The API layer reads the hook
state only to report not-found and not-pending distinctly; transitions stay in
`storage::identity_hooks`.

History reads use the existing `(preparedAtMs, requestId)` keyset, not a frozen
snapshot or change feed. X attention retains its separate revision cursor.
Inspection does not acknowledge work or renew retention. Dispatch operation IDs
recover immutable acceptance; replay never wakes again. Clients must recover a
receipt or current room revision after interrupted writes, not invent a new
operation ID and resend. See [extension API usage](docs/extension-api.md).

### Extension hooks (v1)

`tmt-adapters::extension_hooks` owns consented, best-effort lifecycle
observations. PATH discovery alone never runs a hook: `tmt extension hooks
enable <name>` resolves `tmt-<name>`, requires a regular executable owned by the
current user with neither the file nor its directory writable by others, probes
`tmt-<name> __tmt-hooks 1 capabilities` (a `TMT-HOOKS/1` header and a token set,
1 s, 1 KiB), and records the canonical path, SHA-256 digest, a metadata
fingerprint and the capabilities in `<global>/extension-hooks.json` (0600,
replaced atomically). Before each delivery core re-checks ownership and the
fingerprint; any change skips the extension until it is enabled again. The
grammar stays composable under `tmt extension` for installation commands.

Capture is per connection and transactional. When the process allows capture
(only the `tmt` CLI does) and an enabled extension offers
`lifecycle_observations_v1`, `Storage::open` installs temporary triggers that
record typed evidence in a temporary table: `identity.created` and
`identity.retired` (UUID, lifetime, retired) and `room.created`, `room.updated`
and `room.retired` (UUID, revision, retired), never names, messages or payloads.
Temporary tables take part in the transaction, so rolled-back changes leave no
evidence and nothing is persisted. Storage drains the table on close or drop;
after the command the CLI runs `tmt-<name> __tmt-hooks 1 observe` for each
still-verified observer with the events on stdin, through the supervised process
owner under one aggregate 500 ms deadline and 4 KiB output budget. Output is
discarded, and no exit status, timeout or failure changes the command's result;
consumers must converge through their own reconciliation. Every hook call
carries `TMT_HOOK_DELIVERY=1`, and a `tmt` process that sees it captures
nothing, so an extension calling `tmt` cannot cause nested delivery.

Rehydration context (`tmt whoami --context` and provider injection, which
share `context_command::verified_document`) asks each verified extension that
negotiated `context_v1` for one line, only for a verified, bound identity:
`tmt-<name> __tmt-hooks 1 context` with `{"version":1,"identityId":…}` and
`TMT_HOOK_DELIVERY=1`, under one 300 ms deadline (capped by the provider hook's
remaining budget) and 1 KiB of output per extension, bounded before parsing.
Only `{"summary":"…"}` with at most 240 characters is accepted; anything else,
a timeout, or an absent, changed or disabled executable omits that
contribution. The host attributes each `{extension, summary}` by its consent
name. Summaries are untrusted, informational data: text output labels them
`Extension <name> (informational): "…"` with the same escaping as role and
notes, and the 4 KiB bound drops extension contributions before role or notes
and never loses core counts or inspect commands. Unbound, ambiguous and
unavailable callers never invoke extensions. Office answers with a read-only
line about the identity's desk and meeting-area count from its own world
layout, opening both databases read-only and never through
`OfficeStore::open_configured`, so context never migrates, activates,
reconciles or creates files.

With no consent file or no enabled observer, a command performs at most one read
attempt of the consent file, on its first storage open, and spawns nothing;
commands that never open storage do no hook work at all. Office implements the
protocol by running its full reconciliation on `observe`, and stays inactive
until the user enables it.

### Core command surface

The maintained public surface is:

- `init`, `config`, `completion`, `learn` and `install` for local setup and
  guidance;
- identity and binding commands: `identity` create/show/list and metadata
  set/get/list/remove with exact filters, `list`/`ls`, `add`, `name`/`this`,
  `whoami`, `unbind`, `rm`/`remove`;
- saved-identity notes through `notes path`;
- the versioned local extension interface through `api`;
- profile and exchange commands: `role`, `preamble`, `x list|show|ack|ackall`,
  `reply`, `result`, `talk`/`send`, `check`/`read`;
- `focus <identity|pane>`, which shows a verified pane in the
  invoking user's own tmux client and reports that client (see the driver
  `focus` action), and the read-only `focus --client`, which names the same
  client and the pane it shows without switching;
- managed native updates through `upgrade`/`update`, with the hidden
  `__native-install` and `__native-refresh-skills` composition points used by
  verified release tooling;
- optional `office`, `office install|upgrade|status|uninstall`, local layout and
  local discussion-board operations. Installation
  requires consent; bare `office` and `office status` inspect the installed
  companion and service without downloading, starting or pairing.

The grammar owns option placement and rejection. Handlers do not search raw
argv, create competing option parsers, or reinterpret payload text as flags.
JSON and human output use the same typed result and status contracts.
`identity show <name>` remains a storage-only named read. Without a name it
uses the shared verified-caller selector before opening storage; an unavailable
or unbound caller does not fall back to a working directory, active pane or sole
stored identity. `identity list` and bare `preamble show` remain collection reads.
`OutputMode` contains only the supported JSON selection. Unsupported
`--verbose`/`-v` and `--debug` flags are absent from the grammar and fail with
`USAGE_ERROR` before effects; literal message/option-value text is unchanged.

`skill_reminder` presents at most one best-effort human stderr line after a
successful typed result: a newly created temporary or saved identity, a newly
started local Office, or terminal-only managed-skill drift. Repeated no-op
commands do not create a discovery transition; `TMT_HINTS=off` disables optional
transition hints without hiding error recovery or managed-skill drift. This
owner does not add fields to JSON, alter raw stdout, persist cooldown state or
scan tmux for discovery. Saved inactive target recovery remains a targeted
`NAME_NOT_FOUND` suggestion in the existing error presenter.

`output::table` is the single plain human-table renderer for binding, identity,
exchange and configuration reports. Callers own columns and typed projections;
the renderer owns control-character escaping, Unicode display-width measurement
and spacing. The CLI-only `unicode-width` dependency does not enter domain or
adapter policy. Tables preserve complete values without terminal probing,
truncation or color; narrow terminals may wrap. JSON and exact prompt, final,
profile and diagnostic bodies bypass table rendering.

## Domain and state ownership

### Identity, names and bindings

`tmt-core::names` owns canonical identity and pane-target classification,
including the pinned normalization/casing behavior and bounded name rules.
Canonicalization is ECMAScript whitespace trim, NFKC and root-locale default
lowercase using pinned ICU data, not case folding or compiler-dependent casing.
Dependency upgrades must not renormalize stored keys.
`tmt-core::identity` owns lifetime and storage-only create/promote policy.
`tmt-core::identity_metadata` owns validated string keys and values, exact-match
filters, typed results and shared metadata operations. Metadata is descriptive,
untrusted data; it does not grant permissions, capabilities, availability or
prompt authority.
`tmt-core::identity_status` owns typed self-reported activity/mood, byte limits,
expiry and set/show/clear semantics. `storage::identity_status` stores one atomic
record per active UUID in schema 29; it neither promotes identities nor mutates
their timestamps, appearance or requests. Expired data stays inspectable but is
not current activity. `identity_command::status` reuses verified caller resolution;
`tmt-adapters::identity_status` owns the shared JSON projection. The
[status contract](contracts/identity-status-v1.md) distinguishes this state from
presence and completion. The Office directory reads active UUID statuses in one
batch and composes that projection beside, never inside, appearance snapshots.
`identities/use-status-clock` schedules the next expiry for the mounted directory;
scene cues and Info consume the same observation. Appearance revision merging
preserves independent status observations. No actor polling or status writes occur
in the renderer.
`tmt-core::binding` owns evidence evaluation, retirement authorization and
binding use cases. Unknown or conflicting endpoint evidence is never treated as
proof of death. Saved identities detach and remain offline; temporary identities
may retire only after conclusive evidence.

`binding::session` separates remembered identity-owned harness/session preferences
from binding-owned runtime observations. Schema 33 retains the former independently
of a binding row; deleting/replacing that row resets its observation to unknown.
An idempotent bind retains the row and its observations. Updates use the existing
immediate binding transaction and exact binding ID, so an observation for a removed
binding cannot update its replacement. Retiring an identity, for either lifetime,
clears its remembered session and driver state in the retiring transaction; the
launch preference stays hidden behind the active-identity port. Neither a provider
session ID nor a running observation grants binding ownership or delivery authority.
Resume coordinates pair the session ID with its harness and driver-owned runtime
mode, separately from the preferred harness. Changing that preference cannot
silently reinterpret a saved session as belonging to another runtime.

Schema 37 keeps driver-owned resume state beside the remembered session: a
bounded (1 KiB), versioned document that core stores but never parses, plus a
stale mark for a session a resume found gone. The session and harness stay
columns because core correlates hook events on them. One identity has one current
runtime: only starting provider events replace the session (clearing a stale
mark, and keeping driver state only under the same driver), and a confirmed
launch under another runtime driver drops the previous driver's session and
state. Persistence is an optional driver interface: `RuntimeLifecycle::state_version`
names the version a driver reads, and `RuntimeRegistry::reconcile` discards state
it cannot read and drops sessions of unregistered drivers, which
`Storage::purge_unregistered_sessions` also sweeps. Driver state holds resume
essentials only, never transcript content, arguments or secrets.
Runtime observations retain a driver-supplied PID/start-identity pair and an
optional provider session ID. Schema 34 additionally retains an optional launch
owner PID/start-identity pair alongside the provider observation key in the binding
state. Same-incarnation hook admission and transitions preserve that owner;
admitting a new incarnation clears it. Hook-admitted runtimes without a wrapper
retain an absent owner. Admission needs fresh live evidence; inconclusive
admission preserves the previous observation, including known-ended state.
Clear and in-process resume remain nonterminal transitions and may change the
session ID within one incarnation. End/compact events must match the exact current
key. Observation writes compare the complete expected observation inside the binding transaction;
late updates for a superseded conversation cannot overwrite a newer one. Drivers
own process verification and event mapping; core does not interpret hook ancestry.

`tmt-adapters::runtime` registers pure executable recognition on the driver port.
First-party and community registrations share the same API; descending priority
and then harness ID resolve competing claims deterministically. Registration is
in-process, not dynamic plugin discovery. Explicit launches preserve every argv
byte; the registry neither executes recognition nor remembers arguments or paths.
Bare relaunch resolves the registered executable through PATH with no arguments.
Exact resume is runtime-owned, including the mode: the shared constants are
Claude `default` and Codex `shared`/`embedded`. Provider hooks must record those
same tokens. First-party resume validates the UUID-shaped IDs observed in #321;
community drivers own their opaque-ID contracts.

`tmt-adapters::setup` owns consent-plan inputs and bounded provider settings
publication. The CLI owns one approval and presentation, not provider JSON
rewriting. Hook entries are generated by the Claude and Codex runtime drivers; only exact
owned entries may be replaced or removed. Settings edits retain opaque user
values as raw JSON, refuse malformed/conflicting documents, compare the planned
input again under the setup lock, and keep a recoverable byte-exact backup before
atomic replacement. Independent editors do not participate in that advisory
lock; setup rechecks immediately before publication but is not a filesystem-wide
transaction. The selected PATH launcher remains an unresolved stable symlink,
never a resolved release path. Re-running setup repairs an obsolete owned path.

Claude SessionStart/SessionEnd decoding, context encoding and runtime ancestry
belong to `runtime::claude`. A hook supplies observation only: an existing binding
must match fresh tmux server/pane/marker evidence, and a live Claude ancestor must
belong to that pane's process chain. Payload session IDs never create bindings or
move identities. The CLI coordinator commits the existing session CAS and exact
remembered resume coordinates together. Clear/in-process-resume preliminary ends
retain the binding and mark runtime observation Unknown until the next start;
terminal ends and compact observations require the matching process/session key.
Another live or unverifiable process cannot be overwritten by hook admission.
Session-only interfaces and executable feature-extension observers remain deferred.

Codex uses the existing runtime-caller host classification. Independent hosts
require the same verified pane and fresh process-chain evidence as Claude. Shared
app-server hooks never select a binding through inherited pane/ancestry: they
require one existing exact Codex provider-session mapping, then revalidate that
recorded binding's endpoint and full session CAS. Missing or duplicate mappings
produce no context or write. An exact foreground shared resume may transfer its
owned client observation to the server while retaining the verified launch owner;
arbitrary live attachments cannot be replaced. Observed host mode is stored with
resume coordinates. A shared client exit is Unknown, not a provider SessionEnd.
Provider IDs only correlate existing observations and never create identities.
Codex setup owns `hooks.json` under `CODEX_HOME` (otherwise `~/.codex`), not
provider trust approvals or unrelated `config.toml` settings.

The runtime registry resolves optional `RuntimeLifecycle` implementations by
harness ID. Drivers own payload decoding, observation proposals, context encoding,
host classification, mode and foreground-client exit policy. CLI hook/run owners
only coordinate provider-neutral evidence, process ownership and storage CAS;
adding a lifecycle driver does not add provider switches to those coordinators.
Commands without registered lifecycle policy retain the ordinary owned-child exit
behavior. Codex currently recognizes only the observed `other` SessionEnd reason;
unknown reasons do not establish terminal state.

The provider-facing hook entrypoint always exits zero without permission/decision output.
It supervises a short-lived internal worker through the existing process owner,
with a two-second work budget and bounded cleanup; provider settings allow three
seconds. This bounds process, SQLite and context work without a daemon or late
background context writer. Worker probes stay inside the supervisor-owned worker
group. A failed worker terminates its own group before exiting; the supervisor
owns deadline termination and reaping, so nested probes cannot escape cleanup.
Hooks open only existing compatible storage, use a short lock wait, and never
migrate it. Timeout/error emits no context and at most
one fixed stderr line. No resolvable caller pane is a normal silent outcome,
without context or diagnostics. A verified empty pane receives only user-facing
information, never an instruction for an agent to bind itself.
Successful starts reuse the read-only context formatter;
ends emit no stdout. Provider configuration is changed only by consented setup,
not by a hook, ordinary command, or skill installation.

The CLI foreground owner separates command selection, binding, spawn and runtime
admission. A verified live or stopped previous runtime prevents a second launch.
An inconclusive previous-runtime probe permits a degraded launch only after
fencing that same attachment's stored Running state to Unknown; known Ended is
preserved. This prevents a recovered probe from reviving delivery into the new
command. Admission failure never restarts or kills a successfully launched child.
After waiting, completion rereads the binding in an immediate transaction and
ends only the matching binding, child incarnation and launch owner, using the
current provider key even if a hook changed it. The SQLite handle is closed
before the interactive wait and reopened for completion; storage diagnostics
never replace the actual child exit code.
An owned, unreaped child that has already exited can retain its real start
identity for direct Ended recording; it never authorizes delivery. If a child
exits after the live admission probe, delivery still revalidates process evidence
and the foreground owner records Ended when it reaps the child.
The coordinator invokes its injected `HookObserver` only for committed session
observations, outside storage transactions. Fast exit emits start then end from
one committed Ended record; a failed admission emits neither, and an already
recorded end is not emitted twice. Observer failure is diagnostic, never a veto
or a reason to restart the command. CLI composition currently supplies an empty
observer; registered feature-extension dispatch belongs to the extension envelope,
not runtime recognition or the foreground process owner.

`Storage::identity_candidates` is the storage-only discovery owner for completion
and identity pickers. It opens existing storage read-only, without migration,
creation or presence reconciliation, and returns active identities in saved-first
canonical order with optional literal-prefix and remembered-session filters.
Unavailable discovery is not evidence that an identity does not exist; binding
and launch still perform their normal authoritative checks.
The CLI's hidden completion query resolves the unfinished operand through the
same public Clap grammar and emits only a context tag, candidate names or command
offset. Shell adapters retain generated static completion and delegate arguments
after `run`'s identity to the command's own shell completion. They do not own a
second TMT parser or runtime-driver list. Discovery failures are silent and do
not initialize storage. `run -s` uses the existing binding lifetime promotion;
without it a new identity is temporary and an existing saved one stays saved.

`tmt-core::driver` defines optional typed actions and observation-only hook values.
Unsupported actions are distinct from accepted, queued, failed, denied, approval-
blocked and uncertain outcomes; only unsupported has automatic fall-through in
this contract. Concrete adapters must bound their effects through the existing
process owner. These contracts do not discover or execute plugins, install provider
hooks or replace the durable retirement receipts in `identity_hooks`. Container
interfaces remain the existing tmux binding records; the session interface kind
is reserved, not a shipped session-only binding store. Current
public messaging still uses its existing tmux transport and request lifecycle.
Implicit caller selection first consults the runtime driver's `identify_caller`
action. `runtime_caller::codex` owns Codex thread markers and bounded process
ancestry inspection. It takes one PID/parent/command snapshot and walks it in
memory, reading arguments only for Codex ancestors. Both caller and runtime-start
observations share the fixed-path/locale `process::ps` runner and its missing-only
executable fallback. Tmux continues to own server, pane and marker verification.
A shared app-server's inherited pane is not evidence of the invoking conversation.
A positively observed shared app-server rejects required implicit attribution
before binding/configuration effects. No Codex ancestor means Unsupported even
with an inherited or malformed thread marker, preserving normal host verification.
An unavailable probe fails closed only when a thread marker is present; without
one it is Unsupported. A malformed marker does not override an observed independent
runtime. Optional senders can remain anonymous, with one stderr attribution notice
(including JSON mode, whose stdout document is unchanged).
`run_command` applies this same guard before caller resolution or launch state
access; a rejected caller cannot bind an identity or start the supplied command.
Explicit identity or pane selectors bypass
that inference, not their normal validation. A thread ID is only a correlation
hint: the current selector does not derive identity from remembered session
preferences. Automatic current-session correlation remains dependent on the
runtime hook integration. No caller probe changes bindings or sends input.
`tmux::BindingSession` also implements the action port: status delegates to the same
full server/pane/marker evidence evaluator, and send requires present evidence
before invoking the existing paste-and-Enter transport once. It preserves that
transport's preparation-versus-uncertain failure distinction. `focus` (a
default-`Unsupported` driver action returning the shown and previous interface IDs
and the host's name for the view that moved)
requires the same present evidence but no running agent, then switches only the
invoker's client: the client showing the session of `TMUX_PANE`, or, without
`TMUX_PANE` (key-binding jobs) or for a display-popup whose own pane has no
session, the session named in `TMUX`, choosing
the most recently active such client. A bare tmux "current client" is never used, a
foreign or unidentifiable client is `HOST_UNSUPPORTED`, and focus sends no buffer,
paste or key input. `Tmux::invoker_client` is that resolution alone, read-only,
and backs `focus --client`. Runtime-only actions remain unsupported; a live pane does not
establish a running provider session. Known-ended runtimes reject input as offline.
Missing or conflicting interface evidence masks the reported runtime to unknown
without rewriting stored evidence. Recorded running processes are rechecked through
the bounded `process::runtime` observer before input. It uses a fixed-locale,
fixed-timezone `ps` start identity (second resolution), not a PID alone or provider
transcript. Process disappearance, zombie state or a changed start identity reports
ended. Stopped/traced processes remain unknown rather than ended, allowing later
resumption without sending input to the shell meanwhile. Inconclusive checks also
remain unknown and do not permit fallback to legacy unobserved delivery. The probe
uses `/usr/bin/env` and fixed `/bin/ps`, then `/usr/bin/ps` only when the first
executable is missing, within the same deadline. Systems without these utilities
cannot verify a recorded runtime. The observer does not prove interface ownership;
the driver must establish that separately. For wrapper-launched runtimes, a
surviving child also requires a live matching launch owner before input is allowed.
Missing, reused, stopped or inconclusive owner evidence makes the runtime unknown,
not ended: the child may survive while the shell has reclaimed the terminal. Child
death still reports ended regardless of owner liveness. Owner fields participate in
the same full-observation CAS.

The concrete implementations are `storage::{identities,identity_metadata,identity_status,bindings}`
and `tmux::{metadata,evidence,binding,caller,transport}`.
`binding_command` performs caller/target preflight and composes those owners.
The tmux adapter owns opt-in badge markup derived from recorded session state:
green running dot, dim ended badge, plain unknown label. Names are sanitized
before generated style markup is added. Adapter `pane_badge` refreshes the current
binding's projection after committed launch, exit, provider-hook and recovery
transitions, sharing hook deadlines. It is bounded, best-effort presentation,
never routing evidence; no polling, theme mutation or independent state store.
Presence is observation, not routing permission; an explicit socket or pane
marker cannot authorize a different identity.
`context_command` composes `whoami --context` separately from mutating binding
reconciliation. `storage::context` opens SQLite read-only and reads the binding,
role and unacknowledged X counts in one transaction; fresh driver evidence must still
establish presence before those identity details are rendered. The projection
does not create storage, migrate schemas, refresh binding timestamps, acknowledge
requests or run retention cleanup. Originated and incoming counts use their
independent per-item and bulk attention watermarks, matching the corresponding
X lists. Expired requests are filtered at read time. Runtime caller evidence gates
implicit host attribution; `tmux::observe_snapshot` never initializes server
metadata. Ambiguous, missing or failed evidence renders no human context, not an
unbound hint. Only a verified empty pane gets that hint. `notes::existing_path`
uses the existing no-follow path traversal without opening the notebook content
or creating a notebook. CLI presentation bounds the complete human/JSON output
to 4 KiB, preserves counts and inspect commands when shortening role/path content,
and marks truncation. No request IDs, bodies, receipts or notebook contents enter context.
The extension contribution slot is currently empty; this path neither discovers
nor executes extensions. Session-only interfaces remain unimplemented as above.
Binding SQLite reads and writes reuse `endpoint::valid_process_id` with checked
signed/unsigned conversion. Invalid stored PIDs fail decoding
without repair or retirement, and invalid inputs fail before insertion.

Names are global within the selected local database, not folder-scoped. Plain
`name`/`add`/`marked` creates temporary bindings; `-s` saves/promotes the same
identity UUID. `marked` resolves one `pane_marked` observation on the
invocation-selected tmux server, then passes frozen server and pane ID/PID
evidence through the existing binding coordination. Later focus or mark changes
cannot redirect the operation; lost, replaced or conflicting endpoint evidence
fails closed. The resolver never substitutes a caller/active pane, searches a
different server or mutates the user's mark.
Conclusive pane/server death or explicit unbind retires temporary names without
erasing retained exchanges; saved identities remain available offline. Saved
removal requires explicit force. Neither removal nor unbind kills a pane.
`list` may show verified foreign-server identities, but `talk`/`check` routing
remains current-server-only. Pane number, presentation title and socket pathname
alone are not endpoint identity. Publication and recovery preserve the full
server/pane process evidence; ambiguous observations fail closed.

### Saved identity notes

`tmt-core::identity::NotesIdentityId` is the capability boundary for notebook
storage: construction requires a saved identity and a canonical RFC 4122 UUIDv4.
Display names never become path components. `notes_command` resolves the active
identity before requesting filesystem work; omission uses only a verified tmux
caller and explicit selection can use an offline saved identity.

`ConfigPaths` is the sole layout owner. `tmt-adapters::notes` exclusively creates
`<global_dir>/notes/<identity-uuid>/notes.md`, returning an absolute path. It
creates one directory component at a time with owner-only modes, uses exclusive
no-follow file creation, rejects linked/non-directory subtree components and
nonregular targets, and never opens an existing notebook for writing. The file
body, edit concurrency, and retention are ordinary user-filesystem concerns;
there is no SQLite body copy, revision protocol, watcher, lock, file-size policy,
per-agent isolation, or secure deletion claim.

The local Office notebook extension reads that same file through `notes::read`,
which revalidates active saved identity eligibility. It pins directory handles,
refuses symlinks/nonregular files and delegates to `bounded_file::read_opened`;
it never initializes missing notes. The authenticated loopback GET adapter exposes
only UUID/name/exact UTF-8 content, not caller-selected paths or writes. Its separate
1 MiB viewer ceiling does not restrict agent file edits. The browser port and
read-only panel own response admission and cancellable read lifetime, not storage.
See the [notebook contract](extensions/tmt-office/contracts/notebook-v1.md).

Identity retirement deliberately leaves notebooks in place. A later same-name
identity has a different UUID and therefore a different path. No command moves
notebooks for pane, tmux presentation, role, working-directory, or Office
changes, and no garbage collector is implied.

### Settings and configuration

`tmt-adapters::config::ConfigPaths` is the sole application path owner.
`config::document` preserves unknown JSON fields and validates known settings
through `tmt-core::settings`. `init` exclusively creates the selected local
file as `{}\n`; it neither loads configuration nor opens SQLite or tmux.
Human `config show` derives the effective source and CLI capability from the
resolved settings and editable-key policy. The three `defaults.*` settings are
global-file-only; showing them does not make them CLI-editable. JSON projection
and targeted write validation remain unchanged.
Existing files, directories and links are refused without mutation.
Configuration errors retain their stable public codes and useful paths only at
the adapter boundary.

`json_document` owns editable config/tmux metadata number compatibility:
IEEE-754 values with non-finite opaque values serialized as null. Known invalid
settings still fail. Raw object order is retained on targeted edits; this is not
an exact reply/body transformation or the receipt decoder's policy.

### SQLite and durable exchanges

`tmt-adapters::storage` owns one private synchronous `rusqlite` connection,
schema migrations 1 through 36, WAL/foreign-key/FTS5 setup, busy and transaction
boundaries, and close/checkpoint cleanup. Historical schemas and frozen fixture
provenance are evidence, not a second implementation. The adapter keeps raw
connections private and exposes narrow ports to core services.
It classifies OS-denied writes and SQLite read-only/WAL failures as a typed
not-writable error; a generic CANTOPEN needs independent permission evidence.
An existing data directory without owner write permission is reported, not repaired.
CLI failure projection names the selected data directory and preserves the
pre-transport versus uncertain-delivery distinction. The tmux adapter similarly
classifies socket access denial before CLI presentation.
Migrations preserve recorded names and historical retention backfills. Schema 9
promotes existing identities to saved without changing UUIDs; unsupported custom
identity-table definitions are rejected rather than silently rebuilt. Old
schema-8 writers cannot share the migrated database. Frozen inputs retain their
own provenance in `typescript/test/fixtures/storage-history`, not in this architecture map.
Schema 10 adds identity hook subscriptions and terminal delivery receipts;
registration after retirement queues immediately, and delivered subscriptions
cannot be resurrected by registration retries.
Schema 12 adds a typed inbox route and recipient-scoped attention without
fabricating tmux endpoint evidence. One request/final lifecycle remains the
source of truth; originator and recipient acknowledgment are independent.
Schema 13 adds UUID-owned identity metadata with one unique value per key and an
exact `(key, value, identity_id)` search index. Adapter operations revalidate the
active UUID, serialize writes with the existing immediate transaction owner and
enforce the 64-entry limit atomically. Retirement hides metadata; explicit
content removal deletes it, while a same-name replacement receives a new UUID
and inherits nothing.
Schema 14 adds the installation-owned local Office discussion board. Pure bounded
values, actors, receipts and cursor policy live in `tmt-office-model::office_board`;
`tmt-office-storage::office_board` owns active-UUID preflight, owner-world revalidation, immediate
transactions, soft deletion, board-local idempotency receipts, the single board
revision and indexed keyset pages. The Office command library crosses the verified
`tmt-office` one-shot protocol, while the stopped-service-independent companion and authenticated
loopback HTTP adapter call the same repository. Repository categories are
credential-free Git remote identifiers, not permissions, and category discovery
is a synthetic-general plus stored-root projection rather than a registry.
Schema 30 generalizes the stored category identifier and adds canonical room UUID
categories to this same board store. It transactionally preserves existing roots,
replies, tombstones, sequence/revision values and operation receipts. New room
threads require an existing room, not room membership; retained threads and exact
operation replays remain readable after room removal. Room scope is classification,
not access control. Category discovery uses the same indexed keyset ordering for
repository and room identifiers; pre-upgrade category cursors require a fresh page.
Office `--room` selection reuses the canonical room resolver through `CoreAccess`.
The browser discussion
binding selects General or an explicit room UUID in that same store; placement
changes cannot retarget it. `local/board-navigation` aggregates form-owned leave
protection for category, thread and spatial-entry switches. `use-board-mutation`
owns a frozen operation per form; refresh and ordering preserve selection and
unconfirmed writes. Confirmed discard resets forms, not stored content. Closing
the panel retains its mounted session, while explicit retry reuses the original
scope, entry revision and operation UUID.

`tmt-core::request::RequestService` owns preparation, delivery-state
transitions, exact final submission, waiter release, attention revisions and
bounded retention housekeeping. It samples clocks at the transaction boundary,
never holds a transaction across transport, and treats uncertain delivery as
uncertain rather than as a replay authorization. `storage::requests` owns SQL,
row decoding and ordered bounded cleanup; `request::attention` owns the pure
attention contract. Prompt/final content, attempt metadata, retention and
acknowledgment state have independent lifecycle rules.

The request service reserves cadence together with a durable attempt before
sending, then records definitely-failed, sent or uncertain delivery. Only a
definite failure permits the defined reservation refund; timeouts are not proof
of non-delivery. Final bodies are immutable: identical retries are idempotent,
conflicting second finals fail, and terminal text is never used as completion
evidence. `talk` waits for a stored final unless detached or timed out;
`result` reads by request, while identity-owned `x` exposes outstanding attention.
Reads do not acknowledge. `ackall` acknowledges one snapshot, so a later final
becomes unread again. Acknowledgment means handled, not successful or cancelled.
Retention is frozen per attempt; bounded lazy housekeeping must respect active
waiters, preserve the defined acceptance deadline and never resurrect an expired
submission. The settings owner defines retention defaults and limits.

`RequestRoute` distinguishes unbound direct-pane delivery from durable identity inbox
queueing. Identified talk is Inbox-first with one claimed full-payload live wake;
its public live output remains sent/completed. Pane attempts retain server/pane evidence; inbox attempts retain only
the resolved active recipient UUID and settle as `queued`, never `sent`.
`RequestService::enqueue` prepares the attempt, stores its exact prompt and
publishes recipient attention in one repository transaction. CLI inbox sends use
this path; pane effects retain the separate prepare/send/settle lifecycle.
Both paths reuse the same preparation and queue-transition policy. Database
errors roll back all enqueue writes. A recipient found inactive commits a failed,
non-waiting attempt without recipient attention, matching the prepared queue path.
Interrupting a sender after publication only releases its wait; it does not
retract queued recipient work.
Full request delivery settles the request's recipient attention, not the
originator's response attention; an advisory Office wake leaves it unread.
Delivery failure cannot rewrite the receipt-bound route. Runtime return does not
schedule a second wake. Preamble reservations are prepared once and refunded only
for proven non-delivery; transport still owns literal-input protection.
The recipient revision is allocated atomically with the `queued` transition, so
a merely prepared attempt cannot wake a listener and every newly eligible item
advances that identity's shared participant sequence. Recipient request attention
is projected from the same attempt, while a final
written by another participant reuses the originator response attention.
`storage::requests` provides an indexed watermark and one bounded snapshot;
`exchange_command` owns the monotonic hard deadline and trailing debounce.
Listener polls perform no tmux inventory, retention cleanup, body scan or held
transaction, and introduce no daemon or event bus.

Schema 35 adds optional request notification policy and one-shot reply/timeout
claims under the existing request service transaction. No row means no callback:
historical, anonymous and explicit queue-only requests are not opted in. First
final acceptance may reserve a callback only without a live blocking waiter.
Process evidence is observed outside the transaction and matched against stored
waiter ownership inside it. Acceptance and notification outcomes remain separate.
`delivery` composes registered runtime send with verified host fallback through
the core routing policy; accepted, uncertain, denied and approval-required sends
never fall through. Drivers own fresh runtime proof and sticky-Ended recovery.
`process::detached` owns startup acknowledgment and failure cleanup for one
request deadline observer, and the observer's removal of its own stderr log
after a clean exit, only when the path still names that same file (device and
inode). Failed and crashed observers keep their log as bounded diagnostics;
there is no sweeper. `request_observer_command` owns the per-request log path
and composes durable reads, the timeout claim and delivery outside locks. It has
no restart policy, daemon, provider-specific branch or permission to re-send a
request.

`reply_receipt` is the one maintained receipt codec. `response_command` and
`talk_command` compose it with the request service; neither adds a repository,
schema, connection or alternate final-submission path. Input is bounded and
validated before storage effects. A malformed receipt, a stale revision, an
unknown identity and an uncertain transport outcome remain distinct failures.

Talk preparation renders `<tmt-reply from="…">` using the same resolved
originator's display name (explicit identity before verified caller), or
`unknown`. The attribute is XML-escaped presentation, not authentication,
routing or a strict XML document. It introduces no extra identity lookup or
stored field; original message bytes, originator UUID/kind and reply correlation
remain owned by the existing request contract.

### Tmux and process effects

`tmt-adapters::process` is the shared bounded subprocess owner. It enforces
output caps, monotonic deadlines, process-group cleanup and wait/reap behavior.
Its owned running-command handle separates launch from wait when a caller needs
to release a selection lock; synchronous execution uses that same path. The
original deadline and cleanup ownership survive the split. An abandoned handle
stops and reaps its child without introducing a second runner or background task.
`process::interactive` owns direct-terminal children separately from bounded
probes: inherited streams and the shell's foreground process group are preserved.
Invocation-scoped signal notifications wake its wait without a timer. Terminal
interrupts reach the child directly; wrapper-directed TERM/HUP are forwarded to
the owned child only. A notification failure reports degraded supervision and
waits for the child normally instead of killing a live agent. Abandonment first
requests termination, then kills if necessary and reaps that child, never the shared
process group. Harness-created descendants and wrapper SIGKILL are outside this
cleanup guarantee. The CLI `run` owner uses this adapter for foreground commands.
`interrupt::Interrupt` owns invocation-local signal callbacks and descriptor
cleanup. `tmux` uses explicit socket/server evidence, bounded command budgets,
owned buffers and no ambient host fallback. A failed paste or Enter is an
uncertain delivery and is never retried as if unsent.
Message delivery changes ASCII `!` to fullwidth `！` to avoid agent bash-mode
shortcuts; this is transport policy, not arbitrary output rewriting. `check`
remains bounded terminal diagnostics, not a fallback response channel.

`response_input` owns bounded file/stdin acquisition, regular-file checks,
nonblocking behavior and restoration of inherited descriptor flags. The public
CLI owns stdin during acquisition. These adapters do not invent background
threads or a second process runner.

## Managed skills and native installation

Managed agent guidance is a separate filesystem concern. The canonical
`tmux-team`, focused `tmt-inbox`, and optional `tmt-office` skills are embedded
as one versioned asset bundle by `skill_installation::assets`; digest-addressed
materialization, provider detection, target selection, links, backups, registry,
drift and lock handling live under
`rust/crates/tmt-adapters/src/skill_installation/`. Core install exposes only
`tmux-team` and `tmt-inbox`. Explicit Office install or upgrade exposes
`tmt-office` in detected provider roots and any custom root that still contains
an owned core skill. CLI upgrades refresh recorded Office links without creating
missing integrations. Core's `skill_provider::Provider` is the only provider
inventory. Skill installation does not open application configuration, SQLite
or tmux, and never silently replaces an unmanaged path.

Extension-owned skills arrive as bytes through the local API
(`skills.install`/`skills.remove`, both requiring explicit `consent: true` from
a caller that asked the user). `skill_installation::owned` validates them,
materializes each under `skill-assets/owners/<owner>/<digest>/<name>` (verified
by recomputing the digest) and links it into the same roots as the optional
Office skills. `skill-owners.json`, separate from the target intents that core
refresh reads, records each name's owner, digest and targets. Core's names are
reserved; the first owner of any other name keeps it until an explicit force.
Because the same-user API cannot authenticate its caller, install and remove
refuse targets another owner holds rather than trusting the stated owner.
Claims and unmanaged paths are checked for every target before any effect.
Office links published from the core bundle are adopted by owner `office`
without force (any other owner needs force), and core's bundle then leaves held names alone: Office facade
installs skip them and CLI refresh reports them as skipped. Removal deletes
only links into the owner's store; drift reports owned targets that no longer
point at their owner's current content.

The core skill sources stay under `skills/`; the three Office skill sources live
under `extensions/tmt-office/skills/`. `skill_installation::assets` still embeds
those Office sources into the core bundle, so core compiles bytes from the
extension tree. This is retained core-to-extension extraction debt owned by a
later #328 slice, not a second skill source or a provider-specific copy.

Native executable installation is a different owner under
`tmt-adapters::native_install`:

Core's fixed `native_install::Product` policy owns CLI/Office package identity,
inventory and installation namespace; it has no filesystem or network effects.
The hidden offline installer accepts an explicit product (CLI by default), while
both products use the same acquisition, receipt and atomic publication path.
Office's command link, lock and current release are independent of the CLI's;
existing CLI receipts retain their format. Manifest selection uses product and
target together, rejecting ambiguous or multiply owned artifacts. This internal
path also serves public Office installation. `office_command` owns consent and
typed composition, not a second downloader. Default Office prefix is the user's
`.local`, independent of application configuration; `--prefix` selects another
owned installation. Public distribution and pairing remain separate gates.
GitHub selection filters CLI `v` and Office `tmt-office-v` tags independently.
Downloaded bytes feed the same bounded artifact verifier directly; there is no
extra download-to-disk/read-back stage. Before activating Office, publication
executes the bounded versioned probe and rejects incompatible candidates.
Removal validates ownership and deactivates links without deleting releases,
skills or application data. It is recoverable, not a multi-file atomic deletion:
a missing command link with a retained activation is reported as invalid and
explicit uninstall can finish that state.

- `artifact` consumes cargo-dist metadata and a matching archive, checking
  target, manifest membership, SHA-256, bounded compressed/expanded input,
  notices and executable contents;
- `publication` stages a release under an invocation-owned prefix, writes
  receipt/current/command links atomically under the installer lock, and keeps
  old owned releases until ownership and integrity checks permit cleanup;
- `receipt`, `release`, `managed` and `upgrade` implement local provenance,
  active-release inspection, channel/pin policy, verified HTTPS acquisition and
  forward-only activation;
- `native_install_command` and `native_upgrade_command` are thin CLI
  compositions. Application data and provider skills are separate owners.

The active executable is the authority for a managed update. Installer receipts
are anchored to the installation prefix/current executable, not to
`ConfigPaths.global_dir`; changing runtime config roots must not fabricate or
discard binary ownership evidence. Installation uses staged publication,
expected-current checks, explicit checkpoints and bounded cleanup. A failed
validation or cancellation leaves the previous current release and receipt
intact; a post-activation skill failure reports partial completion rather than
claiming an atomic application-wide transaction.

Public Office installation reports companion activation and optional skill
publication as separate outcomes: a guidance conflict never rolls back an
already activated companion or overwrites user content. `--force` authorizes a
recoverable target backup, not source replacement. Office deactivation retains
managed guidance; it does not silently remove an agent integration. The hidden
offline product installer remains binary-only.

The generated curl bootstrap is release tooling around this same native
installer. It derives archive facts from cargo-dist metadata and does not own a
second target catalog, archive parser, package manager, or production manifest.

## Squad extension

`extensions/tmt-squad/rust/tmt-squad` builds the optional `tmt-squad` executable,
reached through the external command contract as `tmt squad` and, through a
`tmt-sq` link to the same file, `tmt sq`. Its command name is fixed, never taken
from argv[0], so both spellings share one help text, error set and completion.
It is a workspace member for the shared lockfile and toolchain only. It depends
on no TMT crate, and no TMT crate depends on it; the architecture guard enforces
both directions for Cargo dependencies and source references. Squad reaches TMT
through `TMT_EXECUTABLE` (or `tmt` on PATH), using public `--json` commands and
`tmt api`, with its own minimal bounded child runner.

Squad keeps no store. A squad is the core room `squad-<name>`. Member fields are
identity metadata `squad.<name>.<field>`, so one identity can belong to several
squads and removal clears exactly one namespace. `status` joins one
`rooms.roster` snapshot with `ls --room` presence. It always returns one
`sections` shape: without user-defined sections, a single untitled section.
User-defined sections (`[[squad.<name>.section]]`: title, filter, sort) replace
the single list; `filter` owns a bounded boolean language over a row's text
fields, and every section is validated before output. `tmt squad board` renders
the same status document with ratatui over crossterm; `board::terminal` owns raw
mode and the alternate screen behind a `Screen` trait, restoring on return,
error, panic (via the panic hook) and TERM/HUP (signal-hook). One refresh thread
loads snapshots off the input loop, collapsing queued requests, so keys act on
painted data; stale results for a squad the user left are dropped. Without a
terminal or with `--json`, `board` is `status`. `[squad.<name>.board]` selects
split or tabs panes (rows, notes, detail, replies) over a per-layout preset,
validated before raw mode. The notes pane reads the lead's notebook only through
`tmt api notes.read` (bounded, never creating a file); `board::notes` removes
every escape sequence, control character and hidden bidi/format character before
display, since notes are agent-written. `board::markdown` is a thin
pulldown-cmark view over that sanitized text: it styles headings, lists,
emphasis, inline code and links, and shows every other construct as its source.
State `sort` overrides reorder the vocabulary for both `status` and the board.
`effects` holds the row actions behind the plain `jump`, `open` and `copy`
commands and the board. `template` fills `{field}` placeholders into one value and refuses
empty values. Programs run as argv, never through a shell: the configured
top-level `opener` and `clipboard` arrays, or the system opener. An opener
starts in its own process group with null stdio, and a thread reaps it. Copy
prefers the configured program, then, inside tmux, `tmux -S <invoker socket>
load-buffer -w -`: `-V` must report 3.2 or later, and `show -sv set-clipboard`
decides whether the text reached the clipboard or only a buffer. Otherwise copy
writes OSC 52 to `/dev/tty`. `jump` checks membership and then calls `tmt
focus`; squad has no focus logic of its own.
`action` parses `[bind]` and `[squad.<name>.section.bind]` once per load into
events and actions whose arguments are templates; bad events, actions or field
syntax are configuration errors. The board resolves the selected row's section
binding, then `[bind]`, then the host preset (tmux: Enter and double-click jump;
a plain terminal: they open the row's action menu) into a fully filled request
before anything runs; a missing value is a notice, not a partial action. Mouse
capture is part of the terminal state the `Screen` guard restores; each draw
records which screen lines show which row, so a click selects exactly the row
drawn there. `run` fills one argv element per template and starts it like the
opener (no shell, null stdio, its own process group, a reaper thread). `back` keeps a
disposable stack per tmux server and client (`$XDG_CACHE_HOME/tmt-squad/back`,
0700, atomic replacement, 32 entries, corrupt or foreign files read as empty).
Every jump pushes the pane the client left, under the client `tmt focus`
reports; `back` asks core for the invoker's client with `tmt focus --client`,
pops its entry and focuses it, so squad still never talks to tmux about clients.
`hotkeys` generates `squad.tmux.conf` (bindings noted `tmt squad popup|pane|back`)
and owns one `source-file` line in the user's tmux configuration. It edits that
file only after consent, rereads it before publication, keeps a byte-exact
backup and replaces it atomically with the original mode; removal drops only
the exact owned line. A linked configuration is resolved (at most eight hops,
each relative to the link's real directory) and written beside its real file,
so the link survives; dangling or looping links are refused before consent. The bindings record the first `tmt` on PATH that resolves
to the running executable, not the release path. Collisions and ownership on
the running server come from `list-keys -N -P "" -T prefix` (notes) and
`list-keys -T prefix` (commands), because `list-keys -F` postdates tmux 3.2;
squad unbinds only keys whose note is its own. `board --popup` ends the session
after a successful jump.
`send` sends as the user's saved identity through public commands only: detached
`talk --identity <me> --room squad-<name>` with operands after `--`, annotations
as a talk tagged `[<squad> · <row>]`, and replies with the receipt that `x show
--incoming` gives the recipient; nothing acknowledges. `requests` derives each
row's `annotation` (the user's newest open tagged request) and `waitingOnYou`
(open requests to the user) per load from `requests.list`, at most four pages
of 50, and marks the document `olderRequestsNotShown` when a window is cut off.
The same room window yields the replies list (finals to the user's requests,
newest first); bodies come from `requests.show` for the newest eight only, and
the refresh worker caches them by request ID because a submitted final never
changes. Bodies are agent-written and are sanitized like notes before display.
Membership commands are sequences of idempotent core commands, not one
transaction; each reports what it applied, and a re-run converges. `squad.toml`,
beside the global config that `tmt config show` reports, is the user's file.
Squad writes only the top-level `me`, with a changed-input check and atomic
replacement that preserves the rest of the document. `init` settles `me` before
creating the room. The `tmt-squad` lead skill source lives under
`extensions/tmt-squad/skills/` and is embedded only in the squad executable,
never in the core skill bundle. Squad's dependencies must not change the CLI
product: the proof is package-scoped (`-p tmt-cli` alone), because combined
workspace builds may unify shared-dependency features across packages. The
release workflow does not distribute squad until the generic extension installer
(#387).

## Testing and evidence boundaries

Office's opt-in `playwright.visual.config.ts` reuses the local HTTP fixture and
real browser renderer for reviewed platform/furniture/HUD pixel baselines. Its
scenario-local read-only world is not a native admission or persistence oracle.
The browser partition verifier keeps these tests separate from standard CI and
capacity diagnostics; [Development](DEVELOPMENT.md#personal-office-milestone-acceptance)
owns execution, platform-specific baselines and explicit visual-review updates.
Geometry, gesture history and native durability retain their existing test owners.

Retained tests are organized under `typescript/test/native/`, `typescript/test/e2e/`,
`typescript/test/tooling/` and `typescript/test/support/`, with Rust unit/integration tests beside
their owners. They use independent SQL/schema oracles for SQLite behavior and
frozen fixtures from `typescript/test/fixtures/storage-history/`; implementation reads
must not generate their own expected results. Native process tests use absolute
task-owned executables, bounded subprocesses and cleanup that stops, reaps and
only then removes fixture state. Signals are sent only to task-owned child
processes. No host tmux server, provider installation or global environment
mutation is test evidence.

`typescript/test/support/cli-process.ts` owns each native sandbox's active child runs.
It also owns `TMUX_TMPDIR` under the sandbox, so ancestor discovery cannot reach
the host's default tmux server after caller variables are cleared. Native process
fixtures do not start default-socket servers; real tmux scenarios belong to Docker.
Descriptor clones share that lifetime. Direct-child exit starts same-group
cleanup even when descendants retain output pipes. Success requires direct
close and confirmed group absence; cleanup failure is bounded and retains
fixture files for diagnosis. Sandbox disposal cancels outstanding runs before
removing files. This is not containment of descendants that create new sessions,
and does not replace the separate Docker harness or release verifier.

The native process suite proves parser, configuration, identity, notes,
response, exchange, talk, installation and skill contracts through the real executable.
Docker E2E supplies private tmux, caller, lifecycle, transport and cross-process
evidence. Storage adapter tests prove migrations, transaction rollback,
contention, crash cleanup, retention, acknowledgment and late-final behavior.
Tooling tests prove release-script policy and bounded command wrappers; they do
not count as native runtime or release-archive proof.

Within Docker E2E, `cli-assertions.ts` owns the repeated strict success envelope
(zero exit, empty stderr, defined parsed JSON), not domain validation or command
execution. Scenario-specific payload projections and assertions stay local;
sharing a type must not turn required fields into optional ones. A different
stderr or parsing contract is not an interchangeable helper. The native-process
assertions in `typescript/test/support/cli-process.ts` retain their own process-result shape.

All public-command E2E scenarios use `typescript/test/support/cli-executable.mjs` through
the harness. There is no separate product-only native selector; explicit
`TMT_TEST_CLI`/peer descriptors still exercise override and nested-reply behavior.
`tmux-adapter` and `transport-adapter` deliberately select the test-only tmux
probe, not the public CLI. Their evidence cannot replace public command tests.
Feature ownership and deliberate overlap are mapped in DEVELOPMENT.md.

The six required runtime smoke environments are macOS x64/arm64, Linux glibc
x64/arm64 and Linux musl x64/arm64. Four raw native builds feed these checks;
the static Linux musl binaries are reused for both Linux environments. The
historical `Packed install (<environment>)` check names and
`Native package matrix` final blocking aggregator remain for CI
compatibility, but their step descriptions must identify them as native runtime
smoke checks, not npm-package checks. Smoke runs outside the checkout with
isolated HOME/state, no Node/Rust on the product PATH, exact embedded skill
checks, managed skill installation and SQLite reopen/persistence.

Executable selection is checked positively and negatively: a selected native executable
must run, and a missing default native build must fail clearly. No Rust coverage
percentage is compared with the retired TypeScript source or reported as a
zero-file success.

## Release boundary

`dist-workspace.toml`, `scripts/build-native-artifact.sh`,
`scripts/native-cargo.sh`, `typescript/scripts/native-artifact-policy.mjs` and
`typescript/scripts/verify-native-artifact.mjs` are developer/release tooling. The
workflow builds the four supported cargo-dist targets, creates target-filtered
third-party notices (including Vite's bundled frontend inventory for Office), and verifies runtime bytes, linkage, checksums, archive
inventory and executable behavior on matching hosts. CLI runs additionally
verify exact managed-skill contents and the generated bootstrap.

The release workflow remains an explicit product-selected preparation and
verification workflow; publication is separately authorized. CLI and Office
runs share the four-target cargo-dist build and archive verifier, while keeping
product-qualified bundles, independent versions and separate immutable tags.
Only the CLI bundle owns the generated `tmt-installer.sh` and managed-skill
bootstrap proof. Archives, their product-specific manifest/checksums and notices,
plus the CLI bootstrap where applicable, are verified before any public
publication. Raw PR executables do not prove cargo-dist archive correctness.
The runtime/linkage proof is shared through `typescript/scripts/native-runtime-proof.mjs`
and `typescript/scripts/verify-native-runtime.mjs`; do not reintroduce a second archive
builder or proof implementation.

## Maintenance contract

Update this map in the same change when responsibility, dependency direction,
command/error contracts, storage schema or lifecycle, trust boundaries,
resource ownership, shared test infrastructure or release evidence changes.
Keep a significant decision's alternatives, failure behavior and verification
plan in its issue and reflect the delivered boundary here. A green formatter or
checkmark is not architecture evidence.

Every change reports its architecture impact and names the affected Rust owner,
adapter, CLI composition and tests. New policy belongs in the existing owner;
do not add a parallel TypeScript implementation, provider inventory, config path
registry, release catalog, process runner, archive parser or memory/MCP layer.
