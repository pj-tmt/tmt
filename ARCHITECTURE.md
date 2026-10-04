# Architecture

The shipped CLI runtime is the Rust workspace in `rust/`. An optional Office
SPA foundation lives in `extensions/tmt-office/typescript/apps/office`; it is not a CLI fallback or a
shipped connector. The nested `typescript` pnpm workspace owns Vitest, fixture
and release-verification tooling; the repository root has no Node package.
Contributors run Cargo and the nested pnpm scripts directly. The pnpm workspace
is not a second CLI runtime, an npm product, or a source-install fallback. A native source checkout selects
`rust/target/debug/tmt` (or an explicitly supplied native executable); a missing
native build is an error. No test, script, or installer may silently execute an
installed host `tmt` or a retired TypeScript product implementation. Node may
run explicit developer fixtures and verifiers, never serve as a product fallback.

Published releases are immutable. Source changes do not publish replacements
or migrate application data.
TMT remains an invocation-owned local CLI, without a remote MCP server, identity
memory or a separate inbox service. The one MCP server it ships is the hidden
`__channel-server`, a stdio server that an opted-in Claude launch starts as its own
child; it listens on no network port. The independently installed Office companion may
run one explicit loopback-only browser service; it does not execute CLI work or change
the CLI's invocation-owned storage policy.

Any retained `better-sqlite3` use belongs to private developer tooling as an
independent oracle. It is not a Rust runtime dependency or an alternate owner
of native schema and application state.

## Repository layout

Infra reviews the layout map and its machine-readable allowlist,
[`.github/repository-layout.json`](.github/repository-layout.json). Component
ownership comes separately from [`.github/components.json`](.github/components.json).

| Home                      | Responsibility                                                                       |
| ------------------------- | ------------------------------------------------------------------------------------ |
| Repository root           | Entry points, contributor guidance, license and required configuration               |
| `.agents/`                | Contributor skills and their area references                                         |
| `.github/`                | Ownership/layout maps, workflows, shared Actions and isolated release tooling        |
| `rust/`                   | CLI, core, adapters, neutral leaves, private fixtures/release tools and archive note |
| `typescript/`             | Private developer tooling, tests and shared fixtures                                 |
| `extensions/<extension>/` | Product-owned runtimes, contracts, skills, docs and assets                           |
| `contracts/`              | Core public contracts and normative fixtures                                         |
| `scripts/`                | Shared shell/build/development helpers                                               |
| `skills/`                 | Canonical bundled user-agent guidance                                                |
| `site/`                   | User handbook and translations                                                       |
| `design/`                 | Shared visual tokens and CLI style guidance                                          |

New homes or exceptions need infra review and coordinated map/allowlist changes;
ignored local outputs are outside the tracked-file map. The
[layout skill](.agents/skills/tmt-layout/SKILL.md) owns add/move procedures and the
tracked-file guard. Handbook language exceptions belong to
[AGENTS](AGENTS.md#repository-content-language) and the allowlist.

Shared visual tokens have one owner, `design/tokens/tokens.json`, maintained by
the design lead. Its Vite projection and Rust CLI theme tests consume the same
source. Release procedures belong to the
[release skill](.agents/skills/tmt-release/SKILL.md), including the archive's
product-neutral `rust/archive/NATIVE-INSTALL.md`.

## TypeScript workspace boundary

The `typescript` pnpm workspace has one lockfile, retained Node tooling and tests,
the `@tmt/office` SPA, the `@tmt/office-service` trusted pairing service,
the private `@tmt/browser-addon` demo shell and `@tmt/remote-client` device SDK,
the private `@tmt/colab-client` WebCrypto primitive library, and
`@tmt/colab-app` local page preview.
The two Office packages live under `extensions/tmt-office/typescript` as
parent-relative members of that same workspace and lockfile. They resolve only
their declared dependencies, never root-hoisted tooling packages; Office browser
specs reach the tooling-owned SQLite oracle through `typescript/test/support`.
Vite+ owns workspace test and Office/addon/Colab Vite build, dev and preview entry points.
It supplies one Vitest runner and aliased Vite core. Each suite keeps its separate
configuration; the override also supplies that core to the existing plugins. Site and release
tooling remain outside this workspace lockfile. Vite+ also supplies the bundled
Oxfmt formatter. Each package explicitly selects its existing Vite/Vitest config's
`fmt` block; `typescript/scripts/format-workspace.mjs` owns the separate tooling
code and docs selections and expands them to absolute paths before invoking Vite+.
Vite+ supplies bundled Oxlint for workspace lint commands. Each package explicitly
selects its existing Vite/Vitest config's `lint` block, loading the shared
`typescript/scripts/lint-config.mjs` rule configuration while retaining its file arguments,
React plugin selection and warning policy. Compiler commands retain their existing owners.
Rust, root shell launchers, shared contracts and canonical skills remain outside
that boundary. `contracts/` holds core contracts only; Office contracts, vectors
and the Office skill sources live under `extensions/tmt-office/`; the proposed
colab contract lives under `extensions/tmt-colab/contracts/` (see the
[Colab extension](#colab-extension)). The Office SPA build must finish before building the embedded
native companion, followed by installed-browser acceptance; ordinary CLI builds
remain independent. Read
[Office architecture](extensions/tmt-office/docs/architecture.md) for current SPA ownership,
the chosen React/Vite/TanStack/Jotai stack and the
[Office design](extensions/tmt-office/docs/design.md) for planned trust/lifecycle semantics.
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
See [Office architecture](extensions/tmt-office/docs/architecture.md) for history lifecycle and limits.
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
The [workshop references](extensions/tmt-office/docs/references/workshop/README.md)
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
The optional `extensions/tmt-office/scripts/art` authoring tool is not a runtime decoder or validator.
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
[Office architecture](extensions/tmt-office/docs/architecture.md); exact persisted data belongs
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
Community exchange and exploration remain a [sandbox plan](extensions/tmt-office/docs/sandbox.md), not a
runtime SDK, identity registry or alternate exchange engine.

### CI selection and worker model

[`.github/components.json`](.github/components.json) is the one component map:
`owns`/`excludes` define path roots, `selectedBy` overrides ownership for scattered
files, and ordered rules select CI consumers independently. `release:false`
excludes a component from automatic cuts/publication. Only non-released components
may declare `releaseStatus:never` (never shipped) or `releaseStatus:parked`
(explicitly deferred); absence means awaiting activation. `releaseConsumers` names
packaged consumers of private components. Registration alone never authorizes activation.

`typescript/scripts/ci-scope.mjs` owns map validation, path ownership, conservative
CI selection and final gate validation. Its `releasedComponentsForPath` is the
shared release-attribution owner: released roots plus each binary's transitive
Cargo normal/build workspace dependencies; dev edges do not count. Explicit
private non-Rust consumers are additive. `cargo-workspace.mjs` supplies resolved
Cargo metadata; version inheritance/editing has its own private release-tool owner.
CI scope, ownership, binary consumption and version inheritance are separate contracts.

Selected missing, failed, cancelled or unexpectedly skipped work cannot satisfy a
required gate; empty test discovery never passes. Selection, worker, cache and
advisory-browser details live in the
[CI reference](.agents/skills/tmt-release/references/ci-selection.md).
Publication reuses the native aggregate's scope-skip proof with check-suite
provenance rather than recomputing historical selection or accepting bare skips.

## Browser add-on shell

`extensions/tmt-remote/typescript/browser-addon` is a private Chrome MV3 shell
in the existing TypeScript workspace/lockfile, not an installed native product
or a working remote channel. Its composition currently uses a clearly marked
local demo stub; no crypto, pairing, network or core operations are implemented.
The shell's types-only `remote-client.ts` is a UI port agreed with the remote
owner, not a second wire contract or SDK. Future composition may import the
public SDK; views never import its transport internals.

Browser context-menu clicks and trusted popup actions capture only a top-frame
selection, URL and title through `activeTab`/`scripting`; `contextMenus` adds the
selection entry point. There are no host permissions, page-message handlers,
external connectivity or permanent content scripts. Exact plain-text message
formatting and escaped hidden-character presentation belong to `message.ts`.
Source URL admission requires HTTP(S) without username/password; invalid sources
are refused unchanged before preview, menu persistence or intent freezing,
including restored captures and intents.
The popup freezes the reviewed agent UUID, message and operation UUID before
calling the client. Its origin-owned IndexedDB retains one frozen intent and
menu capture; restoration retains intent without sending. The shell exports journal schema
and key constants; IndexedDB ownership and worker-readiness fallback helpers are test-only.
Explicit status recovery never sends. Held operations have no request ID. Explicit retry keeps the same ID and bytes;
starting another message does not cancel submitted work. Replies render as text.
The stub's status transitions are UI evidence, never server security acceptance.

The shell's Chromium profile and loopback page fixture are disposable test
owners. Its separate workflow selects shell and consumed tooling changes,
fails on empty test discovery and does not narrow unknown-path checks. The component map assigns this package
to a private `release: false` owner and selects no native/Office jobs for it. The release generator rejects native crates
under a private owner and excludes the shell from CLI releases. Native remote
product registration remains a later slice.

## Runtime layers

The Rust crates have deliberately narrow responsibilities. Module-level rules for
the core crates live in the [tmt-core-runtime skill](.agents/skills/tmt-core-runtime/SKILL.md).

| Layer             | Owner                           | Responsibility                                                                                                                                                                                                  |
| ----------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure domain       | `rust/crates/tmt-core/src/`     | Identity, names, bindings, profiles, settings, retention, request state and native-install version policy. No filesystem, process, SQLite, tmux, network or CLI framework.                                      |
| Concrete adapters | `rust/crates/tmt-adapters/src/` | Config files, SQLite, bounded files and processes, signals, tmux evidence/transport, response input, HTTP acquisition, native release publication and managed skill files.                                      |
| Application/CLI   | `rust/crates/tmt-cli/src/`      | Core grammar, typed invocations, preflight and use-case composition, completion and the executable entry point. It chooses adapters; it does not duplicate their storage, file, installation or process policy. |

`rust/crates/tmt-command-output` owns shared command output/error values and
formatting. It renders human text through `rust/crates/tmt-cli-style`, the one
implementation of the [CLI style](design/cli-style.md) (palette, themes over the
design tokens, marks, values, messages, lists, tables, the column-width solver
`grid` that tables and extension boards share, the help registration contract and
`Interaction`). Both are leaves with no TMT dependency so extension CLIs can share
them; the architecture guard enforces that. `mark::Mark` owns each shared mark's
symbol, description and style token.

Office is frozen and lives outside core: see
[`extensions/tmt-office/docs/architecture.md`](extensions/tmt-office/docs/architecture.md).
Core keeps only the reserved `tmt office` facade (`tmt-cli/src/office_facade.rs`),
retained extraction debt tracked by #355 and #328; the architecture guard lists the
remaining `office_*` adapter modules and rejects any new one.

`rust/crates/tmt-tui` is an internal, unpublished, application-neutral presentation
leaf for TMT terminal UIs (markup admission, one Taffy geometry computation, Ratatui
paint, reusable components). It has no Squad vocabulary, acquires no terminal, clock,
settings or provider data, and takes its tokens from `tmt-cli-style` roles. The guard
permits only XML parsing, borrowed JSON, shared style, private Taffy and Ratatui, never
core, adapters, CLI or extension behavior. Squad is its sole reviewed consumer; a new
consumer or dependency goes to tmt-lead. Rules and limits: the
[`tmt-tui` skill](.agents/skills/tmt-tui/SKILL.md).

`rust/crates/tmt-cli/tests/architecture.rs` is a test-only import and dependency
guard. One reviewed manifest table owns the fixed workspace package names and
their manifest locations. The guard follows the actual Rust module tree, checks
reviewed layer edges and shared declaration ownership, and keeps an exact
dev-dependency ledger (crate, canonical name and target, with a reason per row);
aliases are rejected and the invoke leaf is guarded for every dependency kind. It
fails closed on unsupported module remapping or incomplete discovery, checks that
CLI crates reach the terminal only through `tmt_cli_style::stream`, and walks each
CLI's grammar against the style ([enforcement](design/cli-style.md#enforcement)).
It is a syntactic guard and never replaces review of behavior or effects.
`tmt-sys` is the single audited `unsafe` boundary: only `tmt-adapters` may depend
on it and every other crate forbids unsafe code.

Workspace quality checks cover the unified feature graph. Native process fixtures
build products separately to keep ordinary CLI feature isolation;
[Development](DEVELOPMENT.md#rust-checks) owns their selection.

## Public command boundary

The grammar (`rust/crates/tmt-cli/src/grammar.rs` and its `grammar/` modules) owns
core command registration, option placement and rejection, and the help projection;
each owner's parser turns its grammar into typed invocations and publishes through
`tmt-command-output`. Hidden commands parse for internal workflows but never appear
in help or completion. Handlers never search raw argv or reinterpret payload text
as flags. JSON and human output use the same typed result and status contracts.
Command dispatch, help and completion rules are in the
[extension surface reference](.agents/skills/tmt-core-runtime/references/extension-surface.md).

### External command contract (v1)

An unknown root command named `[a-z0-9][a-z0-9-]*` resolves `tmt-<name>` on PATH;
core wins every collision. On Unix the adapter `exec`s, so the extension inherits
stdio, TTY and signals and its exit status is the command's. `TMT_EXECUTABLE` names
the invoking executable. Core keeps no registry, manifest, daemon or extension
state. Only arguments after the name are passed, verbatim. Completion v1 is
optional, bounded and falls back to file completion on any failure.

### Local extension API (v1)

`tmt api` is the public, same-user process port for machine-shaped gaps in the
ordinary CLI: one versioned JSON request on stdin, one JSON resource or error on
stdout. It is neither an authentication boundary nor a daemon, batch or stream.
`tmt-adapters::api` owns envelope admission and composition; the CLI owns bounded
stdin, publication and exit status. Protocol major 1 accepts additive operations and
fields; incompatible changes need a new major. Human-shaped operations remain their
ordinary JSON commands, not duplicate API implementations. The
[extension API contract](contracts/extension-api.md) owns operations, bounds,
dispatch readiness and input safety, history and consumption semantics.

### Local MCP (v1)

`tmt mcp --identity <saved-name-or-uuid>` is an agent-launched stdio interface over
the existing exchange. The [MCP contract](contracts/mcp-v1.md) owns its wire,
schemas and bounds; `tmt-adapters::mcp` owns admission and framing and
`tmt-cli::mcp_command` composes the existing command owners in process. It adds no
exchange state, persistence or retry semantics and is separate from the private
Claude channel server.

### Extension hooks (v1)

`tmt-adapters::extension_hooks` delivers consented, best-effort lifecycle
observations and context lines to verified `tmt-<name>` executables
([wire contract](contracts/extension-api.md#lifecycle-hooks)). PATH discovery alone
never runs a hook; ownership and a stat fingerprint are re-checked before each
delivery. Capture is per connection and transactional, delivery is bounded and
never changes a command's result, and a nested `tmt` captures nothing. Extension
summaries are untrusted informational data. With no consent file a command spawns
nothing.

### Core command surface

The grammar owns primary names and accepted aliases (`ls`, `rm`, `mv`, `show`; long
spellings are hidden aliases). Core names and aliases are reserved before external
dispatch. The maintained surface is: local setup and guidance (`init`, `config`,
`completion`, `learn`, `install`); identity and binding commands (`identity`, `ls`,
`add`, `name`/`this`, `whoami`, `unbind`, `rm`, `mv`, `notes path`); the local
extension interface (`api`, `mcp`); profile and exchange commands (`role`,
`preamble`, `x`, `reply`, `result`, `inbox`, `answer`, `talk`/`send`,
`check`/`read`); `focus`; managed native updates (`upgrade`/`update`, with hidden
`__native-install` and `__native-refresh-skills`); and `extension` and the frozen
`office` facade. Output is plain-table, JSON or both from one typed result;
`identity show` without a name uses the shared verified-caller selector and never
falls back to a working directory, active pane or sole identity.

## Domain and state ownership

### Identity, names and bindings

- `tmt-core::names` owns canonical identity classification; pane-target syntax
  belongs to each host. `identity` owns lifetime and storage-only create/promote
  policy, `identity_metadata` and `identity_status` own descriptive, untrusted data
  that grants no authority, and `binding` owns evidence evaluation, retirement
  authorization and binding use cases.
- Unknown or conflicting endpoint evidence is never proof of death. Saved
  identities detach and stay offline; temporary identities retire only on
  conclusive evidence or explicit unbind, and exchanges are kept. Presence is
  observation, not routing permission: a marker or socket cannot authorize a
  different identity, and the pane marker proves ownership by IDs, never by name.
- `binding::session` separates identity-owned session preferences from
  binding-owned runtime observations, and observation writes are compare-and-set
  inside the binding transaction. Drivers own process verification, event mapping
  and driver-state persistence; core stores driver state without parsing it.
- Provider hooks supply observation only: they never create bindings or move
  identities, they run under a bounded supervised worker that always exits zero,
  and provider configuration changes only through consented `tmt setup`.
- `tmt-core::endpoint::ProcessIncarnation` (PID plus core's own start token) is the
  one value for comparing local processes. `tmt-sys` is the single `unsafe`
  boundary.
- Concrete implementations: `storage::{identities,identity_metadata,identity_status,bindings}`
  and `tmux::{metadata,evidence,binding,caller,transport}`; `binding_command`
  performs caller/target preflight and composes them. Per-module rules (names,
  rename and marker, resume and activity, consumption, setup/uninstall, foreground
  launch, caller identification) are in the
  [identity and bindings reference](.agents/skills/tmt-core-runtime/references/identity-bindings.md).

### Saved identity notes

`NotesIdentityId` (a saved identity's canonical UUIDv4) is the capability boundary
for notebook storage; display names never become paths. `ConfigPaths` is the sole
layout owner and `tmt-adapters::notes` alone creates
`<global_dir>/notes/<identity-uuid>/notes.md` with owner-only, no-follow creation.
The file body, concurrency and retention are ordinary user-filesystem concerns:
there is no SQLite copy, lock, size policy or secure deletion, and retirement leaves
notebooks in place.

### Settings and configuration

`tmt-adapters::config::ConfigPaths` is the sole application path owner;
`config::document` preserves unknown JSON fields and validates known settings
through `tmt-core::settings`; `init` creates the local file exclusively and never
opens SQLite or tmux. The global `theme` object is presentation, interpreted only by
`tmt-cli-style` (and read by Squad through `config show`); a bad theme never fails
configuration loading. Only `tmt-cli-style` names colors. Details are in the
[storage and requests reference](.agents/skills/tmt-core-runtime/references/requests-storage.md#configuration-and-theme).

### SQLite and durable exchanges

`tmt-adapters::storage` owns one private synchronous `rusqlite` connection, the
schema migrations, WAL/foreign-key/FTS5 setup, busy and transaction boundaries and
cleanup, and exposes narrow ports to core services. Migrations keep recorded names
and retention, refuse customized table definitions instead of rebuilding them, and
every core-owned table advances the durable change cursor through triggers that a
test requires new tables and columns to extend. Typed not-writable storage failures
are classified once and projected through `tmt-command-output::Failure::storage_access`.

`tmt-core::request::RequestService` owns preparation, delivery-state transitions,
exact final submission, waiter release, attention revisions and bounded retention
housekeeping; `storage::requests` owns SQL and cleanup and `request::attention` the
pure attention contract. It samples clocks at the transaction boundary, never holds
a transaction across transport, and treats uncertain delivery as uncertain, never as
replay authorization. Final bodies are immutable and terminal text is never
completion evidence. Reads never acknowledge; originator and recipient
acknowledgment are independent. `RequestRoute` separates unbound pane delivery from
the durable identity inbox, which settles `queued`. Reply notices are persisted
batch windows composed by `request::notification` and `delivery::notices`, with
detached finite workers owned by `process::detached`. Public behavior and limits are
in the [request contract](contracts/request-response-v1.md); module rules are in the
[storage and requests reference](.agents/skills/tmt-core-runtime/references/requests-storage.md).

### Tmux and process effects

`tmt-adapters::process` is the one bounded subprocess owner (output caps, monotonic
deadlines, process-group cleanup, reaping); `process::interactive` owns direct
terminal children without taking the shared process group. `tmux` uses explicit
socket/server evidence, bounded budgets and no ambient host fallback; a failed paste
or Enter is uncertain and never retried as unsent.

The CLI and the `delivery` and `pane_badge` adapters reach a terminal only through
`tmt-adapters::host::Host`; extensions never do (they read `tmt ls --json` and
`tmt whoami`), and the architecture guard rejects extension code that names the host
port, the tmux module or core's `binding`, `endpoint` or `host` model. Every host,
the built-in tmux and external drivers alike, implements `host::driver::HostDriver`,
and one binding policy in `host::driver::{status, send, focus}` decides which
evidence makes a binding present and when input is blocked. Endpoint identity is
opaque to everything but its host: `HostKind` is pure data, evidence from another
host is `Unknown`, and only `tmt-core/src/host.rs`, `tmt-adapters/src/host.rs` and
`tmux/` may spell a built-in host's name. Core's delivery policy rewrites ASCII `!` to
fullwidth `！` in any text typed into a pane. Details are in the
[hosts and drivers reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md).

### Agent drivers

Each agent driver is one declarative `DriverDescriptor` (name, executables, hook
format, display hue; pure data in `tmt-core/src/driver/descriptor.rs`, listed in
`tmt_core::driver::ALL`) plus one adapter module in
`tmt-adapters/src/drivers/<name>.rs` holding `locate` and the runtime.
`drivers::Registry` joins them in descriptor order for setup, detection, skill
targets, `run`, the runtime registry and caller recognition, and a test requires one
adapter module per descriptor. Detection reads only the filesystem and never starts
an agent. Only those two places spell a driver's name; the tmt-cli architecture test
fails on a production string literal equal to one elsewhere.

### Provider channels

An optional driver port hands talk payloads to a running agent without terminal
paste. [`contracts/claude-channel-v1.md`](contracts/claude-channel-v1.md) and
[`contracts/codex-channel-v1.md`](contracts/codex-channel-v1.md) own behavior,
limits and shipped-versus-planned status; this is the ownership map.

- `tmt run --channel` is the only entry that enrolls. The launcher picks one
  `ChannelMode`; CLI policy has no provider-name branch, and drivers own preflight,
  enrollment, the lease and recovery through the `tmt_adapters::runtime::channel`
  port (`preflight`, `enroll`, `inspect`/`recover`, `enrolled_in_pane`, `send`).
- Delivery stays in the existing routing: the driver's `send` is preferred and falls
  back to paste only after `Unsupported` or `NotSent`. An enrollment applies only to
  the exact launch that created it, an opted-in session is never `NotSent`, and a
  completed write without a provider receipt is `Unacknowledged`, terminal and never
  retried.
- A paste never runs on "no record under this binding" alone: `delivery::guarded_paste`
  and `delivery::pane_channel_evidence` are the only gates in front of the two paste
  places, and they ask each driver, by the pane address its enrollments persisted,
  whether an enrollment belongs to the pane.
- Claude (`drivers::claude::channel`) and Codex (`drivers/codex/*`) keep their record
  layout, lock and launch comparison private; every record or socket mutation proves
  the caller's generation and launch owner, so a stale launcher or server never
  replaces a newer enrollment. `tmt channel inspect|recover` renders what each driver
  reports and holds no record logic. Module detail is in the
  [hosts and drivers reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#provider-channels).

### Driver protocol

Terminal hosts TMT does not build in run out of process as host drivers (#570), and
coding agents will run as runtime drivers (#1083).
[`contracts/driver-protocol-v1.md`](contracts/driver-protocol-v1.md) owns the wire
format for both. `rust/crates/tmt-driver-protocol` holds wire types, bounded strict
`decode`, `serve`/`serve_runtime` and conformance checks over `serde` and
`serde_json` only; `rust/crates/tmt-host-grammar` (a dependency-free leaf) defines
host name, pane-ID and target grammar once, and `tmt-core` may depend on it. The
architecture guard allows exactly those edges. A runtime driver's hook path is
declarative (`RuntimeDeclaration` plus `decode_hook`, no driver process), and only
`locations`, `resume` and `usage` run the driver; runtime launch, hooks and setup
consumers stay unwired until PR B2 of #1266.

`tmt-adapters::driver_protocol` owns shared approval and bounded calls and
`host::external` owns host composition. Drivers are approved only with explicit
consent (`tmt driver install`, never product install or upgrade) into
`<global>/drivers.json`, pinned by digest, with executable ownership and fingerprint
checked before every call; a first-party driver follows its release only while it
declares nothing beyond what the user approved. Core, not the driver, decides
evidence: server identity is core's own process start token and a missing or changed
driver is `Unavailable`, never proof of loss. `rust/crates/tmt-driver-herdr` is the
first driver; its library depends only on the protocol crate, `tmt-invoke`,
`serde_json` and `semver`, and the CLI archive carries its executable as a
companion. Details are in the
[hosts and drivers reference](.agents/skills/tmt-core-runtime/references/hosts-drivers.md#external-host-drivers).

## Managed skills and native installation

Managed agent guidance is a filesystem concern separate from application state.
`tmt_core::skill_catalog` is the one list of bundled skill names; the bundle is
embedded and materialized by digest under `skill_installation`, and the
architecture test fails on a skill-name list anywhere else. Core install exposes
only `tmux-team` and `tmt-inbox`. Skill installation never opens configuration,
SQLite or tmux and never silently replaces an unmanaged path: a real directory,
mismatched name, outside link or modified source is preserved as a conflict.
Extension-owned skills arrive as bytes through `skills.install`/`skills.remove`
(explicit consent), are stored per owner and linked into the same roots; the first
owner of a name keeps it until an explicit force and core names are reserved.

Native executable installation is a different owner under `tmt-adapters::native_install`.
The fixed `Product` policy owns package identity, inventory, namespace and command
links for the CLI and the official extensions (Squad, Remote, Colab and the frozen
Office); archive data never adds a product. Every product uses one acquisition,
receipt and atomic-publication path with independent links, lock and current
release, and the active executable is the authority for a managed update: receipts
anchor to the installation prefix, not to configuration roots. Verification precedes
execution, publication runs the release verifier before the receipt so a rejection
keeps the previous release, and failure or cancellation never leaves a half-published
current release. CLI self-upgrade hands a verified candidate its own
`__native-install` under the
[handoff contract](contracts/native-install-handoff-v1.md) and then lets that CLI
run the consented extension phase; there is no rollback or second installer.
`tmt extension install|upgrade|rm|ls` is the public surface for extensions and
requires consent. Acquisition, receipts, companions, skills trees, repair and the
upgrade handoff are in the
[installer architecture reference](.agents/skills/tmt-core-runtime/references/install-architecture.md);
build, publication and verification procedures are in the
[tmt-release skill](.agents/skills/tmt-release/SKILL.md).

## Squad extension

`extensions/tmt-squad/rust/tmt-squad` builds the optional `tmt-squad` executable,
reached through the external command contract as `tmt squad` and, through a
`tmt-sq` link to the same file, `tmt sq`. It is a workspace member for the shared
lockfile and toolchain only, and is released independently (`tmt-squad-v<version>`).
Module-level reference: the [Squad developer skill](.agents/skills/tmt-squad-dev/SKILL.md).
Drawing ownership and guard verification are defined in the [Squad](.agents/skills/tmt-squad-dev/SKILL.md) and [TUI](.agents/skills/tmt-tui/SKILL.md) developer skills.

- **Seam.** Squad reaches core only through public `tmt --json` commands and
  `tmt api` (`TMT_EXECUTABLE`, else `tmt` on PATH), each call bounded by
  `tmt-invoke`. It never links a core crate or writes core state, tmux or provider
  directories itself. No TMT crate depends on Squad; the architecture guard enforces
  both directions for Cargo dependencies and source references. `tmt-squad` depends
  only on `tmt-tui`, `tmt-cli-style` and `tmt-invoke`, never on core crates. Using a
  new `tmt api` method (for example cron's planned `dispatch.create` and
  `identityHooks`) changes the seam and goes to tmt-lead.
- **Data ownership.** A squad is the core room `squad-<name>`; member fields are
  identity metadata `squad.<name>.<field>`, with no Squad membership store. Squad owns
  `<dataRoot>/squad` (`storage.root`) and disposable caches under
  `$XDG_CACHE_HOME/tmt-squad`. `squad.toml` is the user's file: agents never write it
  and Squad edits it only through its compare-and-set writer. No Squad data goes into
  `config.json` or the core database.
- **Public JSON.** Public documents are display-ready and never carry board-only data
  (home model, token-rate meter state, `usage.*` observations).

Contracts index (shape owner: the embedded lead skill
`extensions/tmt-squad/skills/tmt-squad/SKILL.md`, checked by
`typescript/test/native/squad.test.ts`):

- `sq ls --json`: with `--squad`, one document (`squad`, `sections`, row grid
  `columns`/`lines`, `you`); without it always `{squads: [...], you}`, whatever the
  squad count. Optional keys appear only when set: row `colors`; cell `token`; column
  `valueOnly`, `overflow`, `max_lines`; `hidden_columns`; `partial`/`failures`;
  `olderRequestsNotShown`; `squad.noteAnnotations`, `squad.notesStaleness`.
- `sq config show` and `sq config set --json`: entries of key, value, source and
  editable.
- `sq cron ls|show|add|edit|pause|resume|reassign|rm --json`: job documents with the
  exact message, schedule, owner and pause attribution; writes admit only the recorded
  user or the squad's current lead.

A change to row JSON updates the lead skill and runs the native row-shape test in the
same PR.

## Testing and evidence boundaries

Rust tests stay beside their owners. TypeScript native, E2E, tooling and stress
suites share `test/support`, which imports no suite; native and E2E import neither
each other nor tooling. Scenarios retain assertions; helpers own fixture
mechanics. Frozen inputs and independent SQL/schema oracles must not derive
expected results from the implementation under test.

Tests select an explicit task-owned native executable or the checkout build;
a missing build fails, with no host CLI or retired-runtime fallback. State,
provider roots, prefixes, sockets and processes remain fixture-owned. Native
process fixtures isolate caller ancestry and host tmux discovery; Docker supplies
network-isolated private tmux and deterministic peers. No host tmux server,
provider installation or global environment mutation is test evidence.

Cleanup confirms owned child/group absence before deleting fixture state;
unknown inspection, leaks and false positives fail. Signals target only verified
owned processes. Runtime tests prove CLI behavior, Docker proves transport and
lifecycle, and release tooling proves actual archives and public installation;
one layer's success cannot substitute for another's evidence.

Helper ownership, fixture publication and lifecycle details live in the
[E2E references](.agents/skills/tmt-e2e/references/test-boundaries.md); shared
commands remain in [DEVELOPMENT](DEVELOPMENT.md#native-process-and-shared-tests).

## Release boundary

Release tooling consumes cargo-dist's manifest and product-owned archives; it
shares the native runtime/linkage proof across archive, installer, upgrade and
public smoke verification. Raw executables do not prove archives or public
installation. The candidate-owned installer handoff contract is
[`contracts/native-install-handoff-v1.md`](contracts/native-install-handoff-v1.md).
Archive, installer, verifier and publication procedures belong to
[tmt-release](.agents/skills/tmt-release/SKILL.md).

### Main release cuts

A release is a product-prefixed tag on a main commit. `release.yml` admits main
pushes by cadence, with hourly backup and manual dispatch. Allocation captures
main once and reserves each released component's next alpha number from drafts
and tags; each allocated tag owns an independent pipeline. New work is measured
from the newest non-failed allocated ancestor cut, whether in flight or published.
Verification-failed drafts reserve numbers but allow replacement cuts, including
at the same main commit; they stay unpublished and do not hold later cuts or merges.

The release version is injected at build through the private `tmt-release-tool`:
only the selected version declaration and implied Cargo lock entries may differ
from the captured source. Build metadata and executable versions must agree;
nothing is committed back to main. Main retains development versions. Workflow jobs
own Node architecture selection; version injection preserves it.

Notes, migration comparison and breaking authorization share one boundary: the
component's newest published ancestor. Drafts and failed/running pipelines never
advance it. Publication creates an immutable tag on the captured commit only
after every gate passes. CLI latest converges to its highest published version;
extensions never change latest.

Automatic publication covers authorized existing alpha products only. Ben retains
stable, breaking, version-line changes and manual publication authorization.
Activating a new released product is a component-map change accepted by tmt-lead
and the owning squad lead (for example, #1418). Exact gates and owner recovery
operations belong to the [release skill](.agents/skills/tmt-release/SKILL.md) and
[main-cut reference](.agents/skills/tmt-release/references/main-cuts.md).

### Release-to-Project tracking

Delivery and publication evidence are separate. Release reconciliation rules and
procedures live in [tmt-release](.agents/skills/tmt-release/SKILL.md#project-release-reconciliation);
shared issue lifecycle definitions stay in
[DEVELOPMENT](DEVELOPMENT.md#project-tracking).

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

## Shared extension state layout

`rust/crates/tmt-extension-state` is a library-only, unpublished filesystem leaf
owned by the Remote component. Only the Remote and Colab executables consume it;
its sole dependency is the existing `nix` pin, with no TMT, crypto or storage
crate dependency. Core, adapters and the Colab model do not consume it. The
architecture guard enforces the reviewed manifest, source edges and all dependency
kinds, including aliases and target-specific dependencies.

`Layout` admits an extension-selected private subtree beneath the injected
absolute core-reported data root. It preserves existing root permissions and
canonicalizes aliases only in that trusted root. Private directories must be
owned 0700 directories; allowlisted files must be owned regular 0600 files,
opened with no-follow and nonblocking flags. Read-only lookup and lock probes
create nothing. Reads retain the caller's byte bound.

`Publication` holds a nonblocking lock through stale temporary admission,
cleanup and publication. Only the selected prefix plus 32 lowercase hex digits
matches a temporary; unsafe or oversized matches refuse and foreign names remain.
A staged file borrows that guard and links create-only after writing and syncing
its bytes. The extension retains entropy, key interpretation, error mapping and
its existing staging/removal/directory-sync failure ordering. Store schemas,
identity derivation and Remote's serve-lock proof remain extension-owned; this
leaf neither discovers roots nor accesses core state or provider configuration.

## Remote extension pilot

`extensions/tmt-remote/rust/tmt-remote` is a separate executable reached as
`tmt remote`. Core registers official installer support; the current binary
remains source-only until packaging and publication pass their separate gates. `main` owns style/foreground composition and two bounded
startup calls: capabilities and `storage.root`. `core::CoreClient` owns fixed public `api`/`ls`
subprocesses through the supplied absolute `TMT_EXECUTABLE`; no PATH fallback.
`rust/crates/tmt-invoke` is a TMT-dependency-free leaf owning executable discovery helpers and bounded waited byte captures, deadlines, per-stream caps, cancellation and explicit process-group cleanup; Remote keeps public command choices and error interpretation; its other reviewed leaves are `tmt-cli-style` and `tmt-extension-state`. Request-carried launch options preserve environment inheritance by default or explicitly clear it and copy only named allowlisted caller variables, preserving OS-string bytes and leaving the caller environment unchanged. This policy is configured through the existing invocation entry point; it supplies no memory sandbox or resource-limit guarantee. `LaunchOptions::process_group` defaults to `New`, preserving owned group creation/termination. Explicit `InheritCaller` omits group creation; a started failure detaches and returns `Cleanup::CallerOwned` without signalling or waiting for cleanup. Pre-start failures remain `NotStarted`. The caller must supervise that group. Squad's context wrapper retains its live group-leader check and whole-group abort on a started failure; capture/deadlines/caps remain invoke-owned. Ordinary Squad, Remote and Colab calls use `New`. The shared 20 ms `PULSE` bounds stop-flag observation latency; each wait is also bounded by the remaining request deadline.

`http::Door` is the colab loopback door relocated under remote (#1039). It owns
IPv4-loopback sockets, joined workers, strict HTTP/1.1 framing, exact numeric
Host admission (no alias, so DNS rebinding fails), the origin-form target
grammar that keeps the operation routes and the mount space apart,
header/connection bounds, a door-owned maximum body that handlers can only
narrow, a 32 MiB in-flight body budget reserved before any body byte is read
(bounding unauthenticated memory), absolute acquisition/response deadlines and
shutdown that closes retained sockets before joining workers. It has no
CoreClient/storage reference. A `Handler` admits each framed head (route,
Origin, cookie and body limit) before any body byte is read. `routes::Routes`
is that handler for the machine's stable `/r/<prefix>/` binding routes and the
20-attempt-per-minute unauthenticated budget; `limits` names the binding bounds.
`transport::Transport` moves append/subscribe/ack envelopes and their HTTP Origin
to one message owner. `wire` owns bounded strict JSON admission (including duplicate
members at every payload depth) and preserves exact payload bytes for signatures.
`admission` and `DoorSessions` verify live device/session authority, scope and route,
serialize one normal message per session, and durably consume its expected sequence.
`journal` owns client/machine/incarnation-scoped MAC cursors, metadata catch-up and
monotonic observed-prefix checkpoints. Subscribe/ack return signed batches/checkpoints;
controls never create journal entries. Long polls recheck live authority after each wake
and wake on session replacement/end or door shutdown. Foreground composition supplies
`operations`, which admits strict single-recipient anonymous `dispatch.create`,
journal-owned `dispatch.show`/`operation.show`, and the named public reads `agents.list`,
`identities.status`, `check`, `requests.show` and `result`. Signed discovery advertises
this implemented subset. Agent listing projects only permitted UUID/name/presence and
core-published delivery; status/check restrict UUID inputs to the grant's allowlist.
Result state follows public request history, including an empty retained final, with
no terminal completion fallback. Bounded reads hold an authorized transaction against
cross-process revocation. Other application operations remain refused. The frozen public
core envelope includes
one device provenance line. Adoption commits the exact intent digest, recipient references,
audit and direct/held state before effects. Direct sends and explicit same-ID retries
recover through core `dispatch.show` before any `dispatch.create`; read-only operation
observation never retries. Core owns immutable acceptance, its one-shot advisory wake
and enrolled-pane input protection. Remote treats wake uncertainty separately from
accepted request IDs and never infers readiness from terminal output.

`approval` owns local held-operation confirmation through the existing owner-only
control socket. `tmt remote approve <operationId>` shows frozen source/recipient/message
and requires one explicit confirmation; `cancel` and refusal create no core request.
The IMMEDIATE held claim has one winner. Grant revision/liveness, talk scope and recipient
policy are checked again at the transaction-held core invocation fence. SQLite authority
writers wait 40 seconds, beyond both 15-second core calls and their cleanup margin, so
revocation can wait for an in-flight effect to release its fence. Stop/restart
cancel unconfirmed holds. After a possible effect, failure preserves the original ID
and frozen intent as uncertain; accepted/cancelled operations release their prompt copy.
Transitions retain bounded signed metadata without copying prompt/final text into audit.

Foreground serve explicitly makes its private lock inheritable by the existing
`tmt-invoke` child. Closing the parent's file never explicitly unlocks the shared lease;
restart cannot acquire it while an original invocation survives owner death. Confirmed
child termination plus definitive core absence permits only an explicit retry of the
same ID and bytes. Unconfirmed cleanup disables writes until a fresh lease-owning run.
The effect's `dispatching` audit row is uncommitted during the core call; a crash
mid-call leaves no such row. Recovery uses the already committed adoption/frozen intent
and core's idempotent operation ID, never assumes an absent audit row means no effect.
The runner and core are unchanged. Native tests exercise real signatures/private SQLite
with deterministic public-process fixtures and a SIGKILL lease probe; they do not claim
isolated real-core/private-tmux/mock-agent acceptance.
`audit` writes bounded, sanitized metadata in the adoption/refusal transaction;
`budgets` persists fixed-window call/send/approval counters without resetting on clock
rollback. No core DB is opened. The foreground door has no default deadline;
it runs until interrupted. Colab has no door of its own; remote
mounts its owner-only socket.

`state::Layout` delegates remote's private `<dataRoot>/remote/` subtree to the
[extension state leaf](#shared-extension-state-layout). `MachineKey` retains the
lock-guarded create-only Ed25519 machine key
(`machine.key`, a software file with no hardware claim). `state::Layout` supplies
one foreground serve lock per data root. `store::Store` owns `remote.db` (SQLite) and opens only
with the `state::Serving` proof that the serve lock is held: while serve runs it
is the database's only opener and writer, and every other path (pairing, device
management) reaches remote state only through serve, over its owner-only control socket.
Without a running serve, `tmt remote devices` takes the serve lock itself, so
the database still has one opener. Its
schema history uses core's `_migrations` table (append-only, recorded names must
match, a newer history refuses) with `foreign_keys=ON`. Unlike core's shared
WAL database it keeps `journal_mode=DELETE`, since there is no concurrent
reader, and `synchronous=FULL`, so committed grants and receipts survive power
loss. Schema 1 creates the machine identity once: a UUIDv4 machine ID and the
`/r/<32 lowercase hex>` route prefix, both stable across restarts and neither a
credential. Schema 2 adds `grants`, with one live grant per device key. Schema 3 adds per-device session/run IDs and independent decimal-string client
and machine counters. A signed session open replaces its row, starting client input
at 1 and machine responses at 2 after the open response. Counter exhaustion never
wraps. These rows survive interruption, but only in-memory live sessions authorize
normal messages; restart never revives an old row. `authority` consumes the existing
grant fields as typed direct/hold and all/selected-agent policy, refusing malformed
or unknown authority without changing pairing's producer. Unsafe state fails closed
before the door binds. Schema 4 adds metadata streams, separate request ownership,
fixed-window budgets and immutable audit records. Journal authorization reads the persisted
grant inside its IMMEDIATE transaction. Metadata lasts at most 24 hours/1000 entries
per client; acked prefixes compact sooner. Ownership reads expire after 30 days without
renewal on reads/ack. Expired records remain bounded ID fences, so pruning never permits
re-adoption; at 1000 ownership records/client new adoption refuses. Frozen pending intent
is bounded to 64 MiB/client and 256 MiB total. Audit retains at most 30 days/the newest 100,000 records; pruning and append share
the adoption/refusal transaction. Budget keys are bounded to 100,000. Write or
ownership/budget capacity failure refuses before adoption. Public operation transitions
and frozen-payload release are not wired yet. Remote retains its state error codes
and messages while delegating filesystem operations to the shared leaf.

`control::Control` binds `<dataRoot>/remote/control.sock` (0600, in the 0700
state directory) under the serve lock and speaks one JSON object per line; a
stale socket from an earlier serve is replaced, anything else refuses. Its
operations are `pair` (for `tmt remote pair`) and `devices`, `revoke` and `rename` (for
`tmt remote devices`). `pairing::Pairing` owns the single offer
of the current run (window): a random 16-byte code and 128-bit challenge held only
in serve's memory, a ten-minute deadline, and its phase (open, pinned candidate,
confirmed). `/pair` admits strict enrollment JSON (exactly the contract fields,
strict base64url, the request Origin equal to a browser/add-on's proposed origin
and absent for `cli`, and a `browser` origin equal to this door's own origin),
verifies the full HMAC and the possession signature, pins
the first valid candidate and reports it to the pairing client with its four
fingerprint words. Identical candidates coalesce and competing ones refuse;
three failed code proofs, owner refusal, the pairing client leaving, expiry, a
replacing offer and stop end the offer and erase the code. A `/pair` request
waits for the owner up to 20 seconds, then answers 202 `{"state":"pending"}`
so the device retries the exact candidate; every refusal is a generic 404.
Confirmation inserts the default grant (all agents, the default scopes,
`direct`, no expiry) in one transaction under the offer lock, derives
`K_response` and `serverProof` over the exact receipt JSON, erases the code and
keeps only the candidate, receipt and proof for exact-retry recovery until the
original deadline. A failed grant write ends the offer with no grant.

`pages::Pages` serves the browser assets at the door root, disjoint from `/r/`
and the route prefix: the pairing page at `/pair/<descriptor>` (strict CSP, same-origin
script only), the device SDK module at `/sdk/remote-v1.js` and `/sdk/mount`,
which answers a same-origin page's path with this run's machine and window and
the extension whose mount contains it, from `Mounts::extension_of`. That lookup
scopes honest use only; mounted extensions share one trust domain. The SDK
module and page are embedded with `include_str!` from the crate's `assets/`;
`remote-v1.js` is built from `remote-client` (below) and Code quality rebuilds it
and fails on any difference.

`session::DoorSessions` admits the signed `session.open` control on
`/r/<prefix>/append`: exactly the envelope fields, this machine and window, a live
grant (not revoked, not expired), the envelope and request Origin equal to the
grant origin (a `browser` grant to this door's own origin; none for `cli`), a
timestamp within 60 seconds, a `{clientNonce}` payload whose nonce was not used
by that device within two minutes, and the device signature over the canonical
bytes. It answers a machine-signed response and, for a `browser` device, sets
the `tmt_door` cookie (256-bit token, `Path=/r/<prefix>/x/`, HttpOnly, SameSite=Strict)
whose SHA-256 is all serve keeps. Sessions live in serve memory, one per device:
a newer session, revocation, 12 hours without use or stop ends one, and
reopening is another signed `session.open`. `DoorSessions` supplies one monotonic
idle clock to each `mount::SessionState`; session creation, mounted requests,
tunnel activity and idle checks share it. Production uses `Instant::now`, while
the socket-test harness can freeze and advance it without changing wall-clock
signature/grant admission or production bounds. Every refusal is the generic 404.
`/r/` routes refuse any cookie, so a cookie alone never reaches an operation or
pairing. `devices::Devices` lists grants and revokes one by disabling it and
advancing its revision before acknowledging, then ends the device's session.
Rename shares pairing's pure name validator, changes only presentation, advances
the revision only when the name changes, and ends old-revision sessions for silent
reopening. Revoked grants cannot be renamed. Neither mutation exceeds the JSON
integer revision bound.

`devices::DeviceEvents` owns one joined worker over current durable grants: each
sweep delivers disabled tombstones and current names through `mount::Mounts` on
the existing owner-only socket. There is no journal, cursor or persisted delivery
state. A committed mutation wakes the worker; successful periodic replay recovers
extension restarts, and failed sweeps use bounded backoff. Socket I/O happens
outside the store lock and command acknowledgment. `Mounts` owns private socket
admission, nonblocking connect, bounded HTTP callback and 2xx acknowledgment;
the callback's reserved subtree is refused by browser forwarding and its marker
header is never forwarded from clients. Consumers own durable revision deduplication
and extension cleanup as specified in the
[device-event contract](contracts/remote-channel-v1.md#extension-channel-api).
Shutdown wakes backoff and joins the bounded in-flight attempt before state release.

`site::Site` is the door's handler: the mount space `/r/<prefix>/x/` goes to
`mount::Mounts`, `/pair/` and `/sdk/` to `pages::Pages`, all others to the
`/r/` binding, whose exact routes never overlap the mount space; the root `/x/`
is a plain 404. Mounting under the unpredictable machine prefix keeps the door
cookie (scoped to it) from other loopback listeners at a guessable path, and
mounted replies keep `no-referrer` (or a narrower `same-origin`) so the prefix
does not leak in `Referer`. Mounts forward `/r/<prefix>/x/<extension>/` to
`<dataRoot>/<extension>/door.sock` only for allowlisted extensions (exactly
`colab` in this slice; a general enabled-extension registry is later work)
and only when that socket and its directory are owned by the user, grant
nothing to group/other and are not symlinks. Remote owns admission: the
door's exact Origin for every request except top-level GET navigation and
for every upgrade, a method allowlist and per-extension request/reply bounds.
It forwards the path below the prefix, a small header allowlist and a
`tmt-mount` header, never the door-session cookie; it adds `tmt-device-context`
(ASCII JSON of the extension channel API's owner device context, including the
grant's device public key) only when the
`mount::Sessions` port (`DoorSessions` in serve) resolves the door cookie to an
owner session, and never copies one from a client. Each resolution rechecks the
grant's revocation, revision and expiry; without a live session a request is
forwarded as non-owner. A tunnel opened under a session closes within one
100 ms poll when that session ends, and its traffic counts as session use.
The extension owns its reply: status, content type, CSP and other headers pass
through; the door only fills absent security defaults and drops `Set-Cookie`.
Replies stream one chunk at a time. WebSocket upgrades are spliced as unparsed
bytes in both directions with bounded per-direction buffers, on a tunnel thread
outside the door's edge sockets so open pages cannot starve `/r/`, pairing or
page loads. Each extension has its own tunnel cap (colab: 16) and idle bound
(colab: 120 s without bytes either way); a full pool refuses the upgrade with
503 and `retry-after`. A tunnel also ends when either side closes or pending
bytes stall past their bound, and door shutdown closes and joins every tunnel
before its workers. `Sec-Fetch-Site: cross-site` is refused when present. A
missing or unsafe socket is 404, an unreachable one 503 and a malformed
extension reply 502. Mounted traffic makes no core call and never reaches `/r/`.

`canonical` owns pure decoded-value local-v1 envelope framing and the
`tmt-device-pair-v1` device enrollment and possession framing (kinds `addon`,
`browser` with a loopback door origin, which `Pairing` binds to this door, and
`cli`; the device proposes
no agents, scopes, mode or expiry), the pairing-code text codec (26 base32
symbols, separators limited to ASCII spaces and hyphens) and the four-word key
fingerprint over the pinned BIP-39 English list in
`extensions/tmt-remote/rust/tmt-remote/assets/bip39-english.txt`, the
`tmt-ext-cert-v1` extension key certificate bytes, and the mounted
extension-name grammar that `mount` also uses. `crypto` owns strict Ed25519
verification, full HMAC-SHA256 verification and pure `K_response`/`serverProof`
derivation. Neither module has I/O, clock, storage or CoreClient access. Pairing and message
admission compose these pure primitives; valid bytes alone grant no authority. Remote-generated IDs remain UUIDv4.
Byte construction and valid signatures establish no authority.
Rust tests consume the independent Python canonical fixtures read-only; Rust-owned
RFC/Python/WebCrypto vectors exercise cryptographic validity separately, including fixed
extension-certificate signatures and domain, extension, purpose, key and time binding. The codec
dependencies are the contract's pinned Ed25519 and HMAC primitives, the existing
pinned SHA-256 dependency and the workspace `base64` engine configured for strict
unpadded base64url (no padding, no trailing bits), whose refusals have shared
oracle vectors. Real Chrome MV3 security and browser
interoperability remain later gates; local Node conformance does not replace them.

[`contracts/remote-channel-v1.md`](contracts/remote-channel-v1.md) owns the proposed
remote channel: one device identity, trust grants (direct by default, hold opt-in),
the admitted operations, the extension channel API (device context, route mounting,
opaque relay, operations, agent status) and backends/deploy. Extensions such as colab
are apps on remote and consume that API instead of shipping their own door, sign-in,
pairing or backends. Pairing/authentication/log/SDK behavior remains proposed until
its implementation slices land; the `canonical` and `remote-client` builders below
follow the channel contract's device enrollment, receipt-proof and fingerprint rules.
`firestore` and `cloudflare` are not permitted until their edge admission and
encryption profile are specified. Core never owns a listener or remote state. Core recognizes Remote as an
official installation product. Its archive embeds the pairing page, SDK and wordlist
without companions or skills. Packaging and publication
retain separate gates owned by the [release skill](.agents/skills/tmt-release/SKILL.md).
For shell ownership, see the [browser add-on shell](#browser-add-on-shell).

The private [`remote-client`](extensions/tmt-remote/typescript/remote-client/README.md)
TypeScript module owns decoded-value envelope, device enrollment, possession and
`tmt-ext-cert-v1` signing-byte builders, the `K_response`/`serverProof` HMAC inputs,
pairing-code decoding and fingerprint indexes, with independent exact-byte/SHA-256
fixtures shared with the Rust tests. Its `device` module is the device SDK: a
non-extractable WebCrypto Ed25519 device key (the caller persists the opaque
handle), the pairing link parser and client that accepts the machine key only after
`serverProof` verifies, the `session.open` client that verifies the machine-signed
response, and extension key certification. Network access goes through an
injected fetch. Its browser entry (`src/browser.ts`) is what the door serves:
`vp build` on the aliased Vite core in library mode bundles it, the canonical
builders and the pinned BIP-39 list into one unminified ES module in the crate's `assets/`. It runs the
pairing page (fragment removed first, words shown before the owner confirms, the
key's opaque handle kept in this origin's IndexedDB) and gives mounted pages
`reopenSession`, `operations(session, {timeoutMs?})` and `certifyKey`, whose extension
comes from `/sdk/mount`, never from the caller. The operations helper exposes
`dispatch.create`, `operation.show`, `result` and `agents.list`; scope-free
`capabilities` is used internally for sequence synchronization. A private session
channel retains the signer, pinned machine key and independent sequence counters;
all helper instances for that session share one serialized request lane. Responses
must verify their signature, audience, session, operation, correlation and increasing
machine sequence before their state is exposed. The caller owns durable dispatch IDs
and intent; the SDK freezes serialized payload bytes in memory before signing and
stores no dispatch intent in IndexedDB. Unknown outcomes throw the exported
`ClientError`, preserving the original dispatch ID. Recovery observes
`operation(originalId)` in the existing session. After unknown outcomes or signed
refusals, the next call first probes read-only `capabilities` at n+1, retrying
once at n only on a verified sequence refusal, then performs the caller's call. It never guesses a third sequence, reopens or
automatically dispatches. Exhausted recovery reports `sequence_unavailable` so the
caller can reopen explicitly; reopening ends that session's live sync tunnel.
Verified pre-effect refusals return a refused state on send/observation; read calls
throw the exported typed `RefusalError`. Generic pre-admission 404 signals
`REMOTE_SESSION_ENDED`. The SDK README owns the exported error unions and caller
recovery procedure. Listing forwards optional core-published delivery unchanged.
Every `certifyKey` call signs a new `tmt-ext-cert-v1` certificate
with the same device key and current `issuedAtMs`; verifiers own freshness.
The paired record stores no certificate cache; legacy records with an extra
`certificates` field still load without migration. The browser SDK exposes no
principal: mounted pages ask their extension backend, which uses the door-forwarded
`tmt-device-context` for that request. Remote-generated IDs remain UUIDv4, as defined by the channel
contract. Syntax validation establishes no authority. It uses standard UTF-8 and
WebCrypto primitives and runs in the existing Code quality job: the independent
Python oracle must pass before the workspace-pinned Vite+ test runner runs, and the SDK
tests drive it against a node:crypto stand-in door. A Playwright Chromium smoke
(`test:browser`, in the path-filtered Remote pairing page workflow) pairs a real
browser with a real `tmt remote serve` and checks the cookie, the forwarded
device context, both certificate purposes, silent session reopening, and a direct
send/operation/result read using a deterministic public-core fixture, then verifies that
revocation removes owner context and refuses reopening while retained signatures remain valid.

The #1055 acceptance suite uses E2EFixture through `harness.ts`.
Its `remote-device` peer is test-only, pinned to independent Python/WebCrypto
vectors and imports no SDK. `remote-owner` consumes fixture coordinates and owns selected
real core/Remote binaries, isolated HOME/XDG/private tmux, HTTP and joined process teardown.
Its transparent test-only `TMT_EXECUTABLE` wrapper forwards exact argv/stdin/actual output;
grants may be seeded only in Remote's database after serve and owned core children stop,
never in core storage. Integrated acceptance is limited to #1055's six bullets plus one
permitted/refused read scenario.

## Colab extension

Colab (`extensions/tmt-colab/`: `tmt-colab`, `tmt-colab-model`, `@tmt/colab-client`,
`@tmt/colab-app`) is an activated native extension whose executable embeds the app.
[colab-v1](extensions/tmt-colab/contracts/colab-v1.md) is the normative contract; the
[tmt-colab skill](.agents/skills/tmt-colab/SKILL.md) holds module knowledge and procedures.

- **Layer.** Colab is an app mounted by Remote, with no door of its own. `tmt-colab serve`
  listens only on the owner-only socket `<dataRoot>/colab/door.sock`. Remote mounts it at
  `/r/<prefix>/x/colab/`, owns Host/Origin, cookies, pairing and grants, forwards the
  verified device as `tmt-device-context`, and never forwards the reserved `/.tmt/` subtree
  from a browser. The server stores ciphertext and never decodes Yjs.
- **Dependency direction.** `tmt-colab` depends on `tmt-colab-model` (pure codecs and fixed
  crypto), the `tmt-extension-state` leaf, `tmt-invoke` and `tmt-cli-style`; the browser
  depends on Remote's served SDK (`/sdk/remote-v1.js`). Never `tmt-core`, `tmt-adapters`,
  `tmt-remote` or Office; core is reached only through `$TMT_EXECUTABLE api`. The
  architecture guard enforces the dependency set, that only `tmt-colab` consumes the model,
  and that only `decoder/child.rs` imports `yrs`.
- **Seams.** With Remote: the mount socket, `tmt-device-context`, the device-events callback
  and the browser SDK; the Ask agent sends through Remote's SDK operations helper as the
  paired owner device, with no native bridge, ledger or migration. After a Remote restart,
  Colab's public recovery entry reopens the paired device's session once, through the same
  tab claim; all other app assets stay owner-gated. With core: `Product::Colab` registers
  the executable with the installer, and the app is served from `serve --app-dir`, else
  bytes embedded from `TMT_COLAB_APP_DIR`, else the checkout's Vite output.
- **Renderer invariant.** Parent chrome allows only self-hosted scripts and styles (no
  `unsafe-inline`). Author HTML runs only in `renderer.html` inside an opaque
  `sandbox allow-scripts` frame whose own policy permits inline scripts and styles but no
  network. This contains author code; page self-navigation can still leak a request.
- **Plaintext invariant.** Page source and export are root-local: only the isolated decoder
  child decodes Yjs, no route serves plaintext, and the browser Worker is resource
  containment, not a security sandbox.
