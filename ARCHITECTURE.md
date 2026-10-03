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

`rust/crates/tmt-tui` is an internal, unpublished presentation leaf for TMT
markup. Its version-1 structural admission accepts bounded XML and produces a
template with source locations, not a renderable scene. It refuses declarations
and excessive depth before tree allocation and bounds parser nodes. Static
classes, wrap and literal tokens compile into `style::CellStyle` during admission,
including every repeat template. Tokens use `tmt-cli-style::theme::Role`; no
palette is resolved or copied. `binding::compile` eagerly checks an explicit
application schema, lexical dotted paths (root `$` and repeat aliases), stable
IDs and application-owned source/format handles, including empty repeat bodies.
`materialize` borrows `serde_json::Value` data and checks referenced value kinds;
it acquires no data. Missing required paths are errors; null scalar text is
absent. Direct binds retain display text; only the application's source adapter
applies formats. IDs use scoped components, never collection positions; semantic
row IDs remain separate. Expansion admits at most 20,000 nodes, 20,000 repeat
iterations and 8 MiB of aggregate text/ID bytes (including the duplicate-ID
registry). Stable IDs are nonempty, nonnumeric strings of at most 256 bytes.
Borrowed text is charged before copying; source callbacks own their allocations.
`geometry::layout` maps materialized styles into one private Taffy 0.7.7 flex/grid
computation. It borrows node identity/style, injects scalar intrinsic/wrap metrics,
and returns whole-cell rectangles, content, ancestor clips and overflow/cut intent.
Text measurement and painting share its recorded integer width; fractional
spare cells are styled blanks inside hits; alignment uses the recorded width.
A cut grid cell preserves its logical width/height, exposes at least four visible
cells or hides whole; painting fits each visual line to the clip with end/middle
ellipsis. Squad owns priority selection before geometry, not Taffy: optional
tracks fit their mapped minimums whole or step aside; only non-priority overflow
can cut. Markup and board percentages use CSS content-box shares with gaps in
addition. CLI lists retain their after-gap percentage base and largest-remainder
rounding; no percent adapter or correction loop joins these surface policies. The guard permits XML parsing, borrowed JSON, shared style,
private Taffy geometry and Ratatui buffer painting, never core, adapters, CLI or
extension behavior.
`text` owns markup and board grapheme measurement and fitting; `paint` consumes geometry
preorder into a caller-owned Ratatui buffer. Both use `Cell::text_width`, never the
rounded spare cell. Cuts ellipsize already measured lines without rewrapping;
wide graphemes crossing clip edges leave styled blanks. Theme/Depth are injected,
roles inherit and resolve through the shared screen adapter. The caller supplies
the complete selected-role style: Squad's Look remains the selection policy owner.
Hits borrow scoped IDs and semantic row IDs, inherit identity, intersect visible
buffer clips, omit zero areas and resolve in reverse paint order. The application-neutral
`app` layer owns base focus, one replaceable modal and top-first event routing:
unhandled modal keys/mouse are captured, closing events never replay into the base,
and Ctrl-C returns a quit effect. The caller retains item cursors, data, effects
and terminal lifecycle. `components` owns opaque square-border modal chrome,
fixed footer/status/position slots, visual-line scroll/clamp/reveal and typed
key-help sections. Wrapped fixed text is measured by the existing text owner
before the scroll viewport is reserved; nowrap slots keep their one-line default.
Authored modal cell height controls demand within placement bounds; body
references still fill the application body. Its surface compiler lowers component markup into the existing
bounded primitive binding and geometry pipeline; generated templates are checked
against the same depth/node limits. Component IDs are static scoped IDs outside
repeats in this first API. Key help measures one display-cell label column across
all sections and stacks descriptions when fewer than 20 cells remain. Rendering
injects Theme/Depth and the existing selection policy, and returns current-frame
clipped hits. No terminal acquisition, clock, settings persistence, markdown or
provider acquisition lives in this leaf. Components implement the
[full-screen interaction guideline](design/cli-style.md#full-screen-interaction);
application-owned descriptions and effective bindings supply their text.
`ListState` reconciles stable row identity across refresh/reorder, chooses the
nearest enabled survivor after removal, and reveals the whole wrapped row.
List/table admission requires a row template and typed `id: StableId` and
`disabled: Boolean` fields; table cells use the existing grid tracks. Disabled
and empty rows cannot activate. Ordinary panes use `collection::compile/render`;
modal lists and picker query/list/footer slots use `surface::compile/render_list`.
Their clipped row maps retain the painted model and scroll offset; stale mouse
geometry cannot activate. `Picker` owns bounded grapheme query editing and returns
query changes, selection changes, confirmation or cancellation. The application
filters projected data, routes focused fields, and owns previews, saves and rollback.
The rendering pipeline keeps semantic roles under caller-owned selection styling,
including reverse/bold fallback. Squad's remaining surface migrations stay
consumer-owned #1465 work; Squad help uses the modal, scroll and key-help
components.
Squad is the sole reviewed product edge, through a normal dependency. Its row
compiler binds already projected display values into bounded admitted cells,
without acquiring or formatting sources. Occurrence IDs contain tab, authored
section slot, source squad and member UUID, followed by static line/column keys;
member order is never identity. `App::shown_tab` supplies the retained view owner
while another tab loads; resize/search never substitutes the requested tab.
UUID-free display rows have no actionable IDs.
Taffy is the board's only row sizing owner and `text` its only scalar fitter;
`grid::fit/fit_lines` remain only for CLI lists. Squad retains styled row spans,
selection, scrolling and actions; full markup paint/hit adoption is still #776.
The private component has no release; its inherited version/lock entry follows
the workspace, while product notices include only their actual dependency graph.

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
`tmt-sq` link to the same file, `tmt sq`. Its command name is fixed, never taken
from argv[0], so both spellings share one help text, error set and completion.
It is a workspace member for the shared lockfile and toolchain only. Its reviewed
runtime TMT dependencies are the neutral leaves `tmt-cli-style`, `tmt-invoke`
and `tmt-tui`. The production row compiler binds projected display values; the
test-only source adapter still borrows acquired `Member` values and reuses
`ColumnSource`/`Format`. Neither compiler acquires core/provider data or sorts.
No TMT crate depends on Squad; the architecture guard enforces both directions
for Cargo dependencies and source references. Squad reaches TMT
through `TMT_EXECUTABLE` (or `tmt` on PATH), using public `--json` commands and
`tmt api`, with `runner` mapping results/errors to `tmt-invoke` for bounded capture.

Squad membership has no extension store. A squad is the core room `squad-<name>`.
Member fields are
identity metadata `squad.<name>.<field>`, so one identity can belong to several
squads and removal clears exactly one namespace. The per-member `note` field is
retired: `membership::parse_change` refuses a nonempty `note=` with
`SQUAD_NOTE_RETIRED` and a notebook/task/pending hint during all-pairs validation,
before core calls or writes; empty `note=` still clears stored metadata.
`Squad::roster_with` excludes legacy note values from member fields without
mutating storage. Row JSON omits `note`; list text, board rows and detail do not
render it. `rows::OWN_FIELDS` retains the reserved name, so providers and bound
columns cannot reuse it. Member context belongs in each member's own
saved-identity notebook (`tmt notes path --identity <member>`).
Leadership is the reserved
identity metadata key `squad.<name>.lead.marker` (`true` or `false`), outside the
user field/column grammar; `role` and `lead` remain ordinary free-text fields.
The roster parses that key into `Member::lead_marker` and omits it from public
`fields`, so field enumeration and copy/provider/column consumers never see it.
`Member::is_lead` reads that value, falling back to `role == "lead"` only for an
unconverted member. Before applying `set` pairs, the membership owner records
a marker only when a role write would change that legacy-derived leadership.
Conversion is per member because sequential core writes can fail partway through
a squad-wide conversion. New additions need no marker unless their existing
metadata would make them lead; in that case `add` writes `false` before joining.
`squad lead` preflights the core metadata capacity for every required marker
before any write, records `true` before joining the new lead, then sets previous
leads to `false`, preserving all role text and squad membership. Clearing with
`squad lead --none` preflights and clears those markers without selecting or
joining anyone. Former leads remain members until explicitly removed with
`squad rm`. A concurrent metadata write after
preflight can still split this sequence; it is not a transaction. Removal clears the reserved key with the
rest of that squad's namespace. A required new marker at the identity metadata
capacity limit returns the existing core error before the role pairs or join,
rather than silently changing leadership. Reads never convert state.

The package exposes a Squad-owned `cron` library. `cron_command` composes the
management grammar/output through one `cron_service` shared with future clock and
board callers. `cron::schedule` owns positive elapsed intervals, fixed
local times and five-field cron parsing/next-slot math. Named time zones use
Jiff's system/zoneinfo database without a bundled database. Fixed local times
skip DST gaps and choose the first occurrence in a fold; elapsed intervals keep
their stored anchor and duration. Calendar day-of-month/day-of-week restrictions
use the standard alternative rule unless either field starts with `*`.
`cron::store` owns the versioned `<dataRoot>/squad/cron/jobs.json` document,
including per-squad counters that survive removal, exact message bytes, room and
owner references, schedules, revisions and pause attribution. Its caller supplies
the absolute `storage.root` data root and admits core UUID references. It does
not resolve identities, decide permissions, dispatch messages or track runs.
Reads of an absent store create nothing. A stable, nonblocking `jobs.lock`
serializes reads and mutations; validation and file sync precede atomic rename,
followed by directory sync. Failed publication preserves the previous document;
a directory-sync error after rename reports an uncertain commit for rereading.
New directories/files use 0700/0600 permissions. Invalid existing state fails
explicitly rather than resetting counters or overwriting it. No cron data goes
into `squad.toml` or the core database.

`cron_service` owns list_jobs/show_job/apply, explicit recorded/verified CronActor admission and
room/owner/revision revalidation. Existing mutations retain a JobKey (squad name,
room UUID, c-id) and expected revision; add retains its selected room UUID.
Admission and stored comparisons run inside the stable jobs lock. A user or the
current squad lead may write; an identified ordinary member never falls back to
the user. Reads do not require that permission. Manual and scheduled send callers
obtain admitted immutable jobs through the same locked path, with anonymous
scheduled admission requiring an on job and active owner membership. No dispatch
runs under the jobs lock; core roster/identity state can still change after that
snapshot, and core owns final dispatch admission. `Core::api_write` reuses the
bounded process owner with an explicit identity or anonymous envelope.

Owner hook registration (`identityHooks`, consumer `squad-cron`) precedes job
publication; a failed publication can leave a harmless unused reference. List,
show and apply process one pending retirement page of at most 16 hooks. Future
clock ticks call that same drain. A still-matching room/job/owner reference becomes
paused/no owner with a new revision before hook acknowledgment; obsolete hooks
are acknowledged without editing a reassigned or removed job. Add/edit/pause/
resume/remove notify the owner; reassign notifies old/new owners, with the message
for the new owner. The actor's own notice is suppressed; retirement notifies the
current lead anonymously. These post-commit announcements use deterministic
room/job/revision/action/recipient operation UUIDs. Failures are returned as
warnings, without rollback, outbox or recovery journal. Interruption can lose a
notice. Reassignment retains a pause; resume requires a current owner. Read
projections exclude jobs belonging to retired/replaced rooms while preserving
their records and counters. No clock commands or board integration ship here.

`ls` (alias `status`) joins
one `rooms.roster` snapshot with `ls --room` presence. Presence is read first so
core reconciliation retires dead temporary identities before the roster snapshot;
a member joining between reads has unknown presence until the next load. It
always returns one
`sections` shape: without user-defined sections, a single untitled section.
User-defined sections (`[[squad.<name>.section]]`: title, filter, sort) replace
the single list, and rows that match none follow in one untitled section so
nobody is hidden. The document carries the board's row grid (`rows`: `columns`
and `lines`). A column's `from`/`format` (`source::ColumnSource`) reads the
member's public projection: the `ls --room` row it already joins (`cwd`,
`target`, the normalized `resume`) and roster metadata, which `rooms.roster`
returns unprefixed only when a column reads `meta.<key>`. `status::document`
writes each bound value into the row's field of the column's name, with its
number for sorting, before sections, filters and sorts read it, so the board and
`ls` show one value and a binding adds no core call. It also owns cell color
resolution: a column's numeric `color` thresholds (`rows::Threshold`, validated
theme tokens, strictly increasing) over the bound number or the field read as a
number, else a field provider's token, which `provider::apply` keeps only when
it names a theme token. The row carries the result as `colors` (`{field: token}`,
omitted when empty). `config::States` owns state color and rank resolution:
exact entries (including layout presets) win entirely, else the first ordered
`[[squad.<name>.state_patterns]]` glob, else no color and the default rank.
Explicit sorts precede preset sorts at the same number; unspecified pattern
sort ranks after ranked states. The compiler validates theme tokens, sort
0-999, booleans, unknown settings, and caps of 64 patterns and 256 UTF-8 bytes
per nonempty match with indexed config errors. Its bitset NFA consumes Unicode
scalars with fixed-size transitions, no backtracking or dependency: `*` any
run, `?` one scalar, other characters literal; optional case-insensitive
matching compares each scalar's lowercase form. `status::document` alone
publishes the resolved state token as `colors.state`, ignoring state thresholds
and provider colors. Other color keys still come from thresholds or providers.
The board consumes these tokens rather than keeping a second state-color map;
aggregate lead rows retain their original squad's resolved token. `ls` text
stays uncolored and shares state sorting (including section sort keys) with the
board. State text and attention classification are independent of decoration.
Field providers
(`provider`, `[squad.<name>.fields.<field>]`) run the user's own program per
member through `runner` with the run-binding argument rule
(`Template::fill_argument`: one argument per template, no shell, a value that
would start an argument with `-` refused), 4 at a time, bounded in time and
output. `provider::Cache` keeps each value with the argv that produced it in
`$XDG_CACHE_HOME/tmt-squad/fields/<squad>.json` (atomic replacement via
`cache`: a 0600 file in a 0700 directory), so a changed input never shows an
old value. `preset = "github-pr"` is a fixed `gh pr view {pr_link}` argv whose
JSON `provider::github_pr` turns into `#<n> <state>[ · <review>]`; anything
else from `gh` is a failed run; `provider::apply` writes
current values into member fields before the document is built, `?` plus the
row's `failed` list after a failed run. Readers never run providers: `ls` reads
the cache (`--refresh-fields` runs what is due first), and the board hands each
load's members to one fetcher thread that runs due work off the paint path and
again at the shortest `every`; a save moves the cache directory's stamp, which
`board::changes` watches, so the board reloads early. `markup::Grid` compiles the board's covered tracks
and configured spans through TUI admission and one Taffy grid computation.
Squad resolves configured CSS clamp bases and selects priority tracks before
sizing; growing tracks reuse `rows::NARROWEST` as their default minimum.
The grid retains geometry's logical text widths and clips for fitting; no
arithmetic span solver or scalar `grid::fit/fit_lines` remains in the board.
`rows::Column` still uses `grid::Basis` for cell/percent configuration, with
cell bounds. `rows::Rows` owns prefix coverage, including empty cells; original
span positions survive hiding. Its cells optionally carry a typed shared `Role`,
read from `token` with strict semantic-name validation and published only when
configured. `markup::row_values` admits that token on each cell; the board
resolves the admitted role through Look before projected field decoration.
Missing/empty values and failed providers without projected colors keep Dim;
stale-row inheritance remains intact. `Look::row_span` still overrides cell
colors and Dim for reverse selection. Team alone opts in with `waiting` on its
pending cell; text values, geometry and CLI list styling are unchanged.
Uncovered columns remain projection sources; their JSON metadata
adds optional `valueOnly: true`, omitted for covered columns. `Column::display`
ignores their sizing settings so flat text lists retain natural values. No shared
CLI solver contract changes. Lists keep after-gap percentages, largest-remainder
rounding, cell bounds and growth; their hiding recomputes the shown set.
`grid::fit_lines` remains the list wrapping owner. The board's immutable-view
width/search cache retains admitted projected row cells and geometry together;
selection-only frames change styles without rebuilding templates or sizing. Column metadata preserves percent strings and adds `overflow`
and wrap `max_lines` only when opted in; full row values never change.
`rows::ListSizing` chooses the text sizing policy once from shown column
settings: without percent/overflow it keeps legacy list sizing and complete
piped values. Opt-in text lists decode only projected display settings through
`rows::Column::display` and use the same grid solver/fitter. A pipe's budget is
summed natural data widths plus gaps before priority hiding; such lists may
truncate, wrap or hide columns. The existing list/table owner still renders
sections and styles; no parallel layout engine is introduced. With `--squad`, `ls` returns that squad's document;
without it, always `{squads: [...], you}` in name order (even for one squad or
none), so a script's shape never depends on how many squads exist. Commands that
change state still require `--squad` when several exist; `filter` owns a bounded boolean language over a row's text
fields, and every section is validated before output. `tmt squad board` renders
the same document with ratatui over crossterm; `board::terminal` owns raw
mode and the alternate screen behind a `Screen` trait, restoring on return,
error, panic (via the panic hook) and TERM/HUP (signal-hook). One refresh thread
loads snapshots off the input loop, collapsing queued requests, so keys act on
painted data. Input, snapshots and deferred tab attention share one event channel;
a snapshot wakes the painter directly. The input loop rebuilds only after a
state/input/resize change or when displayed clock text or the delayed spinner
changes. Each immutable view owns disposable markdown wrapping and grid-width
derivations keyed by effective pane width (and grid search); markdown also keys
its styled lines by the active look so theme previews repaint them. Replacing the view
invalidates them, and the scroll renderer copies only visible lines.
The completed-request meter has separate 5–10 second deadlines on that same
worker. `board::rate::Input` captures the observed roster UUIDs and public
`resume` values before section shaping. A normal named-squad load carries that
input; while idle the worker reads only `ls --room --json` for those UUIDs,
without providers, notes/history or staleness publication. Full loads take
priority. Usage events use the same generation cancellation and shutdown owner.
`App` accepts normal counter receipts at the configured cadence and never treats
cached tabs as fresh evidence. It retains one meter per visited named squad,
pruned against visible/hidden tabs, and owns the runtime selected window. Leaving
a tab closes sampling continuity. Meter state is separate from pane/fold settings.

`board::rate` validates cumulative input/output/cache-subset, session/driver/epoch
and sequence/time order. Each reporting UUID owns a bounded ring of 5 s
receipt-time buckets, sized by the longest configured observation window (at most
24 h). Aggregate totals and trend slices derive from those same member rings,
without a second counter tracker. Missing, invalid, gap, decrease, new-session or
recovery evidence establishes a baseline without invented tokens. Removing a
roster UUID drops its history. Failed reads close continuity and mark gaps after
two sampling periods. Provider observedAt is order evidence, not a heartbeat.
Input plus output counts cached input once; mixed providers sum reported token
units, not costs/text volume. Retained usage belongs to the current observed
session model, explicitly best effort.

`config::TokenRate` layers team preset, global `[board.token_rate]` and per-squad
keys; Team alone defaults on. `[board] tok` and per-squad `board.tok` select
exactly three distinct ascending whole m/h windows from 1m through 24h, default
1m/5m/60m. Both layers are validated even when masked; the reader reports the
winning setting path for settings inspection. Built-in all/leads tabs omit the
named-squad meter. The bindable `token-window` action (`w` in both host presets)
cycles the summary through these windows outside text inputs. The meter shows
observed totals and always labels the window, never divides by elapsed time.
Incomplete uptime, gap evidence or unreported members prefix totals with `~`;
unreported identities contribute no tokens. A baseline alone is not measured
zero; usable intervals shorter than 10 s hide the summary. Member cells show
`—` until usable observations exist.

`App::project_usage` derives a board-only row document from the immutable public
status document, using the accepted meter receipts for model and three token
fields. Repeated section rows read one UUID history. Changed values invalidate
only the existing row grid/cell cache; retained views keep their owning values
while another squad loads. TEAM and crew preset TOML declare the default model and usage columns, including
all cell placement and priorities. `ColumnSource` recognizes board-only
`usage.w1`–`usage.w3`; App resolves them by window index. Config labels untitled
usage columns from `tok`, while explicit custom titles remain intact. Custom grids
opt in by declaring those sources. One-shot `ls` has no window history; JSON keeps
the descriptors without values and its schema remains unchanged, while text skips
columns whose source is board-only. Observation policy changes
start fresh history rather than inventing earlier coverage.
The default usage grid hides PR before the longest-to-shortest windows, then
model, using declared grid priorities without changing PR sizing. Model width
follows content up to 14 cells.

`board::meter` owns cubic counting digits (600 ms, 250 ms frame spacing and an
exact final frame), smooth retargeting and immediate window switches/reduced
motion. Its eight trend bars derive from member rings; slices round up to 5 s.
No evidence is blank; measured zero is ▁; nonzero bars use ▂ through █.
The meter renders one right-aligned number/unit/window/trend group with a
seven-cell maximum number region. It drops the trend and shortens the unit before
hiding, preserving its window label and lead/attention text. The normal cached
render and ratatui diff own output; no parallel paint path is introduced.
Window cycling is runtime state, never a config write.
A switch advances the worker's generation, cancelling superseded core reads in
the shared `tmt-invoke` bounded process owner. The refresh worker owns one never-reset stop flag per generation; preemption
and shutdown set that flag while the generation counter still fences events.
Cancellation kills and reaps the
child group without changing ordinary command deadlines or output bounds;
queued results carry their generation and cannot replace a newer view. Worker
shutdown cancels its core read, disconnects requests and joins after terminal
restoration. The input loop asks for a reload at the shown squad's `refresh`
interval (`Config::refresh`: per squad, then top-level `[board]`, then 5 s;
`None` is off), which each snapshot carries, so a squad that failed to load
retries at the default. That timer always runs. Between requests the refresh
thread checks `board::changes` every second: core's `changes.cursor` (the
public extension API method) and squad.toml's modification time and length.
When either moved since the stamp taken just before the last load, it reloads
that squad early, unless its `refresh` is off. A failed read is never a
change, and `API_INPUT_INVALID` (a core without the method) stops cursor reads
for the session, leaving the file check and the interval. A switch never clears the view: `App` keeps the view of each
visited squad, shows a cached one at once, and otherwise keeps the current
frame (marked stale, so row actions refuse) until the new squad's snapshot
swaps in whole. An uncached switch that lasts at least 100 ms shows a spinner in
the fixed summary header, ticking every 80 ms; cached switches show no loading
indicator. `ctrl-r` defaults to refresh in squad, leads and all views; squad/leads
bindings can rebind it through `[bind]`, while all keeps its own `[tabs.all.bind]`.
The effective `ctrl-r` refresh binding is dispatched before text inputs, preserving
search and composed messages. F5 has no default binding but remains configurable.
Tabs are the same width selected or not: selection is a style, never extra
characters. `board::view::tab_label` owns the styled tab and switcher label:
every name follows a fixed two-cell mark slot (`◆ ` waiting, `✗ ` blocked,
else two spaces), with the dominant count after the name. When both states
exist, waiting leads and a blocked `✗n` follows. Only these marks (and the
appended blocked count) use bold configured attention styles; tab names and
primary counts remain selected accent/bold or inactive muted. Selection covers
the entire tab with the existing background or reverse fallback. The switcher
keeps its own selected-row style. Rendered `Line::width` supplies tab scrolling,
hidden reservation, hit geometry and switcher fitting; overflow counters retain
their aggregate attention styling. `attention::Attention` is the one definition
of a squad's tab state, derived from its status document: members waiting on the user (`pending`
or `waitingOnYou`) and members `blocked`, each counted once. `ls` adds it as
`squad.attention`. The refresh computes it for the shown squad from that
document and publishes that view first. The same worker then computes every
other squad's attention from a roster-only document (one `rooms.roster` read
each, plus one `inbox` read shared by all, and no `ls`). Previous tab attention
stays visible until that generation's update arrives; a newer switch preempts
this lower-priority work. The cross-squad leads/all views still read the rosters
needed for their own rows before publication.

Squad's `view` command module owns the factory pane-arrangement catalog and
registers `view ls` (hidden `list` alias), `set` and `rm`; bare `view` lists.
The catalog owns all five arrangements in the existing split/fold grammar:
`team`, `focus`, `notes`, `detail` and `wide` supply arrangements and initial
fold settings only. The team workflow reads the same factory arrangement. `Config::board` resolves a hand-written
per-squad `board.layout` or `panes` first, then per-squad `board.view`, then
top-level `board.view`, then the workflow layout's own arrangement.
`Config::resolve_layout` remains the workflow owner, so a view changes no
states, rows, providers, reminders or meter policy. All view names are validated,
including masked settings. Explicit per-squad fold settings override factory
defaults through the same Board reader. Pane acquisition reads the resolved
Board, independently of the workflow layout. Narrow `wide` folds its middle
column below 180 cells and uses the existing solver's 40:30 redistribution
between rows and notes; it has no width-dependent arrangement resolver.
Named `Config::set_view` and `remove_view` edit only `view` in the chosen board
layer through `Config::write`. Scoped set refuses a hand-written layout with a
manual-removal hint; reset retains custom keys. All-boards choices remain masked
by custom or scoped arrangements. Reset drops only a table emptied by that reset
when its header has no comments; decorated and pre-existing empty tables remain.
No core settings writer is introduced.

The bindable `view` verb (`l`) opens `board::view_picker`, mirroring the theme
picker's scope, navigation and save/cancel lifecycle. Its opening Config is the
save baseline; refresh never replaces that draft. `App::effective_board` is the
single presentation accessor for preview geometry, fold defaults and focus,
while the existing per-tab FoldState retains session overrides. Esc restores
the opening Board and focus with the latest refreshed data, without writing;
successful save uses the normal changed-Board fold reconciliation. A custom
arrangement can preview in this-squad scope on a disposable Config copy, but
scoped save refuses to remove hand-written keys. In all-boards scope a custom
squad keeps its opening Board, shows the masking note and saves the global
view for other squad tabs. The reset entry removes only the chosen layer's view key.
The existing Reload request carries `preview_panes` only while the picker is
open, acquiring missing notes/replies through the same loader and cancellation
fence. Closing it preempts preview reads and returns to resolved-pane acquisition;
no second worker or arrangement resolver is introduced. The built-in leads/all
tabs retain their fixed home/leads composition throughout picker preview, save
and cancel; they offer all-boards scope, which affects real squad tabs only.

`board::app::overlay_event` is the shared modal input adapter for help, settings,
theme/view pickers and the switcher. It synchronizes their controller identities
with one caller-owned `FocusStack` and routes key/mouse events through
`tmt-tui::app::route` before base dispatch. Controllers retain save, rollback
and worker effects; close is consumed, unhandled modal events stay captured and
Ctrl-C returns Quit. Pane cursors and scrolls remain in their existing owners.

`board::picker_surface` retains caller-owned shared Picker state, admitted scenes
and current clipped frame maps for settings, theme/view previews and the tab switcher.
Theme/view controllers derive the selected choice from stable component identity;
they retain scope, opening Config, preview and persistence. Their selection-only
field keeps Tab's scope action. The switcher registers query and list fields:
printable navigation/close keys remain query text, Tab moves between those fields,
and query edits reset to the first match. Refresh follows the selected complete
tab key; resize/model replacement invalidates hits. Its semantic attention spans
use shared hit geometry and Squad's existing tab-color/selection policy. These
surfaces use shared modal chrome, wrapping, scrolling and inside footers.
Settings use grouped stable-key list rows for the reference and an admitted
docked prompt for edits. The existing Config controller retains raw edit text,
validation, disposable preview, stale-file refusal and persistence; edit cancellation
restores the retained list selection and scroll. Group headings are disabled rows;
read-only settings remain selectable so Enter can explain their restriction.

Squad's `settings` coordinator delegates to arrangement, rows, notebook/state,
meter, theme and tab/program area projections. Source-bearing Config reader
results own provenance; presentation does not inspect TOML or resolve values.
`config show` and the bindable inspection overlay (comma by default) share those
results. Provider argv, run bindings and state patterns are read-only;
inspection and edit validation never execute configured programs. Existing
`Config::bindings_for_tab`, `action::effective_bindings` and `tab_view::rows` keep
inspection and loaded tab/selected-section rules together. The overlay owns its
scroll position, blocks underlying input and retains its opening snapshot during
board refresh; close/reopen reads later configuration. Editable entries open a
local input prompt. Each valid value calls `Config::preview_setting`; the app
applies the disposable board, rows, notes mode, state colors, interval and tab
policy to the newest acquired data. The loader retains raw core squad order so
clearing tab order previews the same fallback as reload. Invalid input has no draft; Esc restores the
opening configuration and focus without discarding refreshed rows. Enter saves
only through `Config::set_setting`, then refreshes the values and sources. File
conflicts remain in the prompt and never replace concurrent edits. Provider/run
entries and nested split structures stay read-only. The ordinary loader acquires
preview notes/replies and metadata through its existing cancellation fence while
the overlay is open; closing returns to resolved-pane acquisition.
Aggregate tabs expose fixed grids and global appearance without squad providers.
CLI `config show` without scope inspects board defaults; `--squad` and `--tab`
are exclusive.

`board::help` projects navigation, effective bindings and meter explanations into
shared `tmt-tui::components::KeyHelp` sections. `Action::description` owns binding
wording for help and settings; settings retain their literal JSON value and source
separately from presentation prose. Meter input retains observed roster names,
including the lead and members omitted from displayed rows, for excluded labels.
The admitted help surface uses body placement and shared opaque modal chrome,
one all-section key column, wrapping and a fixed inside footer. Shared key-help
heading and spacing properties let help select bold text and one blank line
between sections without changing the theme palette. Its caller-owned
scroll state and the common App focus adapter route keys and mouse before board
actions; close is consumed, Ctrl-C quits, and base cursors and scrolls remain
with their existing owners. Refresh replaces help data and clamps the shared viewport without
performing reads or actions in paint.

`config::edit` owns the shared settings edit policy and disposable validated
Config draft. `sq config set KEY VALUE` accepts only layout preset, flat split
panes/direction/sizes, refresh, notes mode, hidden tracks, exact state colors and
global tabs order/hide, plus selected-squad reminder enable/threshold controls.
Reminder booleans use true/false and thresholds reuse `Config::reminders` whole
s/m/h validation (1m–24h). The board retains the newest `staleness::Snapshot`
evidence and reclassifies only known ages in memory through that owner; disabled
previews remove marks and unknown ages remain unknown. Preview does no observation,
cache publication or reminder claim. Confirmed edits reach the shared observer on
ordinary reload; its disabled path performs no reminder work. Edits never install
provider hooks or grant extension consent. Arrays use JSON syntax. Nested split-tree structural edits
refuse rather than flattening a custom or factory tree. Partial flat edits retain
the workflow preset and seed missing flat split keys from the resolved arrangement.
The draft uses the existing area validators before `Config::set_setting` calls
only the existing `Config::write` compare-and-set path. Changed files refuse;
comments, ordering and unrelated keys are retained, without backups or a core
writer. CLI edits change no roster or member metadata.

`rows::Rows` carries optional per-squad `board.hidden_columns` as named original
track positions alongside unchanged columns and lines. The reader rejects unknown,
duplicate, uncovered and all-hidden track masks. `markup::Grid` seeds its existing
shown set with this mask before priority hiding and Taffy sizing; cell spans count
surviving tracks in their original ranges. Zero surviving tracks omit a cell.
`ls` text uses the same range visibility; JSON keeps every field value, original
column/line metadata, and emits `hidden_columns` when nonempty. The empty default
adds no JSON member and changes no frozen board parity fixture.

Squad's `theme` command module registers `theme ls` (hidden `list` alias),
`set` and `rm`; bare `theme` lists. Lists and the board picker consume names and
descriptions from `tmt-cli-style::Base`, never a Squad palette. The effective
base source is `default`, `cli`, `board` or `squad`; token overrides resolve
independently. `config::Config` reads core's resolved appearance through public
`config show`, then applies `[board.theme]` and `[squad.<name>.theme]` in
`squad.toml` through `look::board_theme`. Invalid core appearance falls back to
the built-in base with a notice; invalid Squad layers are configuration errors.
All bases and token overrides are validated per layer, including masked values.
Named `Config::set_theme_base` and `remove_theme_base` change only `base` through
the existing writer, keeping token overrides and unrelated content. Squad never
writes `config.json`, and command/picker text states that CLI colors stay unchanged.

The bindable `theme` action (`T` in both host presets and the all tab) opens a
small overlay owned by `board::theme_picker`. The session reads its Config at
opening and keeps that baseline across refreshes. Preview applies the same
in-memory layer edit as CLI set, cached when selection or scope changes, with no
write; `App::look` supplies it to every
pane and tab. Tab changes board/squad scope; built-in tabs have only board scope,
and a masking squad base is named. Overlay input cannot operate underlying rows,
tabs or panes. Enter calls the named Config edit once; failed saves retain the
draft and notice without retry, while Esc drops preview and uses the latest
saved view. A refreshed config cannot replace the opening baseline and permit an
overwrite. No settings-view framework or core configuration writer is introduced.

Squad `config::duration` owns UTF-8-safe whole-unit suffix conversion for provider,
board refresh and reminder timing. Callers retain their accepted units, numeric
forms, ranges and key-specific error messages; refresh alone wraps `"off"`.

Optional `[squad.<name>.reminders]` config is parsed by
`Config::reminders`: enabled for the `team` layout, disabled for the
other layouts, 30 minutes, whole `s`/`m`/`h` values
from 1 minute through 24 hours. `staleness` owns observed raw task/state and
exact lead-notebook content age, separate from providers and column bindings.
`observe` is the one read sequence for a squad's status, used by `ls` and the
board's squad tabs alike: it acquires the nonblocking cache lock before the
roster read, reads the lead's notes through public `notes.read` only when the
observation can publish (or the board shows the notes pane, which then reuses
that one read), records, and hands the bounded room history to the request
overlay. Providers never run there: `ls --refresh-fields` refreshes between
observing and building the document, the board only hands members to its
fetcher thread. `Snapshot::apply` adds the same `staleness` object to every
occurrence of a member UUID and `squad.notesStaleness`; text labels derive from
those objects. The board draws a stale row in the `dim` token with its label at
the row's right edge, reserving that room only when no column would be hidden,
and adds the notes' label to the notes pane title in `waiting`; the label text
carries the meaning without color. The leads tab reads rosters without an
observer and shows no marks; the home model observes squads for blocked-member
ages. Content age is unrelated to `App::loading`, the previous squad's frame
while a switch loads.

The private observation cache under `$XDG_CACHE_HOME/tmt-squad/staleness`
is bounded to 512 KiB and 128 members per room, namespaced by the absolute
config/data-root path and room UUID, with member/lead UUID ownership. SHA-256
fingerprints (`sha2`) retain no notebook body. Nonblocking Unix advisory locking
(`nix::fcntl::Flock`) stays held from
before the read through atomic cache publication; competing readers report
unknown and never regress the cache. First observation starts the clock,
never backdated; unreadable notes, unavailable cache and clock rollback mean
unknown. Fingerprint/ownership/evidence changes publish immediately; otherwise
unchanged observations replace the cache only when its persisted `observedAtMs`
rollback watermark is at least 60 seconds old. Ages are computed on every
observation without writing. A rollback crossing that watermark still reports
unknown and restarts grace; a reversal entirely within an unwritten interval
can shorten reported ages by at most 60 seconds, while first-observed times
remain at or before the watermark. Cache loss/corruption restarts grace. Config edits do not
reset content age. Disabling stops observation; after re-enabling, surviving
fingerprint matches keep their first-observed time. These are observed content timestamps,
not core modification times or a history feed. Age determines staleness;
`activityAfterUpdate` separately records relevant observed PR link/state
changes, member finals, or authoritative idle transitions after a row update.
Only successful unexpired `github-pr` preset cache values, the public room
history and ordinary reads' runtime-verified `session.activity` supply evidence;
self-reported activity and offline presence never establish idle.

`reminder` consumes the generic consented `context_v1` callback at SessionStart
and prompt submission, never Stop. Its cache-only gate exits before core calls
or room locks for cold/off/fresh/claimed/non-lead cases. A warm candidate uses
public config and room commands to validate its root and room UUID, then
`observe::Mode::Reminder` reads only the roster, notes and bounded room history.
The current roster must independently establish the callback identity as the
sole lead. It runs no providers, presence probes or inbox overlays. `staleness`
publishes per-generation claims under the same lock before returning a summary;
`reminder` represents all claims by names/counts in one sanitized line.

Context calls share one monotonic deadline of at most 300 ms. Core's hook runner
isolates the extension's process group; context-only nested calls inherit it.
An invocation-scoped timer bounds input/files/publication/output too, signals
only its live process-owned group, and is canceled/joined on completion. This
path requires the extension to own its process group. Host timeout can cut it off
earlier and owns reaping. Ordinary Core calls retain their existing independent
groups and allowances. No resident worker or core Squad concept is introduced.
The extension guide owns the observed-age, claim-loss and cache-loss limits.

`tabs` owns squad keys, built-in keys (`@leads`, `@all`) and configured member
view keys (`@tab:<name>`), which cannot collide with squad names. `[tabs] order`,
`pin` and `hide` refer to user views as `tab:<name>`; unplaced user views follow
the defaults in definition order. Config reading validates every `tabs.<name>`
filter, sort, section and binding, including hidden views. Built-in names remain
reserved. `tab_view` owns cross-squad acquisition and aggregate documents for
both the board worker and `ls --tab <name>`. A single roster read per squad feeds
source documents and the public member projection, retaining numeric sort values
and source state ranks beside JSON. Configured field providers contribute their
existing cache; aggregate reads never run them. User selection applies before the shared
`status::sections` pipeline; section matches may repeat a row, while unmatched
rows follow untitled. User views use `Rows::leads` with a MEMBER caption, without per-tab row overrides.
Both callers receive the same projected rows, attention and row-grid metadata;
`status::text` renders that document. Unreadable squads are omitted with located
`failures`; a failed inbox read retains available roster fields. Both cases set
`partial`, with a board summary indicator and text warnings, and clear on the
next successful read. Member views join one global `ls` read for presence. Its rows carry their squad, so talk goes to that
squad's room and a jump is the ordinary `tmt focus`. The public all document
retains one row per squad; its board-only home composition also includes
attention members. `jump lead` (`L` in the tmux preset) resolves a
lead name in `App::lead`: the document's `squad.lead` on a squad tab, the
selected row on the leads tab, and the selected entry's lead on home; it then
takes the ordinary jump request, so the popup closes and `back` returns.

`board::home` retains a board-only summary, shared-filter attention sections and
squad tile model as typed `View.home: Option<home::Home>`; other views
carry no home data.
It reuses `tab_view` acquisition and the user-tab section pipeline. Its optional
observed ages come from the existing staleness observer: the home tab starts
one for every squad before its roster read and records afterward, writing its
observation cache under the held per-squad lock when enabled and available.
It respects the reminders policy without extra core commands. Request ages
use shared-inbox timestamps; pending-only rows have no age. The source aggregate
document and `ls --tab all` JSON/text remain unchanged. The home painter uses
one summary band and body, bypassing ordinary pane composition for the shown
immutable home view. `home::tiles` supplies pure lines and local item/line/x/width
regions from one admitted Taffy grid: three columns at 150 cells, two at 100,
and compact rows below 100 or with at least ten visible squads (two compact
columns at 150). The controller's filtered reading order stays row-major.
Each tile shows its squad, lead/model/window totals/share and exclusive non-lead
urgency marks/member count; compact rows retain the last two lead windows.
Whole-roster summary and attention counts retain their existing semantics.
Unknown member states contribute to the member count without inventing a mark.
Runtime `App::home_usage` owns observations, model attribution and the configured
longest-window share; tiles only format them. Unavailable and measured zero
remain distinct; partial readings and shares carry `~`. Uniform window labels
appear once in ③; mixed configurations use a “windows vary” legend and label
each tile's totals and displayed share with its actual window.

Home keeps one `App.selected` cursor, reconciled by section/squad/member identity
across refresh and search. Attention precedes squads; future replies and cron
targets insert between them. Home translates tile regions into global ordinals,
complete selected-range reveal and viewport-clipped continuation hits through
one `Scrolls` pass. Selection covers every padded tile row; gaps and headings
have no hit target. Ordinary pane geometry and scalar rows retain their owners.
Enter jumps to a member or opens a squad; Tab traverses attention/squads, and `a`
opens the real request picker or an annotation to the selected squad’s lead.
The composer retains and revalidates sender, target, lead and open request before
public `tmt answer` or annotation dispatch. Questions stay inside the picker;
tiles show no member names, task/PR fields or private question text. Replies feed
and cron acquisition/lifecycle remain separate owners. A future cron summary
supplies an explicit stable clock-key target, not a squad target.

HOME usage templates are separate typed `View.home_rate` data. While HOME is
open and at least one squad enables token sampling, the existing meter worker
reads one global public `tmt ls --json` per sampling cycle, indexes identities
once, and joins each enabled squad's roster into its retained meter. Other tabs
never schedule that HOME read; switching cancels the existing worker generation
and closes observation continuity. No additional worker, core API or provider-file access is added.
`App::home_usage` exposes the current lead model, configured windows, raw lead
and squad readings, and lead share of the configured longest window (normally
60m). Missing observations remain absent, measured zero remains zero, partial
coverage propagates to share, and a zero denominator has no share. Tile and
header consumers format this projection without sampling or recomputing totals.

Moving a tab (Shift+←/→, or a drag on the tab
line) saves `[tabs] order` through `Config::write`, the same compare-and-set,
format-preserving replacement that records `me`. A tab line that doesn't
fit scrolls: `tab_window` keeps the current tab in view, starting as near the
last frame's first tab as it can. It counts the hidden tabs at each end, and
only the drawn tabs can be clicked. Pinned tabs (`[tabs] pin`) come first from
`tabs::arrange` in `pin`'s order and are drawn before the scrolled window. A
move never moves or passes a pin, since the saved `order` could not reorder
them. The switcher (`s`, unless the user bound
it) filters the tab line's tabs and the hidden ones with `tabs::matching`: a
prefix match first, then a substring, then the letters in order. A shown
squad that isn't on the tab line (hidden) is drawn first, selected, with no
`TabHit`, so it can't be moved. `board`
runs only when `tmt_cli_style::Interaction::view()` is `Interactive` (decided
once in `main`); otherwise it is `ls`. `tmt squad` with no command is `board`. Consent for hotkeys and playbooks is
likewise a `Consent` decided in `main` from `--yes` and `prompt()`. `[squad.<name>.board]` selects
split or tabs panes (rows, notes, detail, replies) over a per-layout preset,
validated before raw mode. Squads with no layout key use team unless they set the simple board form, which keeps crew. Explicit
`crew`, `pr-queue` and `minimal` retain their presets. `Config::resolve_layout`
owns the shared decision for the layout and board readers. The `team` preset uses the same `Layout`/`Board::preset` and ordinary config readers: a
60/40 top-bottom split, rows beside detail/replies at 62/38 in the top, detail
above replies at 50/50, and full-width lead notes underneath. Its crew states,
pending-first ordering, member/state/task/pr/model grid with a pending line,
60-second `github-pr` field and 30-minute observed-age default are all
configurable; existing presets keep their defaults. A user `rows` or legacy
`columns` table replaces the grid, `fields.<name>` replaces that provider's
whole table, additional provider names retain `pr`, and reminder keys override
individually. The nested board requires a full `layout` or `panes` override;
partial `direction`/`sizes` overrides are rejected. Model reads the existing
session projection, and providers remain on the existing fetcher path.
`split` owns validated row/column trees up to three levels and reading/focus order,
not geometry. `board::composition` admits an embedded version-1 XML scaffold before
raw mode, then instantiates its named prototypes from the validated Board/Split
and runtime folds. Folded panes reserve one stacked title line or compact side-by-side
title width; fully folded groups propagate that footprint. Expanded siblings share
the remainder through typed percent/grow styles and one Taffy flex computation. Named
rectangles dispatch to the existing rich pane painters; notes/replies retain
Markdown, wrapping and interaction owners. Tabs reserve a shrinkable one-line
bar above a focused pane with a one-line minimum. No runtime file loader or
alternate composition solver exists. The immutable-view cache keys viewport,
effective Board, folds and tabs focus; row selection does not rebuild geometry.
Nested percentages use raw fractional parents, then cumulative edge rounding:
a 60% Team parent split in half at body height 21 gives Detail/Replies 6/7,
rather than 7/6 from halving an already rounded parent. Rows/Notes and widths
remain unchanged. The tree's reading order remains focus order, skipping folds.
The configured Board/Split never changes during
a toggle. `Config::board` strictly validates the initial `collapsed` pane list
for split mode, plus `fold_below = { width, panes }` with width 1–1000 and
panes present in the resolved layout. Team sets width 100 for detail and replies.
`App` resolves the effective fold set from board body width and immutable defaults;
per-pane user overrides win at either width. The terminal draw owner supplies the
full-width body measurement before rendering; the view only passes the resulting
set to `board::composition`. `App` owns bounded per-tab session overrides, preserving them
through unchanged refreshes and cached switches, resetting them on changed board
configuration, and dropping removed tabs. Restarting uses config again. The
existing `action` parser/dispatcher owns `toggle <pane>...`, accepting one or
more unique literal pane names. It acts on the named panes present in the board,
silently doing nothing when none are present. If any is expanded it folds all;
otherwise it expands all, setting each pane's session override. Both host presets
bind `d` to `toggle detail replies` when the resolved board contains both panes,
otherwise the one available pane; neither yields no default `d` action or hint.
Configured and section bindings still override the preset. Footer and help name
the effective panes and current state (`detail+replies ▾` when any is expanded,
`detail+replies ▸` when all are folded); the footer drops the whole hint if it
does not fit. View presets supply only immutable Board defaults.
Each render records the visible title hit regions; a left press toggles before
row dispatch, without selecting a row or contributing to row double-click history. Folded
bodies produce no row/scroll hits. Collapsing focus returns to visible rows, otherwise the next
expanded pane; with every pane folded there is no body focus. Expanding from that state
focuses the expanded pane. The notes action expands notes before focusing it.
Single expanded panes keep their existing borderless rendering; their folded
title is clickable to expand. The detail pane appends full projected `row.fields` values for board columns not already represented by its header, task, activity or links, in column order; it escapes and wraps them without grid fitting, source lookups or provider calls. The notes pane shows the squad lead's own saved-identity notebook, read-only;
there is no separate squad notebook. `observe` selects the member with
`Member::is_lead` and reads its UUID through public `tmt api notes.read`
(bounded, never creating a file), the same notebook that
`tmt notes path --identity <lead>` discovers. `board::refresh::lead_notes`
maps a missing saved notebook to `(no notes yet)`; a temporary lead's
`NOTEBOOK_SAVED_IDENTITY_REQUIRED` error is displayed as failure text.
`board::notes` removes
every escape sequence, control character and hidden bidi/format character before
display, since notes are agent-written. `board::markdown` is a thin
pulldown-cmark view over that sanitized text: it styles headings, lists,
emphasis, inline code and links, and shows every other construct as its source.
`links` classifies explicit Markdown destinations as web, GitHub issue/PR, local
path, built-in `tmt:` or user-configured scheme. The same Markdown pass retains
destination occurrences and wrapped display-cell ranges; unsupported constructs
remain source text. Every admitted label uses the existing Link role and
underline; kind and full destination appear in the footer before activation.
Tab/Shift-Tab select links in focused notes (Tab keeps pane traversal when none);
explicit configured bindings win. A first click selects/previews, a click on the
selected occurrence activates, and Escape clears selection. Plain mode is inert.
Only `tmt:jump/back/talk/answer/open/copy/annotate` are admitted. Except `back`,
`/<member-name-or-id>` must resolve to a current row or the separately projected
lead. Optional `?text=` is bounded percent-decoded composer text for talk/answer/annotate only. Those verbs reuse
existing prompts/request pickers; submission revalidates sender, squad, member,
lead or open request after refresh. Answer uses the existing public core answer
adapter. Undefined/invalid schemes are plain and cannot dispatch.
Only user-file `[links] scheme = "run program {path}"` grants a custom program:
validated literal executable and one argv element per template, no shell or
option injection. Reload replaces that authority. Existing detached spawn/reaper
owns programs; absolute local paths reveal after canonicalization: macOS uses
`open -R`, configured/Linux openers receive only the containing directory.
Relative paths are inert; opening files requires a user-defined custom scheme.
Neither parsing nor paint opens files, fetches URLs or invokes commands.
Its mapped rendering retains each painted line's notebook source line without a
second Markdown parser. `App` keeps one notes cursor per visible/hidden squad,
anchored to the complete sanitized source line (nearest match for duplicates,
clamped position after deletion), with a continuation offset for wrapped lines.
Cursor movement and click placement reveal the painted line through `Scrolls`;
wheel scrolling suspends following until cursor movement. Every painted
continuation of the selected source line uses the existing selection background/reverse fallback across the pane
width. Only visible lines are decorated; a fixed two-cell gutter holds the sent
marker or blanks before wrapping, keeping text aligned without clipping.
Notes annotations reuse the ordinary composer and annotation sender, addressed
to the current lead and tagged with the source line number and a bounded quoted
excerpt. Opening, canceling or submitting an empty composer sends nothing.
The `[<squad> · notes L<one-based line> <JSON quote>] ` tag is the contract
between the annotation sender and request projection; display quotes are separate.
`requests::apply` projects only the user's open notes annotations to the current
lead as optional `squad.noteAnnotations` (`requestId`, zero-based `line`, `quote`),
using the existing bounded room history. The painter marks the nearest matching
quoted source line with `✎`; answered requests disappear on the next refresh.
No additional core read, notebook mutation or acknowledgement is introduced.
The detail pane appends the selected member's saved-identity notebook after its
fields. The session requests only a visible, expanded selected detail, accounting
for effective Board previews, tab focus and the last painted viewport; temporary
identities show `(temporary identity: no notebook)` without a read. Leads/home
never show member detail notebooks. `board::refresh::Deferred::Notebook` runs
public `notes.read`
with the same bounded cancellable reader and 1 MiB API notebook limit as lead
notes, never creating a file. Full reloads take priority; queued selection jobs
collapse to the latest. Events retain the existing generation cancellation and a
session selection/refresh revision, so obsolete results cannot update the cache.
Each accepted snapshot revalidates the visible selection; hidden detail does not
read. `App` owns the last eight identities' sanitized notebooks, preserving the
rendered body for unchanged content and invalidating it on width, look or render
mode changes. Both notebook panes share safe Markdown/plain rendering and the
missing placeholder; failures replace the selected cache entry. Paint and input
perform no core reads.
State `sort` overrides reorder the vocabulary for both `ls` and the board.
`effects` holds the row actions behind the plain `jump`, `open` and `copy`
commands and the board. `template` fills `{field}` placeholders into one value and refuses
empty values. Programs run as argv, never through a shell: the configured
top-level `opener` and `clipboard` arrays, or the system opener. An opener
starts in its own process group with null stdio, and a thread reaps it. Copy
prefers the configured program, then, inside tmux, `tmux -S <invoker socket>
load-buffer -w -`: `-V` must report 3.2 or later, and `show -sv set-clipboard`
decides whether the text reached the clipboard or only a buffer. Otherwise copy
writes OSC 52 to `/dev/tty`. `jump` checks membership and then calls `tmt
focus`; squad has no focus logic of its own. `jump --lead` finds the squad
from `--squad`, else the caller's identity (`tmt whoami`) in exactly one
squad roster, else the only squad, and jumps to that roster's lead the same
way; no lead is a refusal before any focus.
`action` parses `[bind]` and `[squad.<name>.section.bind]` once per load into
events and actions whose arguments are templates; bad events, actions or field
syntax are configuration errors. The board resolves the selected row's section
binding, then `[bind]`, then the host preset (tmux: Enter and double-click jump;
a plain terminal: they open the row's action menu) into a fully filled request
before anything runs; a missing value is a notice, not a partial action. Mouse
capture is part of the terminal state the `Screen` guard restores; each draw
records which screen lines show which row, so a click selects exactly the row
drawn there. `board::scroll` is the one scroll owner: every pane hands its
lines to `Scrolls::show`, which keeps a position per pane, clamps it to the
content, reserves the last line for an `↑ n  ↓ m` indicator when the pane
overflows, and records where the pane was drawn so the wheel scrolls the pane
under the pointer and a left click focuses it. Panes keep no scroll state of their own; the rows pane only
asks it to reveal the selected record's visual-line range while followed (or
its first line when taller than the viewport). The draw records record starts
and hit targets for every continuation; paging moves by viewport lines for all rows, including existing notes/configured row lines, with record paging when no positions were drawn. `run` fills one argv element per template (refusing a value that would start an argument with `-`) and starts it like the
opener (no shell, null stdio, its own process group, a reaper thread). `back` keeps a
disposable stack per tmux server and client (`$XDG_CACHE_HOME/tmt-squad/back`,
0700, atomic replacement, 32 entries, corrupt or foreign files read as empty).
Every jump pushes the pane the client left, under the client `tmt focus`
reports; `back` asks core for the invoker's client with `tmt focus --client`,
pops its entry and focuses it, so squad still never talks to tmux about clients.
`hotkeys` generates `squad.tmux.conf` (bindings noted `tmt squad popup|pane|back|lead`;
the optional lead key's `run-shell` job has `TMUX` but no `TMUX_PANE`, so it
passes `TMUX_PANE=#{pane_id}` for core to name the caller)
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
`send` sends through public commands only: detached `talk --identity <sender>
--room squad-<name>` with operands after `--`, annotations
as a talk tagged `[<squad> · <row>]`, and answers as one `tmt answer <member>
--request <id>` (core selects and proves the request; no receipt passes through
Squad); nothing acknowledges. Squad has no talk, reply or replies commands of its
own: those words refuse before parsing with the core command that replaces them. The sender (`me::resolve_sender`) is
an explicit `--identity`, otherwise the identity core attributes the call to
(`tmt whoami`), otherwise the recorded user; with none, `SQUAD_SENDER_UNKNOWN`
names both ways to set one. `whoami`'s `PANE_NOT_FOUND` and an unbound pane mean
"no caller"; any other core error, such as `CALLER_IDENTITY_AMBIGUOUS` on a shared
runtime host, fails the command rather than falling back to the user. "You" for
`waitingOnYou`, `ls` and the board (`me::you`) is the recorded user,
otherwise the saved identity bound to the calling pane (the board reads it once
per worker); when neither exists, `ls` and the board footer show one hint
line. The board also sends as "you", because a popup's pane is not its operator.
`requests` derives each
row's `annotation` (the sender's newest open tagged request) per load from
`requests.list` for the squad room, at most four pages of 50, and `waitingOnYou`
(what waits on "you", oldest first) from `tmt inbox --json`, at most 200; it
marks the document `olderRequestsNotShown` when either is cut off.
The same room window yields the replies list (finals to the user's requests,
newest first); bodies come from `requests.show` for the newest eight only, and
the refresh worker caches them by request ID because a submitted final never
changes. Bodies are agent-written and use the notes sanitizer and Markdown
renderer, with full wrapped content and a two-cell indent. Prompts wrap with a
hanging indent; recipient/age headers remain single-line. The immutable view's
`Derived` caches rendered bodies by request ID, effective pane width and look;
headers and prompts are assembled each frame so ages stay current without
reparsing Markdown. View replacement discards the cache. Replies keep the shared
`Scrolls` owner, including overflow indicators, wheel and keyboard paging.
Membership commands are sequences of idempotent core commands, not one
transaction; each reports what it applied, and a re-run converges. `add` reports
`added: false` for an existing member, including a repeated name in one call,
and preserves its state and task; only a missing state receives the configured
initial value. `lead --none` reports a null lead and the former leads in
`replaced`, retaining their membership. `squad.toml`,
beside the global config that `tmt config show` reports, is the user's file.
`Config::write` owns format-preserving replacement for `me`/`me_id`, tab order,
board views and theme bases. It checks the original bytes, edits a cloned document,
skips unchanged bytes and assigns the new document only after successful
publication. A changed file is refused, not overwritten. Its byte check and
atomic replacement are not a locking transaction; backups are not created.
`me` and `me_id` (the UUID `me` named) are written together. Nothing asks for `me`: `init` only creates the room (`--me`, for
scripts, is checked before any effect), and `tmt squad me [<name>|--clear]`
shows, records or removes it. The UUID decides, as it
does for binding markers: while `me_id` names an active identity, that identity is
the user and `me::resolve` rewrites `me` to its current name. Only when `me_id` is
missing or no longer active does the name decide, and its UUID is recorded. An
edited `me` that names a different identity is reported with a warning, never
followed, so a reused name cannot make squad act as someone else; `tmt squad me`
changes the user. A failed write never fails the command, and the board's
refresh (`me::current`) neither writes nor prints. With hooks enabled, `tmt-squad __tmt-hooks 1 observe` applies an `identity.renamed`
observation for `me_id` at once; the hooks are optional, and the same repair
happens on the next command that reads `me`. The `tmt-squad` lead skill source lives under
`extensions/tmt-squad/skills/` and is embedded only in the squad executable,
never in the core skill bundle. Optional playbooks (`tmt squad playbook
ls|show|install|rm`, first `tmux-squad`) live beside it in
`extensions/tmt-squad/playbooks/`, deliberately not under `skills/`: the release
archive ships and the extension installer offers every skill under `skills/`,
while a playbook is installed only on request, and a test pins that no playbook is
in that tree. `playbook.rs` holds the one catalog of embedded sources and registers the
subtree through `tmt-cli-style` (summary and examples per command, `--json` from
squad's global option); it is Squad's first dependency on that crate. `show`
prints the exact bytes; `install` asks (the same `consent` helper as `hotkeys`),
then calls `skills.install` as owner `squad`, and `rm` calls `skills.remove`
with the playbook's name, so the lead skill and `tmt extension rm squad`
are unaffected. Squad never writes a provider directory and never executes a
playbook. Squad's dependencies must not change the CLI
product: the proof is package-scoped (`-p tmt-cli` alone), because combined
workspace builds may unify shared-dependency features across packages. Squad
is versioned independently and released as its own product (`tmt-squad-v<version>`
tags); its archive also carries `skills/tmt-squad/`, the same source, as the
release's skills tree.

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
