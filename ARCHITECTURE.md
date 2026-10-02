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

This section owns the repository layout map; the infra squad reviews layout changes.
The machine-readable top-level allowlist is
[`.github/repository-layout.json`](.github/repository-layout.json). It lists permanent
entries and current exceptions with their removal issues. Component ownership comes
from [`.github/components.json`](.github/components.json), through `ci-scope.ownerOf`;
layout permission does not change component ownership, CI selection or release policy.

| Home                      | Responsibility                                                                                                                         |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Repository root           | Short entry points, contributor guidance, license and required repository/tool configuration; no product source or generated evidence. |
| `.agents/`                | Repository contributor procedures.                                                                                                     |
| `.github/`                | Component ownership, layout allowlist, workflows, shared Actions and isolated release tooling.                                         |
| `rust/`                   | Native CLI, core, adapters and shared Rust leaves, and the release archive note; extension crates remain under their extension.        |
| `typescript/`             | Private developer tooling, tests and shared fixture support; no product-runtime fallback.                                              |
| `extensions/<extension>/` | Feature-owned runtimes, contracts, skills, documentation and assets.                                                                   |
| `contracts/`              | Core public contracts and their normative fixtures.                                                                                    |
| `scripts/`                | Shared root shell/build/development helpers.                                                                                           |
| `skills/`                 | Canonical bundled user-agent guidance.                                                                                                 |
| `site/`                   | User handbook and its build; handbook text remains owned by tmt-lead.                                                                  |
| `design/`                 | Shared design tokens and CLI style guidance.                                                                                           |

Homes of moved guidance:

Core public process and request/response contracts live in `contracts/extension-api.md`
and `contracts/request-response-v1.md`; the Remote channel contract lives in
`contracts/remote-channel-v1.md`. CLI style guidance lives in `design/cli-style.md`.
Release-verification procedures belong to
[DEVELOPMENT's release section](DEVELOPMENT.md#native-release-verification).

- The handbook owns user guidance. Office documentation and art helpers live in
  `extensions/tmt-office/docs/` and `extensions/tmt-office/scripts/art/`.
- `rust/archive/NATIVE-INSTALL.md` is the short offline note that every release
  archive carries under the entry name `NATIVE-INSTALL.md`; the archive inventory
  is part of the installer contract, so the name stays. Runtime performance probes
  are in [DEVELOPMENT](DEVELOPMENT.md#optional-performance-probes). No top-level
  exception remains.

New homes or exceptions require an infra-reviewed proposal with a component owner
and bounded responsibility. Update this map and the JSON allowlist together;
remove an exception when its last tracked entry moves or is deleted. The tooling
layout test checks every tracked file's component owner and that tracked top-level
entries are a subset of the allowlist, and rejects temporary exceptions with no
tracked entry; ignored local outputs are outside that map.
For add/move review and rename hygiene, use the
[layout procedure](.agents/skills/tmt-layout/SKILL.md).

Shared visual tokens have one owner: `design/tokens/tokens.json`, maintained by
the design lead. `design/tokens/tokens-plugin.ts` projects them into CSS for Vite
consumers without a runtime or package dependency. The handbook imports that
source; Rust CLI theme tests check its built-in palette against the same file.

## TypeScript workspace boundary

The `typescript` pnpm workspace has one lockfile, retained Node tooling and tests,
the `@tmt/office` SPA, the `@tmt/office-service` trusted pairing service,
the private `@tmt/browser-addon` demo shell and `@tmt/remote-client` byte codecs,
and the private `@tmt/colab-client` WebCrypto primitive library.
The two Office packages live under `extensions/tmt-office/typescript` as
parent-relative members of that same workspace and lockfile. They resolve only
their declared dependencies, never root-hoisted tooling packages; Office browser
specs reach the tooling-owned SQLite oracle through `typescript/test/support`.
Rust, root shell launchers, shared contracts and canonical skills remain outside
that boundary. `contracts/` holds core contracts only; Office contracts, vectors
and the Office skill sources live under `extensions/tmt-office/`; the proposed
colab contract lives under `extensions/tmt-colab/contracts/` (see the
[colab boundary](#colab-extension-proposal)). The Office SPA build must finish before building the embedded
native companion, followed by installed-browser acceptance; ordinary CLI builds
remain independent. [DEVELOPMENT.md](DEVELOPMENT.md#office-browser-verification)
owns the direct commands and their order. Read
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

`.github/components.json` is the one component map: who owns the CLI, Office and Squad
paths, and the ordered rules that say which CI consumers a path selects and why.
`typescript/scripts/ci-scope.mjs` reads it and owns conservative affected-area
selection and final gate validation. Native source/skill changes, Office's Rust crates,
core test suites and shared or unknown paths retain full native verification; Office-only
app/docs paths avoid native matrices, and unread prose selects nothing beyond `Code quality`.
Frozen Office verification follows component ownership, Office docs and Office-specific
workflow/emulator machinery only. Shared dependencies and unknown paths do not select it.
Weekly/manual runs cover Office; merge groups and main pushes never run Office web,
local-service or companion-dependent native jobs. Workspace Rust clippy/test/build still
include the Office members. The explicit remote-Rust rule retains full workspace coverage
independently of private release ownership. Full Rust checks reject
empty remote test discovery before executing all workspace tests, including the
remote lifecycle tests and core architecture guards. Diffs include deletions and both
sides of renames. The selector writes
a per-path evidence table (owner, rule, selection, map digest) to the run summary. When
every path that selects native work is owned by Squad, the native scope is `squad`: the
same job names run Squad's Cargo checks and architecture guard, its native tests and its
E2E file, while the CLI runtime builds, packed installs and tooling unit tests are skipped
because the CLI is unchanged (Squad cannot affect core: the architecture guard rejects any
dependency in either direction). `Native package matrix` expects exactly that set of results
for the scope; anything shared, CLI-owned or unrecognized runs the full set. `Docker E2E`, the
required check, is a gate over two shard jobs that split the E2E scenario files by the committed
weights in `typescript/test/e2e/shard-weights.json` (the first shard also runs the Rust adapter
tests): it requires both shards when native work is selected, the first alone for a scoped
component and neither when nothing native is selected, so a skipped, cancelled or missing
selected shard fails it, and a guard proves every scenario file is in exactly one shard. Existing required check names
remain; `Code quality` gates selected Office verification and `Native package
matrix` gates all selected native jobs. Selected skipped, cancelled or failed
jobs cannot satisfy either gate. No passing zero-test configuration is allowed.
`Native Rust contracts` aggregates independent fmt/Clippy, workspace test/build,
Office local-service and native process workers, plus an MSRV worker that reads
`rust/Cargo.toml` and checks every workspace target. Full scope requires all except Office;
the Office worker also requires explicit `native_office=true`. Squad skips Office, and
none skips the aggregate. Missing, failed,
cancelled or unexpectedly skipped workers fail closed. The native process worker
consumes the Office fixture producer's SHA-256-checked local-service executable only
when Office is selected. Otherwise it excludes Office-owned native suites and requires
no companion artifact, while keeping nonempty core discovery and independent fixtures. The Office check worker consumes the same embedded SPA; other
fixtures remain independently built in the native worker.

Rust dependency caches (`Swatinem/rust-cache`, pinned by commit SHA) have one
main-only writer per key: workspace tests write the shared dev dependency cache,
MSRV writes its toolchain-specific cache, and each native runtime target writes
its own cache. Every writer uses the single seed-event classification (`verify=false`)
and the main ref; PR and merge-group runs only restore. Other workers restore
the shared cache without saving. Dev debug
information and incremental compilation are disabled across CI; release profiles
retain their manifest policy. Main cache seeding runs on selected Cargo/workflow
changes, weekly and manually; feature-branch dispatches only restore. A seeding
run has no diff and takes full native scope. Its Rust aggregate still checks the
workers; the outer verification gates remain skipped.

`ci.yml` owns all four required checks: Code quality, Unit tests, Docker E2E and
Native package matrix. Both pull requests and `merge_group` candidates run those
checks. Merge groups select paths from the common ancestor of fetched
`origin/main` and `merge_group.head_sha` through that queue head. Under HEADGREEN,
the event's `base_sha` can be a preceding queued commit; using it would omit earlier
pending changes and let a prose-only tip skip their checks. PRs retain merge-base (`...`)
selection. Both use the same component-map rules, scopes and E2E partitions;
empty or unreadable merge-group diffs fall back to full native verification with
Office still unselected. The changes job fetches full history, and missing commit objects
cannot yield a successful empty selection.

The changes job also owns the `macos` classification: false only for
`merge_group`. Separate macOS raw-runtime build and packed-install jobs consume
that output, sharing their verification steps with Linux through YAML anchors.
The native aggregate requires a valid classification and exact `skipped` macOS
results when false; full-scope PRs require success. Missing, failed or unexpected
results never pass. All selected Linux rows remain required. Release builds and
archive verification retain macOS before publication. Advisory Office browser
checks remain separate; the repository owner controls merge-queue rulesets.

The advisory Office browser workflow has a separate ownership-based PR flag,
`office_browser`: Office-owned component paths and the browser
verification machinery select its emulator/image work. The selector owns the
Office-specific workflow/emulator exception so that machinery
exercises itself. Shared dependency/selector/generic fixture changes, ordinary
core product dependencies and unknown paths do not select
browser PR work while Office is parked. Scheduled/manual runs cover all twelve
partitions, including the emulator; the existing native/local PR pauses remain.
Required native CI keeps conservative selection; the Rust gate validates the explicit
Office selection and exact worker results in both states.

The same map feeds release versioning. `typescript/scripts/release-please-config.mjs`
generates `release-please-config.json` from the map (one release-please package per
component root, minus its excludes), the Cargo workspace (which crates declare their own
version, which path dependencies a component links, which crates have a `Cargo.lock`
entry, which files are tracked) and `native-release-policy.mjs`, the one owner of tags and publication flags. Its
`readWorkspace()` exposes Cargo-resolved crate versions through bounded, offline metadata. Native
CLI version expectations and Office installation/hook fixtures select their crate from that
reader once per suite instead of parsing TOML separately. A `Cargo.lock` line is updated by
whichever component declares that crate's version: a crate
that inherits the workspace version is declared by the owner of `rust/Cargo.toml`, even when
a private `release: false` component owns the crate, because the next locked build fails
when that release leaves its entry behind. Office is parked this way: it owns its files and
CI scope but has no release-please package, manifest entry or release run, and its binary
opts out of cargo-dist with `dist = false`. The native-release entry delegates to
`native-release-policy.mjs require-released`, which checks the same component policy
before preparation or draft planning, so a parked product cannot enter the bundle
pipeline through manual preparation. Native tests compare the CLI's installable
extensions with the released extension components in the map.
release-please attributes a commit to a package by the files it touches under the package
path and can only drop paths, so the CLI's `exclude-paths` lists everything under each
extension root except the crates the CLI links (today the Office model, command and service
crates), and a change to those crates counts toward the CLI release as well as Office's. The
reverse direction has no release-please config option. The map's private TUI leaf declares
`releaseConsumers: ["squad"]`. `release-please-run.mjs` wraps the pinned public commit iterator
and adds a consumer-root marker to each matching commit's in-memory file list before the normal
split, excludes and per-product release cutoff. Original files and ordering are preserved;
no source file, private-leaf version or release manifest entry is created. TUI-only fixes therefore
propose Squad, while the CLI remains excluded. Other shared leaves retain package-root attribution;
expanding consumption requires a separate ownership review.
`.release-please-manifest.json` holds the last published versions and belongs to
release-please after its first release pull request. The CLI is pinned with a lockfile in
`.github/release-please/`, outside the `typescript` workspace. Only the release job and CI jobs running
release-config tests install it; tests load that same isolated pin to verify the wrapper's API shape
and real Manifest attribution, without adding release tooling to other workspace installs.
The config retains `always-update` for conflict recovery: each component edits the shared
manifest, whose adjacent lines can conflict. `release-please-run.mjs` wraps the pinned
GitHub update boundary to preserve an open PR's head when its title, complete inline
notes, generated file bytes and modes are unchanged and GitHub does not report a conflict.
File comparisons use the observed immutable head SHA. Changed content, missing files
and confirmed conflicts use the original updater; unknown mergeability preserves an
otherwise unchanged head and returns normally so draft reconciliation continues. Acquisition
errors fail visibly without treating uncertainty as equality. Overflow notes retain the
original update/overflow behavior. This avoids CI restarts for unchanged release content;
new release content still needs a new verified head.
A tooling test fails when the committed config is not what the generator writes, when a
workspace crate's lock entry or declared version is managed zero or several times, or when
a tag disagrees with the policy or a package could leave the alpha line (release-please's
`prerelease` option also keeps the version line, so `false` would graduate 5.0.0-alpha.8 to
5.0.0; the flags a published release carries come from the policy when the draft is
published). Nothing runs the pinned CLI until the release workflow adopts it.

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

The Rust crates have deliberately narrow responsibilities:

| Layer             | Owner                           | Responsibility                                                                                                                                                                                                  |
| ----------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure domain       | `rust/crates/tmt-core/src/`     | Identity, names, bindings, profiles, settings, retention, request state and native-install version policy. No filesystem, process, SQLite, tmux, network or CLI framework.                                      |
| Concrete adapters | `rust/crates/tmt-adapters/src/` | Config files, SQLite, bounded files and processes, signals, tmux evidence/transport, response input, HTTP acquisition, native release publication and managed skill files.                                      |
| Application/CLI   | `rust/crates/tmt-cli/src/`      | Core grammar, typed invocations, preflight and use-case composition, completion and the executable entry point. It chooses adapters; it does not duplicate their storage, file, installation or process policy. |

`rust/crates/tmt-command-output` owns shared command output/error values and
formatting. It renders human text through `rust/crates/tmt-cli-style`, the one
implementation of the [CLI style](design/cli-style.md) (palette, themes over the
design tokens, marks, values, messages, lists, tables, the one column-width solver `grid` that tables and
extension boards share, the help registration contract and the one
interaction decision, `Interaction`). Migrated command
modules, starting with `binding_command` (`tmt ls`, `name`, `add`, `rm`,
`whoami`, `unbind`), also render through it directly and write through its
`stream`. That crate is a leaf with no TMT dependency, so extension CLIs may
share it; the architecture guard enforces both. `mark::Mark` owns each shared
mark's symbol, canonical description and style token. Theme tests compare the
design registry against those descriptions; board-only registry entries remain
outside the shared enum. `extensions/tmt-office/rust/tmt-office-command` owns the public Office
grammar, typed requests and handlers. Core's reserved `tmt office` facade mounts
that grammar and calls the same handlers through an in-process `CoreAccess` port.
`tmt-cli/src/office_facade.rs` is the sole core registration and command-library
dependency; grammar, translation and invocation types are pure forwards through it;
direct `tmt-office` uses a bounded process implementation over public core JSON
commands. Both entry points keep the existing public Office command behavior.
The command crate also owns the companion invocation boundary (`office_companion`),
the CLI's bounded Office file readers and snapshot export, and `verify_release`,
the Office release verifier it hands to native installation.
`extensions/tmt-office/rust/tmt-office-service` owns the loopback service
lifecycle and its private receipt, shared by the Office commands, Office storage's
switch and the companion. No core crate declares an `office_*` module except the
facade, which PR B of #355 removes.

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
Geometry and painting remain unimplemented; row-track attributes remain for the
existing grid compiler to admit. The guard permits XML parsing, borrowed JSON
and the shared style leaf, never core, adapters, CLI or extension behavior.
Squad is the sole reviewed product edge, with a test-scoped adapter until the
row-track compiler adopts it. Existing board and list production paths remain
unchanged; `tmt-cli-style::grid` remains the sole row sizing policy.
The private component has no release; its inherited version/lock entry follows
the workspace, while product notices include only their actual dependency graph.

`rust/crates/tmt-cli/tests/architecture.rs` is a test-only import and
dependency guard. One reviewed manifest table owns both the fixed workspace
package names and their documented manifest locations. It follows the actual
Rust module tree, checks reviewed layer edges and shared declaration ownership. Its policy owns an exact dev-dependency
ledger (crate, canonical name and target, with a reason per row); aliases are rejected,
and the invoke leaf remains guarded for every dependency kind. The guard fails closed for unsupported
module remapping or incomplete discovery. It also checks that the CLI crates
reach the terminal only through `tmt_cli_style::stream`, and a grammar walk in
each CLI checks every command's help against the style
([enforcement](design/cli-style.md#enforcement)). It is a syntactic guard and never
replaces review of behavior or effects.

The optional `extensions/tmt-office/rust/tmt-office` executable remains a member
of the `rust/` Cargo workspace, with the same lockfile and `rust/target` output.
This package owns the companion entry point, embedded SPA and local HTTP service.
`extensions/tmt-office/rust/tmt-office-model` owns Office domain values, strict
codecs, immutable catalogs and data-only admission. It depends on core identity
syntax, numeric limits and content digests, never on Storage or runtime adapters.
Retained Storage imports that owner directly; there are no core Office re-exports.
Codecs own in-memory PNG processing, the companion's world reply admission and
snapshot projection, and the board wire limits; filesystem reads, publication and
process/config access remain adapters. Acquisition errors may retain
an `io::Error` value without giving the model an I/O operation.

`extensions/tmt-office/rust/tmt-office-storage` owns Office storage at
`<global>/office/office.db` (#353). Its schema v1 repeats the core schema 35
definitions of the 14 Office-owned tables, so migration copies raw cells, minus
the two `identities(id)` references that Office replaces with preflight. It adds
an Office-local retired-identity marker and a migration record; schema v2 adds the
activation marker written when `office.db` becomes authoritative, and v3 adds
the retired-room marker. Switched storage from an earlier Office schema upgrades
when opened. The crate reaches
core only through public owners: `config`, `file_lock`, the process runner and the
`StorageError` type with its `classify` mapping. It owns the Office repositories
(worlds and legacy blocks, profiles, prop and avatar catalogs, the discussion board
and whiteboards) on `OfficeStore`, and the `access` operations shared by the
one-shot companion protocol and the local HTTP service. Production opens go
through `OfficeStore::open_configured(&StorageLayout)`, which selects the store
from Office's own files and the receipt the migration coordinator maintains:
an existing `office.db` is finished (activation) or, if recovery reverts it,
skipped; otherwise the coordinator's fresh path runs, and an install whose
Office tables hold nothing but the seeded catalogs switches immediately
(`migration::switch_fresh`, the normal decision transaction and receipt, skipping
only the backup and service quiesce because there is no user data to protect and no
service can be running against an unswitched store). Any user data keeps the legacy
store on the shared core file until `tmt office storage migrate`. A receipt
without usable storage fails with the recovery error rather than recreating it.
The single-file constructor is test-only, so no production path can bypass a
switched store. The CLI side keeps only file readers, reply
decoders and wire limits in `tmt-adapters`; the core `tmt office` facade still
reaches storage only by invoking the installed companion.

Office reads core-owned identities and rooms only through the UUID-keyed
`tmt-office-storage::core_references::CoreReferences` port (`identity`,
`active_identities`, `room`, and a batch `resolve`), separate from the CLI selector
port `CoreAccess`. Production implements it as `ProcessReferences`: it runs the
invoking `tmt` (`TMT_EXECUTABLE`, else `tmt` on `PATH`, never itself) through the
supervised process runner, using `tmt --json identity ls` and the
`references.resolve` API operation, so the Office binary never opens or migrates
the core database. The in-process `CoreStore` is compiled only for tests and the
`in-process-core` feature, and a crate test forbids `Storage::open` and
`CoreStore::open` anywhere else. Core sets `TMT_EXECUTABLE` for one-shot companion
launches and for the local service when started from `tmt`: core declares its own
executable at startup (`core_executable`), so a CLI installed under any name hands
down its own path, and a non-core process (the companion's direct mode) passes on the
`tmt` that launched it.

The local service reaches core only through `local_service::core::LocalCore`, whose
production implementation runs the same `tmt` through `CoreClient`
(`tmt-office-storage::core_client`): startup readiness, request dispatch with its
advisory wake, request history and receipts, room save/retire/list, notebooks, and
profile presence and self-reported status. The browser is the local owner, so its
writes use the API's `"originator":"anonymous"` (no writer identity, exactly like the
CLI without `--identity`; it grants nothing beyond same-user CLI calls). Each handler
maps core's error codes through an explicit status table and reports anything else as
unavailable storage. Presence comes from `tmt ls --json`, which verifies tmux
endpoints, and status from the batch `identities.status` operation, so a profile poll
is two processes. Unit tests run the same operations in-process behind `LocalCore`;
a crate test forbids `Storage::open` and `tmt_adapters::storage` in service code.
The one remaining path check, `retirement_consumer`'s "does core's file exist" stat,
reads no contents so that `office sync` does not make core create it.
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
pins those sites. Known debt owned by #355: the remaining `office_*` modules in
`tmt-adapters` (the architecture guard lists them and rejects any new one), and
removing the retained Office rows and fences from core. The migration coordinator is the single
documented exception that opens the legacy core database (see below); it is a
legacy path, removed after the release that stops shipping schema ≤ 35 upgrades.

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
confined by the crate guard to `migration/switch.rs`. This legacy exception is
removed after the release that stops shipping schema ≤ 35 upgrades. Core schema 36 triggers then reject every write to the
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
It depends on core and the existing adapters, not the CLI.
`extensions/tmt-office/rust/tmt-office-pairing` owns validated deployment decoding,
bounded HTTP, protected pairing records and explicit platform credential stores
(`office_deployment`, `office_http`, `office_pairing`); no core crate depends on it,
so the CLI never links the credential-store backends. Serde derives reject duplicate/unknown descriptor fields; the URL
Standard library matches browser URL interpretation instead of introducing a
handwritten parser. Neither dependency enters core. The probe acquires no
credentials or network data. `tmt-office-model::office_protocol` owns the fixed typed
handshake; `tmt-office-command::office_companion` verifies active installation ownership
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

`rust/crates/tmt-cli/src/grammar.rs` owns the ordered core command registrations,
shared spec/option helpers and public help projection, and mounts the Office subtree
from `tmt-office-command::grammar`. Private modules under `grammar/` own command
builders by group: `launch.rs`, `presence.rs`, `rooms.rs`, `requests.rs`,
`identity.rs`, `settings.rs` and `installation.rs`. Root help/API, retired-command
and internal-completion registrations stay in the root. Group modules share root
helpers and do not import from each other.
`grammar/completion.rs` and `grammar/extensions.rs` retain completion and external
command recognition. Each visible core command
is registered from a `CommandSpec` (summary and examples) through
`tmt_cli_style::apply`; hidden internal commands have no help page. Squad registers each
command from a `CommandSpec` in `extensions/tmt-squad/rust/tmt-squad/src/specs.rs` through
`tmt_cli_style::command` and resolves `tmt squad help <command>` with `tmt_cli_style::route`.
Each owner's parser turns
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
`tmt-<extension> --help`. Root help lists discovered extensions in its `Extensions`
section, including executable names shadowed by core, which it marks ignored. PATH directory enumeration happens only for root help,
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
and exit status. `tmt-adapters::api` owns envelope admission and composition.
Its `api.rs` facade retains the dispatcher, protocol bounds, settings selection
and storage lifetime. Private `api/` modules own operation-family inputs and
composition for requests, dispatch, rooms, notes, changes, identity hooks, skills,
references and identity status. Identity, room, request history, dispatch and
notes retain their existing domain, transaction and resource encoders. The same
one-shot delivery helper serves Office and API dispatch. Explicit identity selects write attribution, not privilege.
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
operation ID and resend. See [extension API usage](contracts/extension-api.md).

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
record typed evidence in a temporary table: `identity.created`,
`identity.renamed` and `identity.retired` (UUID, lifetime, retired) and
`room.created`, `room.updated`
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

Provider `UserPromptSubmit` hooks use the same generic callback and aggregate
budget, returning attributed extension lines and, only when nonzero, one incoming
unacknowledged-attention count with an explicit-identity inbox pull command as
event-specific `additionalContext`. The count reuses the existing read-only
context snapshot; it is not a count of unsent or unanswered requests. No worker,
new counter or automatic delivery is created. Zero attention with no extension
contributions emits nothing. Hooks require an already running, verified binding whose
provider session and runtime incarnation match the caller, and recheck the
binding/preferences after callbacks before handing context to the provider.
They neither admit a session nor replay the SessionStart identity preamble.
Setup includes one synchronous prompt-submit entry in its consented plan;
existing SessionStart and opt-in Stop behavior retain their owners.

With no consent file or no enabled observer, a command performs at most one read
attempt of the consent file, on its first storage open, and spawns nothing;
commands that never open storage do no hook work at all. Office implements the
protocol by running its full reconciliation on `observe`, and stays inactive
until the user enables it.

### Core command surface

The Clap grammar owns primary names and accepted aliases. Listing uses `ls`,
removal uses `rm`, identity renaming uses `mv`, and record display uses `show`.
Long spellings remain hidden aliases and resolve to the same typed invocation;
help, examples and guidance use the primaries. Root `uninstall` remains distinct
from identity retirement. Core names and all aliases, including `mv`, are reserved
before external PATH dispatch. The recursive listing guard in `tmt-cli-style::audit`
covers core, embedded Office and the separate Squad grammar. Options, operation
enums, JSON API method names and persistence semantics are unchanged by spelling.

The maintained public surface is:

- `init`, `config`, `completion`, `learn` and `install` for local setup and
  guidance;
- identity and binding commands: `identity` create/show/ls/mv and metadata
  set/show/ls/rm with exact filters, `ls`, `add`, `name`/`this`,
  `whoami`, `unbind`, `rm`, `mv`;
- saved-identity notes through `notes path`;
- the versioned local extension interface through `api`;
- profile and exchange commands: `role`, `preamble`, `x ls|show|ack|ackall`,
  `reply`, `result`, `inbox`, `answer`, `talk`/`send`, `check`/`read`;
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
stored identity. `identity ls` and bare `preamble show` remain collection reads.
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

`tmt-core::names` owns canonical identity classification (pane-target syntax
belongs to each host, `tmt-core::host`),
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

A pane's binding marker (`@tmux-team.agent`) proves ownership by its IDs alone:
identity, binding, server and pane process. Its name is informational. It must be
a well-formed name consistent with its canonical form, but it may lag the stored
name, and every reader resolves the identity by ID and shows the stored name.
Rename (`binding::rename_identity`) changes the name and canonical name of an
unretired identity in the immediate binding transaction, under the same global
uniqueness as creation, so everything keyed by the UUID follows. A marker that
still carries the earlier name stays active; the post-commit cosmetic refresh
(`Tmux::update_binding_cosmetics`, used by rename, bind, run and `pane_badge`)
rewrites it to the stored name only when the marker is still this binding's.
Read paths such as `ls` and `talk` never write it.

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
state. Persistence is an optional driver interface: `RuntimeLifecycle::reads_state`
names the versions a driver reads, and `RuntimeRegistry::reconcile` discards state
it cannot read and drops sessions of unregistered drivers, which
`Storage::purge_unregistered_sessions` also sweeps. Driver state holds resume
essentials only (the model and, opted in, usage numbers), never transcript
content, arguments or secrets.

Schema 38 adds a resume-pending mark. A resume launch sets it on the exact
remembered session before the child starts, and any starting provider event clears
it with the stale mark. At exit, in one transaction, the session goes stale only
if that same session is still pending, the exit was non-zero and not 128+n, and
the provider's TMT SessionStart hook was installed at launch
(`setup::start_hook_installed`, read-only). Otherwise only the mark clears, so a
crashed launcher leaves a harmless pending mark, never a false stale one.
`tmt resume` and its `run --resume` alias never fall back to a fresh start. Only
that resume path purges unregistered drivers' sessions and reconciles unreadable
state, and it reports each change once. Read-only projections (`identity show`,
`ls --json`) read preferences without a write transaction and ask the session's
own driver for its model.

Schema 39 admits a second terminal host (Herdr, #479):

- `bindings.transport` accepts `tmux` and `herdr`. SQLite cannot alter a CHECK,
  so the migration rebuilds the table with only that CHECK changed, from a
  verbatim copy of its schema-38 definition. It refuses a table that differs
  from that copy, or any view or trigger that depends on it, rather than drop
  custom columns or rules.
- `request_attempts.host` and `request_responses.host` record the request
  fence's host. NULL is tmux, the only host that wrote earlier rows, so
  history is never rewritten; inbox routes carry no host.
- `host_servers` holds TMT's UUIDv4 for each server incarnation (socket, PID
  and start time) of a host without a server-level store of its own;
  `Storage::host_server_id` gets or creates it. tmux keeps its ID in a server
  option and has no rows there.

Binding queries still read tmux rows only until the core endpoint types carry
the host.

Schema 40 adds the change cursor behind the `changes.cursor` API operation
(contract in [extension-api.md](contracts/extension-api.md)). It is a one-row
`change_cursor` counter. Every core-owned table has three AFTER triggers,
`<table>_advances_change_cursor_on_{insert,update,delete}`, that advance it
inside the writing transaction, so no write path can forget. Migration
bookkeeping and the Office tables fenced by the schema-36 cutover have none.
An update counts only when some column's value differs (`WHEN OLD.c IS NOT
NEW.c OR ...`), so a reconcile that rewrites a row with the same values is not
a change, and `bindings.last_verified_at`, which `ls` refreshes while
reconciling presence, is not compared at all. A later migration that adds a
core table or a column, or rebuilds a table (as schema 39 rebuilt
`bindings`), must create or recreate its triggers: `change_cursor_tests` fails
until every table is covered or deliberately excluded and every column is
compared.

Schema 41 admits any host an approved driver serves (#570). The host columns
of `bindings`, `request_attempts`, `request_responses` and `host_servers`
accept any host name (1 to 32 of `[a-z0-9-]`, starting with a letter); inbox
routes still carry no host, and `host_servers` still has no tmux rows. Each
table is rebuilt from its own stored definition with exactly that one CHECK
replaced, its rows copied unchanged, and its stored indexes and triggers
(schema 40's change-cursor triggers among them) replayed. The source must
match what migrations 1 through 40 make in a fresh database, for these tables
and everything that mentions them; anything customized is refused. Like
schema 9, the rebuild runs with foreign keys off, so dropping
`request_attempts` does not cascade into `request_notifications`, then checks
them, all in one immediate transaction that rolls back to schema 40 on any
failure; foreign keys are restored on every exit.

Schema 42 adds `bindings.pane_incarnation`, beside `pane_pid`. It holds the
start token of the `ProcessIncarnation` that core observes for the pane shell
when a binding is created (`BindingEndpoint::pane_incarnation`). It never comes
from a host's or driver's text.

- **Existing rows:** bindings made before schema 42 keep NULL until they rebind;
  there is no backfill.
- **Verification:** `evaluate_binding` treats a known recorded value and a known
  observed value that differ as a reused pid, which is `EndpointLost`. NULL, or a
  failed observation, is unknown and proves neither loss nor sameness; the pid
  and marker rules decide as before.
- **Read paths don't observe:** on macOS one `ps` costs about 110 ms, so tmux and
  Herdr reads (`ls`, status, send) leave the observed value unknown and never
  compare; their pane IDs are already unique within a server incarnation.
- **Cost:** recording costs one `ps` per new binding.
- **External hosts:** pane IDs are declared by the driver, so external hosts will
  observe on verification, scoped to the binding's pane.
- **Format:** the column admits 1 to 256 printable ASCII bytes, not all blank,
  matching what `ProcessIncarnation` accepts.
- **Cursor:** the bindings cursor update trigger compares the column.

Schema 43 adds `identities.auto_named`, a private boolean defaulting to false.
Only an unnamed registered-runtime launch inserts true, independently of temporary
or saved lifetime. No name pattern or user-editable metadata grants this provenance.
The identities change-cursor trigger includes it. Existing identities remain explicit.
Caller-scoped `name`/`this` checks the exact live binding and this flag inside one
binding transaction, checks name uniqueness, renames the same UUID, optionally
promotes its lifetime and consumes the flag. Binding/session rows are unchanged;
ordinary binding and `add`/`marked` keep their existing conflict behavior. An ordinary
`mv` also consumes provenance when it changes the name.

The claude and codex drivers implement persistence with one document
(`runtime::driver_state`): version 1 is `{"model": <slug>}`, and version 2 adds
`"usage": {"tokens", "windowTokens"?, "observedAtMs"}` (the model is then
optional). Version 3 additionally stores session-scoped main-turn activity,
including the exact provider session and process incarnation. Documents without
activity retain versions 1/2, byte for byte. Version 4 adds optional cumulative
consumption and its private source cursor; all four versions are read. Optional
consumption is omitted if it would exceed the existing 1 KiB DriverState cap,
preserving model/context/activity evidence. Reading versions 1–3 retains their
model/context/activity without inventing counters; their first consumption
observation starts a new epoch with gap=true and complete=false. Older readers discard an unknown
version under the existing reconciliation contract. The model's only source is the `model` field
of a starting hook event, which both providers document (see
`runtime/fixtures/README.md`). Claude may omit it, for example after `/clear`, and
then the previous model stays. When a provider sends no model, nothing is stored.
A model is never inferred from transcripts or arguments. Resume replays a stored
model (`claude --resume <id> --model <m>`, `codex resume -m <m> <id>`, following
each CLI's recorded usage) only when the document is readable and the slug is a
safe single argv value. Otherwise it resumes with the provider's default.

Main-turn activity (#656) comes from TMT's own UserPromptSubmit/Stop command
hooks as installed by `tmt setup`. Claude runs these synchronously: admitted
transitions commit inside the hook call, before it returns. Ordering relies on
that provider contract, not a TMT sequence or receipt-time guess. Changing an
owned entry to `async` is a user-modification edge detected by setup inspection;
the event path does not re-read effective user/project/plugin settings. Codex
also requires its first-party turn ID; a Stop for another turn changes no
activity. Provider-only extras are deferred (`providers: {}`). The driver fixture
README owns source/version provenance and the documented-contract limitation.

`binding::session::activity` owns normalized transitions and clock validation;
`runtime::driver_state` owns their opaque persistence. Model/usage retention never
transfers activity across sessions or process incarnations. SessionStart resets
activity; start/end events do not establish a binding. Duplicate events do not
renew timestamps. The existing binding/preferences transaction checks the full
snapshot, and expired handlers do not begin a write. No detached writer exists.

Public `ls --json` rows expose `session.activity` with `state`, `sinceMs`,
`lastActivityMs`, and `providers`. Working/idle describe the last admitted
main-turn event, not all background tasks. Runtime uncertainty is unknown;
ended requires a conclusive core process observation. Timestamps are accepted
observation times, never inferred from silence, usage, terminal text or probes.
The state clock has no stalled threshold. Storage-only identity output does not
assert activity liveness. The Stop entry remains opt-in through `setup --usage`;
without it, no end event can be recorded.

Context usage (#519) is opt-in: `tmt setup --usage` adds a TMT `Stop` hook next
to the lifecycle hooks, and `--no-usage` removes only that entry. A turn end is not
a session transition. The worker verifies the caller exactly as for a lifecycle
event, and writes only when the binding's current conversation is the
remembered one the event names. It replaces the remembered state in one
compare-and-set transaction, and prints nothing, even on failure. This is the one
place a driver reads its own provider's transcript (`runtime::transcript`), and
only for usage numbers:

- the path must be a regular `.jsonl` file under the driver's own tree
  (`~/.claude/projects`, or `$CODEX_HOME/sessions`);
- it is opened without following a final symlink and without blocking;
- context usage reads at most the last MiB, skipping a line cut by that window;
- consumption reads at most one additional MiB (plus a boundary byte), from the
  appended Claude cursor or the latest Codex tail. No polling is introduced.

Unusable context usage writes nothing for that value. A start that changes the context
(startup, clear, compact) drops usage; a resumed Claude start records the
`context_tokens` it reports. Core never parses the document:
`RuntimeRegistry::remembered_usage` projects it as `resume.usage`.

Completed-request consumption (#872) is separate from context size.
RuntimeLifecycle/RuntimeRegistry project the driver's counters as optional
`resume.consumption` in ls/identity JSON: `inputTokens`, `outputTokens`,
`cachedInputTokens`, epoch, sequence, `observedAtMs`, complete and gap.
Cached input is a subset of input. Claude input adds uncached input, cache-read
and cache-creation; cached input is cache-read. Codex uses `total_token_usage`
input/output/cached fields; reasoning is already in output. Input plus output
counts provider-reported token units, not cost or interchangeable text volume.
These are accepted completed-request observations, not streaming throughput.

The first Claude observation baselines at current EOF with zero counters and
retains the last main message ID's hash; historical requests are not replayed.
The append-only scan counts each new contiguous `message.id` group once, carries
the last ID across reads, cross-checks `requestId` and usage equality, and skips
sidechains/synthetic records. Private dev/inode/offset and hashed IDs retain
equality without transcript text or paths. Counter evidence and its cursor use
the same trusted descriptor and captured file end. Noncontiguous older ID repeats are
not expected and may count again: exact historical-ID dedup is deliberately
outside the bounded one-KiB contract. In-place rewrites that retain inode and
do not shrink also violate the append-only assumption. A partial final line
waits for its newline, with `complete=false` and `gap=false`. Cursor loss, shrink,
replacement, a scan over one MiB, invalid main records or overflow starts a new
epoch at current EOF with `gap=true` and `complete=false`; a cut fragment is
discarded through its next newline, and history is never recounted.

Codex's first observation baselines at the provider's cumulative totals.
Unterminated final records wait for a newline with complete=false; an invalid
newest token_count is unavailable rather than falling back to older totals.
A component decrease, file shrink or replacement starts a new epoch/gap.
Epochs and sequence are driver measurement coordinates, not provider IDs;
sequence increases within an epoch when source evidence advances.
`observedAtMs` is acceptance time, not token generation time or a heartbeat.
Duplicate hooks without source changes retain the counter's timestamp/sequence.
A later complete scan clears the gap flag within its new epoch. Every start
resets consumption; failed reads leave it absent or unchanged rather than
inventing zero. Rate consumers baseline first/reset/gap observations and never
differentiate context usage. All writer verification/CAS/deadline behavior stays
with the existing hook owner; core does not parse the cursor or counters.

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
byte (`tmt run --channel` lets the driver append its own provider flags after the
user's and never rewrites theirs); the registry neither executes recognition nor
remembers arguments or paths.
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
After publishing, setup records the driver, settings file and launcher in
`<global>/setup-record.json` (`setup::record`), and `setup --remove` drops that
entry. An entry is fully determined by driver and launcher, so the record
stores no JSON. Hooks installed before the record existed are adopted when
setup finds exactly what it generates. An invalid record is preserved and
stops setup before any provider file changes.

Guided `tmt setup` (no driver, `setup_command/guided.rs`) plans from
`Registry::detect`, which reads only the filesystem:

- `Present` and `ConfigOnly` drivers get core skills in their skill roots
  (`skill_installation::plan_core`, then `publish_core`), and recorded extension
  skills are linked into roots that lack them (`plan_owned`, `publish_owned`).
  Apply publishes exactly the planned targets: `publish_core` classifies each
  target again under the installer lock and skips one that changed since
  planning. A target that is not TMT's is kept and reported, never replaced. A
  link into another TMT home's `skill-assets/<bundle>/<name>`, whose skill
  declares that name, is an outdated TMT skill: the plan names it, and apply
  backs it up before linking the current one;
- `Present` drivers with hooks get `setup::plan`, then `apply` and the record.

It prints only what is missing, asks once (`SETUP_CONSENT_REQUIRED` without a
terminal or `--yes`), and applies skills before hooks.

`tmt uninstall` (`uninstall_command`) plans every removal read-only, then
asks once; `--yes` never implies `--purge`. A running local Office service is
found in the plan; if its state cannot be confirmed, the plan stops with a
`tmt office stop` hint. Uninstall then works in order:

0. stops that service through Office's own stop path
   (`office_facade::service_control`);
1. hooks: the recorded ones, plus exact TMT hooks found without a record
   (`setup::removal`); removal is the exact inverse of setup's own edits, and
   a file left holding only `{}` is deleted; once no TMT hook is left in a
   directory, setup's `.tmt-setup.lock` there is removed while held;
2. owners' skills and bundled skill links, then the skill stores and records
   (`skill_installation::uninstall`);
3. extensions, then the CLI: links and release directories
   (`native_install::remove_product`);
4. the setup record;
5. with `--purge`, the data directory.

An unreadable record stops the run before any change. Anything that differs
from what TMT wrote is kept and reported, and so are the settings backups
(`settings.tmt-backup-*.json`) that setup and uninstall write. A failed step stops the run, and
running it again resumes.

Claude SessionStart/SessionEnd decoding, context encoding and runtime ancestry
belong to `drivers::claude`. A hook supplies observation only: an existing binding
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

The CLI foreground owner (`run_command`) keeps its public entry points, caller and
configuration selection, and storage startup/close in the facade. Private
`run_command/run.rs` owns the bound foreground launch and completion;
`run_command/resume.rs` owns command selection, resume pending marks and settlement.
The existing flow separates command selection, binding, spawn and runtime admission.
A first operand recognized as a registered bare runtime executable selects an
auto-named launch only when no active identity holds that token. A colliding bare
or flag-bearing shorthand refuses with explicit named-command alternatives; the
explicit name plus executable form retains its meaning. The parser admits opaque
provider tails, while authoritative identity lookup and runtime recognition stay
in launch composition. Auto-name creation uses the normal binding lifecycle and
prints one line before spawn. Failed spawn retires only its exact unchanged,
new temporary automatic binding; saved identities and renamed/replaced bindings
remain. Provider hooks, foreground completion and channel enrollment retain the
same UUID/binding owners across naming, without a separate anonymous session store.
A verified live or stopped previous runtime prevents a second launch.
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
offset. Shell adapters retain generated static completion and delegate `run` arguments
to the command's own shell completion. Launch completion includes registered
executables and identities; its storage-only composition resolves whether the
command begins at the first operand or after an explicit identity. They do not own a
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
public messaging still uses its existing tmux transport and request lifecycle,
except that a session that opted into a provider channel (`tmt run --channel`) is
reached through it and never pasted to (see "Provider channels").
Implicit caller selection first consults the runtime driver's `identify_caller`
action. `drivers::codex::caller` owns Codex thread markers and bounded process
ancestry inspection. It takes one PID/parent/command snapshot and walks it in
memory, reading arguments only for Codex ancestors. Both caller and runtime-start
observations share the fixed-path/locale `process::ps` runner and its missing-only
executable fallback. Tmux continues to own server, pane and marker verification.
Selected multi-process start observations and Herdr pane-parent observations use
`process::process_info` through optional evidence methods on `CommandRunner`.
The real runners acquire macOS BSD info through `tmt-sys::bsd_info` or Linux
bounded `/proc/<pid>/stat` reads. Start tokens retain the UTC, second-resolution
`ps-v1` representation; Linux combines boot seconds and process clock ticks.
Native acquisition is deadline-checked, not cached. If any selected native read
is unavailable, the entire batch uses its existing bounded ps fallback, retaining
batch-wide failure and cleanup semantics. Scripted runners default to that
fallback. Individual runtime checks and whole-table ancestry scans still use ps.
`tmt-sys`, owned by core, is the workspace's single audited unsafe boundary.
Only `tmt-adapters` may depend on it; the architecture guard rejects every
other consumer, including extension, dev, build and renamed dependency edges.
It is a leaf depending only on libc, with no build script and one macOS-only
safe function wrapping `proc_pidinfo`. Its fixed `proc_bsdinfo` buffer is
zero-initialized, and only an exact returned byte count is accepted. Every
unsafe block documents its buffer, initialization and size-check safety.
All other workspace crates retain `unsafe_code=forbid`; the architecture guard
checks manifest inheritance, the leaf dependency and the absence of build
scripts. The leaf denies unsafe by default and permits it only in that audited
function. Linux acquisition remains safe Rust in the adapters.
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
The action port's policy is written once, in `host::driver`, over each host's
`HostDriver` (see the host port below). Status delegates to the same full
server/pane/marker evidence evaluator, and send requires present evidence
before invoking the host's paste-and-Enter transport once. It preserves that
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
transcript. `tmt_core::endpoint::ProcessIncarnation` (a PID and that opaque start
token, from core's own inspection) is the one value for comparing a local process:
runtimes, launch owners, notification waiters and Herdr servers use it, each with
its own stored columns and lifecycle. Process disappearance, zombie state or a changed start identity reports
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
Consented extension contributions share this bounded context owner as defined
above. Session-only interfaces remain unimplemented as above.
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
A bind commits identity creation before its binding transaction. When that second
step is refused deterministically (`PaneAlreadyBound`, `NameAlreadyActive`,
`PaneNotFound`, `TargetChanged`), core retires the temporary identity this
invocation created, in one transaction that re-checks it is still temporary,
unretired and unbound, through the ordinary retirement path (role/preamble
removed, exchanges kept). Existing and saved identities are never touched, and
uncertain outcomes (`Unverified`, endpoint failure, deadline) keep the row for a
retry. A failed retirement is reported as a secondary diagnostic
(`BindingError::CleanupFailed`), never in place of the bind error.
Conclusive pane/server death or explicit unbind retires temporary names without
erasing retained exchanges; saved identities remain available offline. Saved
removal requires explicit force. Neither removal nor unbind kills a pane.
`ls` may show verified foreign-server identities, but `talk`/`check` routing
remains current-server-only: an absent name reads as not found, while an existing
name bound on another host reads as not active
without asking the caller's host, and one on another socket fails closed. Pane number, presentation title and socket pathname
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

The global file's `theme` object is presentation, not a core setting.
`ConfigFiles::theme` checks only its shape (an object of strings), reporting a
wrong one as a `ThemeProblem` rather than a configuration error, and never
affects loading the other settings; `tmt-core` knows nothing of colors. The CLI
(`appearance`) gives it meaning through `tmt_cli_style::Theme::parse`: `config
show` reports it resolved with its source and names a bad key in `themeError`
(an `error:` line in text) while still succeeding, because Squad reads `config
show` to find its own file; and at startup, only when stdout or stderr is a terminal and the user
set `theme.base`, `tmt` sets the process theme once
(`tmt_cli_style::theme::configure`), which `stream::stdout` and
`stream::stderr` apply at the stream's color depth. A missing or invalid theme
leaves every command on the terminal's own 16 colors. The Squad board reads the
same resolved theme from `config show` and layers `[squad.<name>.theme]` over it
(`look`), defaulting to `auto`; a bad global theme is a notice on the board, a
bad squad theme a `squad.toml` error. The global `appearance::parse` rejects
`auto` with a board-only hint; the shared parser accepts it for `squad.toml`
`[board.theme]` and `[squad.<name>.theme]`, including both picker scopes. No CLI
auto resolution path exists. `tmt-cli-style::theme::background` owns pure COLORFGBG/OSC 11 parsing and
luminance classification. Its bounded reader takes injected read/clock functions;
it opens no terminal and retains received bytes for the caller's input owner.
`Base::Auto` and `Theme::resolve` consume a supplied background signal without
changing token overrides or `Theme::default()`. The executable owns environment
observation, query eligibility, terminal I/O and detection lifetime. Squad's
`board::terminal` queries after raw-mode entry and before the input and refresh
workers start, then `look` caches the optional signal for this process. Concrete
bases do not query; COLORFGBG wins without I/O. The 100 ms query discards received
startup input, and its input-thread filter removes late OSC 11 responses before
board actions. Picker previews consume the same cached signal. Plain theme
listings read COLORFGBG only, report an unknown resolved base as null, and retain
configuration provenance separately from detected provenance.
Only `tmt-cli-style` names colors: the
native architecture test (`colors`) rejects color literals in other production
code, the Rust extensions included. `Look::row_span` owns the board's selected
reverse-fallback span policy: cells, pending text/mark and age labels share one
foreground, with semantic bold; real-background and unselected spans keep their
original styles. The view supplies selection and semantic context, never a
second depth/fallback decision.

`json_document` owns editable config/tmux metadata number compatibility:
IEEE-754 values with non-finite opaque values serialized as null. Known invalid
settings still fail. Raw object order is retained on targeted edits; this is not
an exact reply/body transformation or the receipt decoder's policy.

### SQLite and durable exchanges

`tmt-adapters::storage` owns one private synchronous `rusqlite` connection,
schema migrations 1 through 44, WAL/foreign-key/FTS5 setup, busy and transaction
boundaries, and close/checkpoint cleanup. Historical schemas and frozen fixture
provenance are evidence, not a second implementation. The adapter keeps raw
connections private and exposes narrow ports to core services.
WAL setup retries only classified Busy within one five-second contention budget,
including SQLite's own bounded busy waits, and verifies the returned journal mode
is `wal` before migration. Success restores the normal five-second busy timeout;
exhaustion preserves the original Busy error. Other setup and transaction failures
are not retried by this policy.
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
credential-free Git remote identifiers, not permissions; `tmt-core::repository_id`
owns their canonical grammar for both remote resolution and the board. Category discovery
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

Whether a request still accepts a first final is one service rule,
`first_final_refusal`: final submission enforces it, and the open-request read
(`open_requests`) applies it to what `storage::requests` narrows by the same
columns. "Waiting on you" is therefore an open-request question, not an
attention one: acknowledgment and live delivery settle attention but leave a
request open until a final or its acceptance deadline. `answer_target` selects
one open request by recipient and originator, never guessing among several, and
derives the route proof in-process from the recorded attempt, so `tmt answer`
submits through the same acceptance path as `reply` without exposing a receipt
([contract](contracts/request-response-v1.md#inbox-and-answer)).

`RequestRoute` distinguishes unbound direct-pane delivery from durable identity inbox
queueing. Identified talk is Inbox-first with one claimed full-payload live wake;
its public live output remains sent/completed. Pane attempts retain server/pane evidence; inbox attempts retain only
the resolved active recipient UUID and settle as `queued`, never `sent`.
`RequestService::enqueue` prepares the attempt, stores its exact prompt and
publishes recipient attention in one repository transaction. CLI inbox sends use
this path; pane effects retain the separate prepare/send/settle lifecycle.
`talk_command` preserves explicit inbox selection separately from its offline or
live-wake projection. Pending explicit-inbox output says no notification was
attempted and recipient pull is required; this presentation never changes the
route, claims, attention or queue acceptance. The public output contract is in
[contracts/request-response-v1.md](contracts/request-response-v1.md).
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
Schema 44 persists fixed reply-notice windows and rendered notice members under
`storage::requests::reply_batch`, independently of immutable final bodies and X
attention. `request::notification::batch` owns the quiet/deadline policy;
`reply_notice` composes enrollment evidence, enqueue, binding-fenced delivery and
one-shot settlement. `reply_notice_command` schedules finite detached workers,
with process-incarnation CAS claims before waits, sealed batch membership, and
per-frame attempt evidence before transport. An approval-blocked registered frame
is settled definitely unsent and does not stop independent later frames; joined
host notices retain the host's single final approval result without input fallback.
A unique SQLite sending claim serializes worker transport per binding, including
separate zero-window notices.
Only exact process-death evidence may release a stranded sending claim; attempted
frames retire uncertain and untouched members remain queued. Workers hold no
transaction while sleeping or probing. A failed send never replays; later eligible enqueue can
recover a proven-dead worker's never-attempted frames while settling its unresolved
attempted frames uncertain. Clean workers remove their own logs; failed workers
retain them.
Channel enrollment bypasses enqueue, and enrollment beginning during a window
keeps individual driver notices. The delivery owner retains all routing and paste
gates. `HostDriver::input_activity` reports elapsed real key evidence or Unknown;
core applies the configured quiet period. Ordinary missing evidence is Unknown;
failed probe cleanup aborts the worker and retains diagnostics. tmux matches attached clients' current
pane and reads `client_activity`; other hosts explicitly report Unknown. No
screen contents or provider prompt buffer is interpreted as typing. See
[request notification behavior](contracts/request-response-v1.md) for timing and limits.

`process::detached` owns startup acknowledgment and failure cleanup for one
request deadline observer or reply notice worker, and the worker's removal of its
own stderr log
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
The [request contract](contracts/request-response-v1.md#talk-completion) owns talk interruption
and retry guidance on either side of preparation.

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
cleanup.

The CLI and the `delivery` and `pane_badge` adapters reach the terminal host
only through `tmt-adapters::host::Host`. Extensions never do: they read presence
from `tmt ls --json` and the caller from `tmt whoami`, and the architecture
guard rejects any extension source, test code included, that names the host
port, the tmux or Herdr module, or core's `binding`, `endpoint` or `host`
model. The host port holds the binding session (the core `BindingEndpoint`
and `Driver` ports), caller and target resolution, snapshots, capture, send,
focus and pane cosmetics, over two hosts: `tmt-adapters::tmux` and
`tmt-adapters::herdr` (#479). A handle has a primary host; its session observes
new panes there, and probes, marks and clears every stored binding on that
binding's own host, so presence is complete from either host. Each host
implements `host::driver::HostDriver`: snapshot, probe, publish, clear, the
runtime in a pane, input and focus, at the driver protocol's granularity
(#570). The session picks the driver of an entry's host and runs one binding
policy over it: `host::driver::{status, send, focus}` decide which evidence
makes a binding present, when a runtime blocks input, and what a failure
means. A host without input (`has_input`) is `Unsupported` before any evidence
is read, and `focus_preflight` refuses before any evidence is read too. A send
that passes the evidence and runtime checks first offers the message to the
agent the host recognizes in the pane (`prompt`); only `Unsupported` (no
agent-aware input, or no agent seen) falls back to raw pane input (`input`),
and any other answer, such as an agent that is blocked or not ready, is final.
tmux recognizes no agents, so every tmux send is raw input. `DeliveryError`
(`host::delivery`) is the host-neutral input failure: the stage that failed,
whether text may have reached the pane, and the host's own cause. The
out-of-process client of #570 slice 3 implements the same trait. `HostError` and
the host `ActionError` wrap each host's error and read exactly as it. The
architecture guard rejects production references to the host modules outside
`host.rs` and their own directories.

Herdr is reached only through its documented CLI (`herdr <group> <command>`,
JSON out) under the bounded process owner, on the socket a caller's
`HERDR_SOCKET_PATH` or a stored server names, and refuses servers older than
0.9.1 in semantic-version order (so a 0.9.1 pre-release is refused). A Herdr pane ID is the terminal ID, which follows a pane through moves
while the public `wN:pM` (its target and display address) is reused after a
restart. A caller's Herdr pane counts only when its shell is an ancestor of the
caller (`process::ancestry`, shared with tmux); inside both hosts the nearer
pane wins. A Herdr server incarnation is its server process (the parent of
every pane shell) as a `ProcessIncarnation` core observes; Herdr keeps no server-level store,
so TMT's UUID for it comes from core's `HostServerIds` port, implemented by
`Storage` (`host_servers`) and resolved by `Host::resolve_servers` before any
binding transaction opens, so the transaction only sees resolved evidence. A
stored Herdr server is live only as the same incarnation; otherwise it is lost
only when its recorded process is conclusively gone. The marker is pane tokens
under source `tmt` (a long name spans continuation keys) and proves nothing
unless its IDs match storage. Herdr merges a report into the source's tokens
key by key, so each publish also clears the marker keys it does not set. Herdr delivery, capture, focus, badges and hook
context are not implemented yet: `send` is unsupported so core falls through to
the Inbox, a pane route refuses before input, and `check` and `focus` refuse.

Endpoint identity is opaque to everything but its host. `tmt-core::host::HostKind`
is the pure-data list of hosts, like the driver descriptors: each owns its stored
token, its pane-ID syntax (`is_pane_id`) and the text it reads as a pane target
(`is_target`), and `names::is_pane_target` asks every host. `ServerEvidence`
carries its host, so bindings, target evidence and request fences do too; core
stores and compares pane IDs as opaque strings. Evidence from another host is
`Unknown`, never proof of loss, and presence is grouped and scoped by host and
socket (`ServerSelector`). Storage writes `bindings.transport` and new request
fences' `host` from the endpoint. A stored host is only its name: tmux, Herdr, or
`HostKind::External` for any other valid host name, which reads whether or not
its driver is installed, and a NULL fence host is tmux. An external host's
pane-ID and target syntax come from the approved drivers, which the CLI
registers once at start (`host::external::register_approved`) in core's one
write-once registry; until its driver is registered, no pane ID or target is
its own. Whether a driver serves a host is decided in the adapters that run
drivers, not stored in the core type. A `Host` handle states why it was chosen:
`for_caller` (a caller-scoped command), `for_server` (a stored binding or request
endpoint) or `for_target` (an explicit pane target); its methods take endpoints,
never loose socket or pane strings. Only `tmt-core/src/host.rs`,
`tmt-adapters/src/host.rs`, `tmux/` and `herdr/` may spell a host's name, which
the architecture guard enforces for every crate but Squad (its tmux-only hotkeys
and clipboard are extension features). Names that a later host reads as targets
(Herdr's `wN:pM`) are refused only as new names: an identity that already holds
one keeps it for lookup and marker checks, and explicit resolution prefers it.

`tmux` uses explicit socket/server evidence, bounded command budgets,
owned buffers and no ambient host fallback. Explicit target resolution preserves
command deadline, I/O, spawn, output-limit and signal failures through the host
port as `RECONCILIATION_FAILED` (exit 1), rather than `PANE_NOT_FOUND` (exit 3).
A completed unsuccessful lookup or a successful reply without a valid pane ID
still yields no target; socket denial remains `TMUX_PERMISSION_DENIED` (exit 1),
and failed cleanup is never suppressed. Optional caller evidence retains its
best-effort absence policy. A failed paste or Enter is an
uncertain delivery and is never retried as if unsent.
Message delivery changes ASCII `!` to fullwidth `！` to avoid agent bash-mode
shortcuts (`tmt_core::driver::pane_input_text`). It is core's delivery policy
for any text typed into a pane, raw input or a prompt, on every host; hosts
and drivers add nothing. It is not arbitrary output rewriting. `check`
remains bounded terminal diagnostics, not a fallback response channel.

`response_input` owns bounded file/stdin acquisition and regular-file checks. It
polls against the deadline before each read and never mutates stdin descriptor
flags. The public CLI exclusively owns stdin during acquisition. These adapters
do not invent background threads or a second process runner.

### Agent drivers

Each agent driver is one declarative descriptor plus one adapter module:

- `tmt-core/src/driver/descriptor.rs` holds every `DriverDescriptor` and
  `tmt_core::driver::ALL`. A descriptor lists the name, executables, hook format
  and display hue. It is pure data, so parsing, completion and style read it
  without the adapters.
- `tmt-adapters/src/drivers/<name>.rs` holds the behavior keyed by that
  descriptor: `locate` (the configuration directories, skills root, legacy
  guidance and hook settings file, resolved against one captured
  `ProviderEnvironment`), and the runtime (claim, resume, lifecycle and caller
  recognition) when the driver has one.
- `drivers::Registry` joins the two in descriptor order. Setup, detection,
  skill targets, `run`, the runtime registry and caller recognition iterate it.
  A test requires exactly one adapter module per descriptor.

`tmt_core::driver::detection` decides from the filesystem alone whether a
driver is `Present` (an executable on `PATH`), `ConfigOnly` (configuration
directories but no executable), `Absent`, or `Broken` (on `PATH` but not
executable). `Registry::detect` gathers that evidence and never starts an
agent; guided setup (#333) and every status or install path use it.
`Registry::probe_versions` additionally runs one bounded `--version` per
present driver (5 s, 4 KiB, empty stdin) for diagnostics only: running an
agent can write under `HOME` (Codex creates `~/.codex/tmp`), so setup, install
and status commands never call it.

Only those two places spell a driver's name. The tmt-cli architecture test
fails on a production string literal equal to a driver name anywhere else.
Stored harness IDs are the descriptor names, so storage is unchanged.

### Codex native channel

`drivers/codex/queue` owns exact native request/receipt validation, while
`drivers/codex/transport` owns synchronous WebSocket framing and the absolute
I/O deadline. The only new transport dependency is adapter-local tungstenite,
exactly pinned with default features disabled and `handshake` enabled; no TLS
or async runtime enters core or shared ports. The architecture dependency guard
permits it only in adapters. The [Codex channel contract](contracts/codex-channel-v1.md)
owns limits, exact-build qualification and receipt semantics. Binary preflight
and owned-endpoint initialization enforce the qualified builds at their respective
boundaries; a preflight advisory never qualifies an endpoint. Provider-private
enrollment and the shared launcher establish authority before
this transport is used; no arbitrary endpoint becomes a delivery target.

`drivers/codex/record` adds provider-private opt-in/readiness persistence (#737),
using the existing nonblocking file lock for compare/write/remove. It stores
exact launch/process/thread coordinates but no capability material and owns no
binding transaction. The launcher must validate new-launch authority before
calling it; record-level takeover and withdrawal remain generation/incarnation
scoped. The same contract owns this persistence definition and its launcher/crash-cleanup rules. The registered consumer composes these records through the shared channel port.

`drivers/codex/server` and `attachment` own endpoint/foreground
planning. A launch-owned process group and private capability share one
cleanup owner; process cleanup precedes inode-checked file removal. Attachment
planning resolves cwd once and names an exact thread. The channel contract owns
the startup, credential and failure limits; live-provider continuity and model-free product routing have separate evidence.
Codex owns folder-trust onboarding: its user answers the TUI prompt; the channel
never approves it or writes trust configuration (see the channel contract).

`drivers/codex/lease`, `supervisor`, `delivery` and `channel_hooks` compose the
native consumer (#739). The supervisor owns the original
endpoint process handle; launcher EOF requests endpoint cleanup but preserves
the provider enrollment. Explicit withdrawal is reserved for no-child or
confirmed foreground reap. Pre-handoff startup failures retire only after
confirmed cleanup; a complete Ready frame may already have escaped, so a later
flush failure retains evidence. Provider records retain the pre-spawn pane address
and Unknown/Known foreground state; app-server readiness is never foreground
lifetime proof. The channel contract owns takeover, pruning and recovery limits.
`drivers/codex/recovery` implements `inspect` and `recover` through
`Store::recover`, which removes the exact observed record under its per-binding
lock, and cleans a generation directory file by known file (the names
`server.rs` owns) only when its app-server was recorded and is gone. Codex registers through the shared `Runtime.channel` port; native enrollment
selects the one terminal route before provider preference. Both the identity and
raw-pane paste boundaries query provider evidence, including reply notifications.
The shared launcher owns admission and confirmed-only withdrawal; the provider
owns its endpoint, record, foreground planning and one-shot queue transport.
The contract distinguishes accepted provider attachment evidence from the real
CLI/router tests. Permission options configure the owned thread and server,
not remote resume; unsupported forms fail in enroll before any spawn.

### Provider channels

An optional driver port lets a launch hand talk payloads to a running agent
without terminal paste. [`contracts/claude-channel-v1.md`](contracts/claude-channel-v1.md)
owns the behavior and the shipped-versus-planned status (#329); this is the
ownership map.

- `tmt_adapters::runtime::channel` defines the port. `RuntimeChannel` verifies the
  provider (`preflight`) and enrolls one launch (`enroll`) into a lease,
  `ChannelEnrollment`: the foreground command the launcher spawns verbatim, the
  provider child's environment (never ambient or persisted), optionally the
  provider session the driver created before the child starts, `foreground_started`
  and a consuming `withdraw`. `ChannelPlan` carries the identity and a `PaneAddress`
  (tmux server incarnation, pane ID, pane process), which the driver persists in its
  enrollment before the child starts. The launcher calls `foreground_started` once
  with the exact child incarnation it spawned and observed, before admission, and
  retires the lease only when no child was spawned or its wait returned; on any other
  path it drops the lease and the record stays. The driver
  plans the command from the user's command and owns everything that proves a
  cleanup is for exactly that launch; the CLI neither parses provider arguments
  nor inspects the lease. A driver registers it in `Runtime.channel`, which
  `RuntimeRegistry` exposes as `channel(harness)`. The directory for endpoint
  records is `ConfigPaths::channel_directory()`. The port and core stay free of
  provider and transport dependencies.
- Recovery is part of the same port. `RuntimeChannel::inspect` reports one
  binding's enrollment as an `EnrollmentReport` (record, generation, pane, every
  recorded process with an exact `observe_recorded` observation, what recovery would
  remove and keep, and the driver's verification text), and `recover` removes exactly
  one named generation under the driver's own lock, returning `Recovery` or
  `RecoveryError`. The defaults describe a channel that keeps no records; every
  driver that writes records implements both. `inspect_command`, `recover_command`
  and `valid_enrollment_id` are the single owners of the command text drivers print
  and of the ID shape. `RuntimeRegistry::channels` lists every registered channel
  for the CLI, since an enrollment names its driver only in its own record.
- Delivery stays in the existing routing. The driver's `send` is the preferred
  action of `delivery::send`, whose `send_preferred` falls back only after
  `Unsupported` or `NotSent`. An enrollment applies only to the exact launch that
  created it: with none, or with one that a different launch, positively proven
  current, has outlived, a driver's `send` returns `Unsupported` and the baseline
  transport runs; a session that opted in never gets `NotSent`, and when the
  launch cannot be verified its outcome is `Denied`. Other outcomes are
  `Uncertain` or `Completed`, and a completed write without a provider receipt
  is `DeliveryAcceptance::Unacknowledged`, a terminal acceptance that routing
  never retries or falls back from. The record layout and the launch comparison
  stay inside each driver.
- A paste never runs on "no record under this binding" alone. `RuntimeChannel::enrolled_in_pane`
  asks each registered driver, by the pane address its enrollments persisted and
  never through a stored binding (observation deletes the binding of a pane that
  lost its marker), whether a live or unconfirmed enrollment belongs to this pane. It
  returns `PaneEvidence` (`enrolled`, plus `skipped` records no driver could
  attribute) or an `EvidenceError` (fault, record at fault, the driver's recovery
  text). `RuntimeRegistry::enrolled_in_pane` merges the drivers (first error wins),
  and `delivery::guarded_paste` and `delivery::pane_channel_evidence` are the only
  gates in front of the two places that paste: the baseline fallback of
  `delivery::send` and the raw-pane path of `talk`. Which driver carries an
  identity's delivery is decided by `RuntimeRegistry::enrolled_harness` (an
  enrollment names its driver before any preferred harness exists) and only
  otherwise by the preference.
- `delivery::Delivery` carries `Unacknowledged` and `ChannelUnavailable(EvidenceError)`,
  and `delivery::send` returns an `Attempt` (the `Delivery` plus the skipped records
  to report). `Unacknowledged` settles as an uncertain wake and `talk` keeps waiting
  for the durable reply; `ChannelUnavailable` stops the request with
  `CHANNEL_NOT_READY`, `CHANNEL_UNREACHABLE` or `CHANNEL_ENROLLMENT_ENDED`
  (`DELIVERY_PREPARATION_FAILED` for other evidence), and `talk` shows the driver's
  record and recovery text. `Failure` carries an optional additive `deliveryState`.
- `drivers::claude::channel` owns the Claude record and the send classification
  behind `ClaudeRuntime::send`. It reads the stored binding and the per-binding
  record under the channel directory, applies the launch-applicability rule, waits
  a bounded time for a channel that is not ready, and exchanges one frame over the
  owner-only socket; the record grants nothing unless it matches the binding's
  launch owner and runtime observation. Without the discovered configuration the
  outcome is `Denied`, never `NotSent`.
- The same module owns enrollment, the pane lookup and the stdio MCP server
  (`channel/server.rs`). `ClaudeChannel::enroll` writes the per-launch record (with
  identity, pane address and, later, the published foreground) and returns a lease
  that withdraws only what it wrote and keeps the record while a recorded provider
  process may still run. `enrolled_in_pane` reads only `<uuid>.json` records and
  observes only the exact process incarnations an attributed record names (launch
  owner, foreground, provider): a record whose recorded processes are gone has
  ended, one that never recorded a foreground is unknown and terminal for its own
  pane, naming the exact `tmt channel recover`, and a record that names no pane is
  skipped and reported. `channel/recovery.rs` implements `inspect` and `recover` for
  Claude records over the same reader and lock. `enroll` takes over a record of the same binding only when it is
  positively over, or when only its owner was recorded and the launch is in the very
  pane it names, and prunes other ended launches from one process snapshot.
  Every mutation of a record or socket (the enroll write, the lease's foreground
  publication and withdraw, the server's readiness publish, bind and socket
  removal, and recovery) runs under one lock file in the channel directory, taken through
  `file_lock::exclusive`. Apart from `enroll`'s own takeover rule, each proceeds
  only while the record still carries the caller's generation and launch owner, so a
  stale launcher or server can never replace or remove a newer enrollment. The
  server's calling thread is its only output writer, and it can only complete an
  enrollment that `enroll` created.
- `tmt run --channel` (`run_command/channel.rs`, `run_command/run.rs`) is the only
  entry point that enrolls. It probes the provider version and shows the driver's
  advisory before binding, enrolls after binding and before the spawn, publishes
  the child through `foreground_started`, and retires the lease only on a failed
  spawn or a reaped child. The hidden `__channel-server` command
  (`channel_server_command.rs`) parses its argv into a `ServeRequest` and calls the
  driver's `serve`. A session that never opted in has no record and keeps its
  existing transport.
- `tmt channel inspect|recover` (`channel_command.rs`) resolves a target through
  the shared `target::resolve` or takes `--binding` as given, asks each registered
  channel, and renders the reports and outcomes. It holds no record logic: each
  driver applies the recovery rule the
  [Claude channel contract](contracts/claude-channel-v1.md#recovery) owns.

### Host driver protocol

Terminal hosts that TMT doesn't build in will run out of process as host
drivers (#570). [`contracts/driver-protocol-v1.md`](contracts/driver-protocol-v1.md)
owns the wire format. `rust/crates/tmt-driver-protocol` encodes it for both
sides:

- wire types and per-operation limits;
- `decode`, which is core's bounded, strict parsing and validation against the
  pane-ID and target grammar each driver declares;
- `serve`, a driver's entry point;
- `conformance::check`, which runs through any invoker.

A host's name, pane-ID prefix and target template (parsing, matching and the
overlap check between hosts) are defined once in `rust/crates/tmt-host-grammar`,
a leaf with no dependencies at all. The protocol crate wraps it with the
environment `caller` may read, and `tmt-core` may depend on it to recognize an
external host's stored pane IDs without taking on the wire crate or serde. The
protocol crate otherwise depends only on `serde` and `serde_json`, so a
community driver builds against these two small crates alone. The architecture
guard allows exactly those edges.

`rust/crates/tmt-driver-herdr` is the first driver built this way (#479). The
`tmt-driver-herdr` executable depends on the protocol crate, `tmt-invoke` (its
bounded process owner), `serde_json` and `semver`, and never on core or the
adapters; the architecture guard holds it to those edges. It answers `caller`,
`server`, `resolve-target`, `snapshot`, `publish`, `clear`, `capture`, `input`
and `prompt` through Herdr's documented CLI (floor 0.9.1) and `ps`. Its children get an allowlisted
environment without `TMT_DRIVER_CALL`. Herdr reports no server pid, so `server`
names the parent of a pane's shell, and a server with no pane reads as no
server. `publish` refuses with `not_found` unless the pane still runs `panePid`,
then reads the marker back. The marker tokens are the built-in Herdr host's,
byte for byte: both crates test against one fixture
(`tmt-driver-herdr/src/fixtures/builtin-marker.json`). `input` takes one
line: Herdr's `send-text` types raw, so a line break would submit before core's
Enter, and text with CR or LF is refused as `bad_request` before any effect. A
message reaches an agent pane through `prompt`: Herdr's `agent prompt` pastes
the whole text (bracketed when the agent enabled it) and submits it. Its
`agent_not_found`, `agent_blocked` and `agent_not_ready` become `no_agent`,
`blocked` and `not_ready`, and no other operation answers those codes. Both
pass the text as the last argument: Herdr reads it literally even when it
looks like an option, and it has no `--` separator. A plain
pane gets single-line input, otherwise the inbox. The driver doesn't declare
`focus` (Herdr has no command that focuses a pane by ID). It
isn't packaged and doesn't serve `HostKind::Herdr`; the built-in `herdr/`
adapter does until the driver replaces it.

`tmt-adapters::host::external` holds the core side of that boundary:

- **`registry`:** the approved drivers in `<global>/drivers.json`. Approval
  refuses a declaration that a built-in host or another approved driver would
  read as its own. It is two steps: `inspect` runs every check (ownership,
  digest, one `capabilities` probe that may take up to 10 s while the driver
  is still told the protocol's deadline, conflicts) and writes nothing, and
  `commit` checks the conflicts again against the registry as it is then and
  writes the record. `state` reports an approved driver as `ok`, `changed`
  (fingerprint, ownership or digest no longer as approved) or `missing`.
  A record's `source` is `path` (the default, not written) or `firstParty`.
  A path approval is pinned to its digest. A first-party approval
  (`inspect_first_party`) is of the driver the running release ships as a
  companion, whose bytes must match the receipt digest. It follows that
  release: `resolve_first_party` uses the record while the release ships
  the approved digest. After an upgrade it describes the new driver (its
  receipt digest and one `capabilities` probe) and adopts it under the same
  approval, asking nothing, only when it declares nothing beyond what the
  user approved: the same protocol and pane-ID and target syntax, and no
  operation or `callerEnv` variable outside the approved ones. Same name and
  syntax leave every conflict check as it was. Anything else is a gap: a
  release that ships none reads `missing`, and a driver that asks for more,
  is renamed or doesn't match its receipt reads `changed`. Either way it is
  unavailable at run time until approved again, and `state` returns the
  reason, which `tmt driver ls` shows. Every read-modify-write of
  `drivers.json` (approval, removal, adoption) holds `drivers.lock`, and the
  write is a staged file renamed into place.
- **`tmt driver` (`tmt-cli/src/driver_command.rs`):** the registry's front
  end. A record is written only with explicit consent, never by install or
  upgrade. `install <path>`, or `install <name>` for a first-party driver
  (a bare name with no `/`), refuses before asking, then shows a detail view
  (`detail::write`) of the version, protocol, executable, SHA-256,
  operations and the environment `caller` reads, and asks `Approve host driver <name>? [y/N]` through the
  shared consent prompt. `--yes` skips the question. A run that can't ask
  (no terminal, or `--json`) refuses with `DRIVER_CONSENT_REQUIRED` and writes
  nothing. `ls` is one `HOST DRIVERS` list section: a row per driver with
  its state mark (`●` ok, `✗` changed, `○` missing), name, version, state
  and path, `tmt driver install <path>` (or `<name>` for a first-party
  one) as the trailing action of a changed or missing one, and a note with
  each such driver's reason (`reason` in JSON). `rm` withdraws an approval without asking; an
  unknown name is `DRIVER_NOT_FOUND`. Bindings on a removed driver's host
  stay stored and read as unavailable.
- **`DriverProcess`:** runs one operation through the bounded process owner,
  under the operation's deadline and output bound, with `TMT_DRIVER_CALL=1`. It
  decodes the answer against the driver's grammar.
- **Trust:** `executable_trust` is shared with extension hooks. It checks
  ownership and the stat fingerprint before every call, and the digest once per
  process.
- **Recursion guard:** a `tmt` started with `TMT_DRIVER_CALL` refuses every
  command but help and `--version` (`DRIVER_CALL_REFUSED`).
- **`Drivers` and `ExternalDriver`:** a `Host` carries the approved drivers
  (production constructors read the registry; the test `_with` constructors
  approve none). A binding session opens one driver per external host on first
  use and serves it as an ordinary `HostDriver`: `snapshot`, `publish`, `clear`,
  runtime and the pane incarnation. A host without a usable driver, removed or
  changed since approval, answers `Unavailable`, which never proves loss.
- **Core-led evidence:** an external server's identity is core's own `ps` start
  token for the pid the driver's `server` names; the driver's `startTime` is
  advisory and never stored. A probe is decided by core: the recorded server
  process gone or replaced is Dead, the same process plus the driver's snapshot
  is Live, anything else is Unknown. The driver's optional `probe` operation is
  not called. `process::runtime::observe_starts` covers the server and scoped
  pane shells using native process evidence, with a bounded batched ps fallback.
- **Caller and targets:** `CallerEnvironment` carries only the variables
  approved drivers declare for `caller` (`driver_env`). A driver's `caller`
  names a pane; core counts it only when that pane's shell is an ancestor of
  the caller (`process::ancestry`), and the nearest verified pane wins across
  tmux, Herdr and external hosts. Without a verified external pane the choice
  is exactly the built-in one. The handle then keeps that pane and its socket
  for `caller_pane`, server resolution and `resolve-target`. `Host::for_target`
  picks the host whose registered grammar reads the text as a target (tmux, the
  broadest, is the default), and an external target resolves through the
  driver on the caller's server or the driver's default one. Only a definite
  answer (no such pane, no server, no approved driver) is "not found"; a
  driver that fails or runs late is a failure (`RECONCILIATION_FAILED`).
  Caller detection stays best-effort: a failing driver is just not the
  caller.
- **Delivery and inspection:** a driver that declares `input` takes messages.
  The prompt-first `send` asks its `prompt` when declared and maps the answer
  as the contract says (`no_agent` or `unsupported` falls back to raw input;
  `blocked` is `AwaitingApproval`; `not_ready`, `not_found` and `bad_request`
  were not sent; anything else is uncertain). Raw input is staged like tmux:
  paste with `enter: false`, core's paste-to-Enter delay, then Enter alone,
  each call with its operation's own deadline. A pane-addressed message
  (`Host::send`), `check` (`capture`) and `focus` go through the driver too;
  focus on an external host needs no tmux client, and its `viewer` names the
  server whose views moved.

The atomic owner-only replacement of such settings files is `private_file`,
shared with the extension hook consents. The approved drivers' syntax is
registered with core at start (see the host section above). Bindings on an
external host are made from its caller or an explicit target, and read,
published, cleared, messaged, captured and focused through its driver. A driver
without `input` keeps today's behavior: a send is `Unsupported`, the request is
kept, and `--inbox` queues.

## Managed skills and native installation

Managed agent guidance is a separate filesystem concern. The canonical
`tmt_core::skill_catalog` is the one list of bundled skill names and groups
(core or Office), in bundle digest order. `skill_installation::catalog` pairs
each name with its embedded bytes and records the earlier bundle layouts that
upgrades still verify. `Catalog::new` joins owned skills from owner records
without letting an owner shadow a core name. Names, sources, inventories and
ownership checks derive from these, and the tmt-cli architecture test fails on
a list of skill names anywhere else. The `tmux-team`, focused `tmt-inbox`, and
optional `tmt-office` skills are embedded as one versioned asset bundle by
`skill_installation::assets`; digest-addressed
materialization, provider detection, target selection, links, backups, registry,
drift and lock handling live under
`rust/crates/tmt-adapters/src/skill_installation/`. Core install exposes only
`tmux-team` and `tmt-inbox`. Explicit Office install or upgrade exposes
`tmt-office` in detected provider roots and any custom root that still contains
an owned core skill. CLI upgrades refresh recorded Office links without creating
missing integrations. The driver descriptors (see Agent drivers) are the only
provider inventory. Skill installation does not open application configuration, SQLite
or tmux, and never silently replaces an unmanaged path.
Ownership requires a matching known skill name and a link into this TMT home's
canonical `skill-assets` store. Existing generations retain digest and inventory
validation; a missing generation is a dangling TMT link, eligible for refresh or
removal. A real directory, mismatched name, outside link, or modified source is
preserved as a conflict. Historical bundled Office target intents join owner
records for explicit extension removal, and completed removal retires those
intents, including preserved user conflicts, so core refresh cannot resurrect
or reclaim an integration the user removed.

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
installs skip them and CLI refresh reports them as skipped. Removal is by owner, or
by a named subset of that owner's skills (`skills.remove`'s optional `skills`), so an
extension can retract one optional skill without touching the rest. It deletes
only links into the owner's store; drift reports owned targets that no longer
point at their owner's current content.

The core skill sources stay under `skills/`; the three Office skill sources live
under `extensions/tmt-office/skills/`. `skill_installation::assets` still embeds
those Office sources into the core bundle, so core compiles bytes from the
extension tree. This is retained core-to-extension extraction debt owned by a
later #328 slice, not a second skill source or a provider-specific copy.

Native executable installation is a different owner under
`tmt-adapters::native_install`:

Core's fixed `native_install::Product` policy owns package identity, inventory,
installation namespace and command links for the CLI and the official extensions
(Office, and Squad with its two links `tmt-squad` and `tmt-sq`). It has no
filesystem or network effects, and archive data never adds a product. The hidden
offline installer accepts an explicit product (CLI by default), and every product
uses the same acquisition, receipt and atomic publication path. Each extension's
command links, lock and current release are independent of the CLI's; existing
CLI receipts retain their format. Manifest selection uses product and
target together, rejecting ambiguous or multiply owned artifacts. This internal
path also serves public Office installation. `office_command` owns consent and
typed composition, not a second downloader. Default Office prefix is the user's
`.local`, independent of application configuration; `--prefix` selects another
owned installation. Public distribution and pairing remain separate gates.
GitHub selection filters CLI `v`, Office `tmt-office-v` and Squad `tmt-squad-v`
tags independently.
Downloaded bytes feed the same bounded artifact verifier directly; there is no
extra download-to-disk/read-back stage. Publication runs the caller's release
verifier on the written candidate before its receipt, so a rejection keeps the
previous release current. A product whose row requires a verifier (Office) is
refused without one before anything is written. Office callers pass the bounded
versioned probe from `tmt-office-command`; core's installers (`tmt extension`
and the hidden `__native-install --product office`) borrow it through the
facade's `release_verifier` until PR B of #355.
Removal (`uninstall_extension`, for any extension product) validates ownership
of every command link, refuses a foreign same-named command, and deactivates the
links without deleting releases or application data. It is recoverable, not a multi-file atomic deletion:
a missing command link with a retained activation is reported by `extension ls`
as `partiallyRemoved` with an exact removal command, and explicit uninstall
can finish that state. Listing this state does not execute or mutate it.

`tmt extension install|upgrade|rm|ls` (`tmt-cli::extension_install_command`)
is the public surface for the official extensions over this path. Its facade
retains dispatch, consent, errors, interruption, rendering and uninstall; private
`extension_install_command/` modules own install, repair, list/upgrade and skills
settlement through the existing native-installer and owned-skill adapters. The names
come from the fixed product table, never from PATH or archive data. Installable
eligibility is separate from historical product recognition: Office is frozen,
so install and explicit extension upgrade refuse before consent or acquisition.
Root upgrade skips Office without inspecting its installation. Listing omits an
absent Office, marks an existing or partially removed Office as frozen, and never
looks up an Office upgrade, even with `--check`. Historical receipts and Office
skill catalog names remain available for listing and consented removal. Other
install, upgrade and uninstall operations require consent (`--yes`, or an interactive prompt), and refuse a
non-interactive run without it. `ls` reads local receipts only. `--check` adds a
bounded release lookup (`latest_release_version`, metadata only), and a failed
lookup reports `unknown`. Shadowing canonicalizes every `tmt-<name>` on PATH and
reports those that resolve elsewhere, without executing them. Root help groups
discovered extension names that resolve to the same file (`squad (also: sq)`).
`tmt office install|upgrade` calls the same CLI-owned `require_installable`
guard before entering the Office handler, retaining one frozen rule and message.
The facade's status and removal operations keep their Office-specific flow.

An extension's agent skills belong to one owner named after it (`squad`,
`office`) in the owned-skill registry (`skill_installation::owned`). After
activation, `native_install::release_skills` re-reads the release's skills tree
under the installation lock and checks every byte against the receipt; a damaged
tree publishes nothing. `install --skills` publishes the whole tree; a terminal
install without it asks once; any other run names the skills and how to publish
them. Install and upgrade refresh the tree skills the owner already holds and
remove, by name, those the new release dropped, so other skills the owner holds
(such as playbooks) are untouched. `uninstall` removes every skill the owner
holds, from every target, because a skill that points at a removed command is
broken guidance; its single consent prompt names the skills and targets.

A release may also carry a companion executable beside its own
(`Product::companions()`): the CLI may carry `tmt-driver-herdr`, the
first-party Herdr host driver (#479). A companion is optional, so a release
from before it existed still verifies:

- the manifest may declare it once, and the archive must then hold it as an
  executable regular file; an archived companion that isn't declared, or a
  declared one that isn't archived, rejects the release;
- publication writes it beside the executable (0755);
- the receipt records its digest exactly when the release carries it, and
  inspection re-verifies it. A file without a recorded digest, a recorded
  digest without the file, a changed file or a lost execute bit fails closed.

The running tmt verifies and publishes an upgrade, so readers learn a
companion one published release before any archive carries it.
`native_install::active_companion` names a companion of the running, active
CLI release and its receipt digest by reading only the receipt, cheap enough
for every command; whoever runs the companion checks its bytes.

An extension release (never the CLI) may carry a bounded agent-skills tree,
`skills/<name>/<path>` (`native_install::skills_tree`). It has the binaries'
integrity:

- the manifest declares the tree with the single asset `skills`, the form
  cargo-dist gives an included directory; the CLI may not declare it, and an
  archived tree that is not declared, or a declaration with no tree, rejects
  the release;
- the file inventory comes only from the archive, whose SHA-256, verified
  before parsing, covers every byte; the bounds below apply while decoding,
  before any file is kept;
- extraction accepts only regular files under `skills/` and directories that
  hold one;
- paths follow the owned-skill name and canonical path rules
  (`skill_installation::valid_skill_name`/`valid_skill_file`);
- bounds are 16 skills of at most 64 files, 1 MiB each, with a `SKILL.md` per skill.

Any violation rejects the whole release before publication. The receipt records a
digest per skill file, and inspection re-verifies them. Any unrecorded file,
link or special entry fails with "Installed release inventory has changed", the
same error a reader that predates the tree raises, so both fail closed. Extension
receipts are bounded by a limit derived from the skill bounds; the CLI receipt
stays at 16 KiB.

- `artifact` consumes cargo-dist metadata and a matching archive, checking
  target, manifest membership, SHA-256, bounded compressed/expanded input,
  notices and executable contents;
- `publication` stages a release under an invocation-owned prefix, writes
  receipt/current/command links atomically under the installer lock, and keeps
  old owned releases until ownership and integrity checks permit cleanup;
- `receipt`, `release`, `managed` and `upgrade` implement local provenance,
  active-release inspection, channel/pin policy, verified HTTPS acquisition and
  forward-only activation; which GitHub prerelease flag a release may carry is
  per product (`Product::accepts_prerelease_flag`, matching the publication
  policy in `native-release-policy.mjs`); a receipt's recorded repository must
  be `pj-tmt/tmt` or its historical names `wkh237/tmt` and `wkh237/tmux-team`
  (read-only compatibility for existing receipts); release lookups and new receipts
  always use `pj-tmt/tmt`;
- `native_install_command` and `native_upgrade_command` are thin CLI
  compositions. Application data and provider skills are separate owners.

`native_upgrade_command` upgrades the CLI and refreshes its managed skills before
asking the newly installed executable to upgrade installed official products.
The bounded hidden `__native-upgrade-extensions --json --plan` command supplies
pending versions; the parent owns one terminal consent question and sends that
exact plan on stdin to the new executable with `--yes`. It validates the bounded
plan/result reports and exit status. Unsupported older targets fail with a rerun
hint; old-process extension logic is never used as a fallback.
`extension_install_command::upgrade_all` in the new executable discovers products
in its managed CLI prefix and retains each channel/pin. A selected
version uses the same native acquisition/activation path without creating an exact
version pin; extension verification and skill settlement retain their existing owners.
JSON/non-terminal runs without `--yes` report `consentRequired` without mutation.
Product failures remain independent in the aggregate report; CLI failure stops the
extension phase, while a pinned CLI permits it. No rollback or second installer exists.
`NATIVE_UPGRADE_FAILED` includes an explicit diagnostic `cause` in JSON. HTTPS
failures retain the transport cause or diagnostic class without echoing rejected
URI/proxy credentials. Managed-skill conflict reports retain the path array and
provide one shell-quoted backup command per preserved entry; recovery moves the
entry outside skill discovery without changing its source.

Explicit extension `install --repair` is a separate recovery composition in
`native_install::repair`, for GitHub and local-archive receipts. `receipt`
separates bounded metadata/recorded-path validation from payload verification;
normal readers still require both. An eligible verification failure carries
`RepairRequired` to the CLI, which owns the single quoted repair-command hint.
Repair admits only safe owned layouts and no-follow regular files/directories,
acquires the exact recorded artifact with matching provenance and digests, and
preserves version/channel/pin. Observation parses the same bounded receipt bytes
used for revalidation. Local repair requires the original archive and matching
manifest; it retains local provenance and cannot replace a GitHub source.
Acquisition holds no installation lock; the stable lock and a pre-activation
current/receipt revalidation fence publication.
`publication` shares candidate staging, durable activation, cleanup and typed
post-activation failures between normal installs and repair. The damaged release
is retained untouched at its original path, including foreign entries, rather
than treated as content TMT may overwrite or delete. It is never a verified
execution candidate; no automatic retention cleanup is implemented. A healthy
repair is a no-op. Local receipts remain installation evidence, not signatures;
repair does not claim protection from a hostile same-UID writer. Provider skill
refresh remains with the existing verified-tree/skill-owner composition.

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
It is a workspace member for the shared lockfile and toolchain only. Its reviewed
runtime TMT dependencies are the neutral leaves `tmt-cli-style` and `tmt-invoke`.
Its `tmt-tui` dev dependency supplies a test-scoped binding adapter borrowing acquired `Member`
values and reusing `ColumnSource`/`Format`, with no core/provider acquisition or sorting.
`tmt-tui` becomes a normal dependency when production adopts it (#774/#776).
No TMT crate depends on Squad; the architecture guard enforces both directions
for Cargo dependencies and source references. Squad reaches TMT
through `TMT_EXECUTABLE` (or `tmt` on PATH), using public `--json` commands and
`tmt api`, with `runner` mapping results/errors to `tmt-invoke` for bounded capture.

Squad keeps no store. A squad is the core room `squad-<name>`. Member fields are
identity metadata `squad.<name>.<field>`, so one identity can belong to several
squads and removal clears exactly one namespace. Leadership is the reserved
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
leads to `false`, preserving all role text. A concurrent metadata write after
preflight can still split this sequence; it is not a transaction. Removal clears the reserved key with the
rest of that squad's namespace. A required new marker at the identity metadata
capacity limit returns the existing core error before the role pairs or join,
rather than silently changing leadership. Reads never convert state.
`ls` (alias `status`) joins
one `rooms.roster` snapshot with `ls --room` presence. It always returns one
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
`board::changes` watches, so the board reloads early. The board sizes it with `tmt-cli-style`'s one solver
(`grid::solve`, `grid::span`, `grid::fit`, `grid::fit_lines`). `rows::Column`
uses `grid::Basis` for cell or percent widths; bounds stay in cells. `rows::Rows`
owns positional coverage: cells start at track zero, so the maximum line span
covers a prefix, including empty cells. `Rows::solve` passes only that prefix to
the shared solver and pads uncovered trailing positions with `None`, retaining
span indices. Uncovered columns remain projection sources; their JSON metadata
adds optional `valueOnly: true`, omitted for covered columns. `Column::display`
ignores their sizing settings so flat text lists retain natural values. No shared
solver or all-covered output contract changes. The solver
resolves percentages against data width after marks/borders/gaps, rounds by
largest remainder, clamps cell bounds (percent columns default to a four-cell minimum, capped by an explicit max), then grows. Hiding recomputes the shown set.
`grid::fit_lines` owns escaped, exact-cell-width bounded wrapping, with a final
end ellipsis. Column metadata preserves percent strings and adds `overflow`
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
and sequence/time order. Complete deltas enter a fixed 720-slot ring of 5 s
receipt-time buckets: observed batches, never reconstructed completion times.
Missing, invalid, gap, decrease, new-session or recovery evidence establishes a
baseline without invented tokens. Prior-epoch tokens and gap evidence expire by
the selected window (5 s, 1 min, 30 min or 1 h). Never-reporting members are excluded
and listed in help; previously reporting members with lost evidence make the
known sum a lower bound. Failed reads become partial after two sampling periods.
Provider observedAt is order evidence, not a heartbeat. Input plus output counts
cached input once; mixed providers sum reported token units, not costs/text volume.

`config::TokenRate` layers team preset, global `[board.token_rate]` and per-squad
keys; Team alone defaults on. Built-in all/leads tabs omit the meter. The default
window is 1 min; 5 s is offered only at exactly 5 s sampling. The bindable
`token-window` action (`w` in both host presets) cycles available windows, outside
text inputs. Longer windows divide known deltas by the covered span until full;
the label discloses that span. Windows with no usable interval hide, including
warm-up (10 s for windows other than 5 s). Measured zero renders `0`, or `≥0` for
missing reporting coverage. `board::meter` owns cubic counting digits (600 ms,
250 ms frame spacing and an exact final frame), smooth retargeting and immediate
window switches/reduced motion. Its eight trend bars derive from the ring;
slices are rounded up to 5 s, so trend spans are 40 s/80 s/30 min/1 h. No evidence
is blank; measured zero is ▁; nonzero bars use ▂ through █.
The meter renders one right-aligned number/unit/label/trend group, using a
seven-cell maximum number region and no padding between its parts; the trend
preserves its eight slots, including empty slices. The meter owns step-aside: drop trend, then only a full default-1m label, shorten
`tok/s` to `/s`, then hide before cutting lead/attention text. Covered-span and
non-default labels persist. The normal cached render and ratatui diff own output;
backend-cell tests prove meter-only ticks emit inside the meter band, with no
parallel paint path. Window cycling is runtime state, never a config write.
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
The catalog uses the existing split/fold grammar: `team` reuses the current
team arrangement, and `focus`, `notes`, `detail` and `wide` supply arrangements
and initial fold settings only. `Config::board` resolves a hand-written
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
by custom or scoped arrangements. No core settings writer is introduced.

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
carries the meaning without color. The leads and all tabs read rosters without
an observer and show no marks. Content age is unrelated to `App::loading`, the
previous squad's frame while a switch loads.

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

`board::tabs` owns the tab
keys: a squad's name, or a built-in key starting with `@` (`@leads`, `@all`),
which no squad name can. `[tabs] order` and `hide` arrange them. The leads tab's view is
built by the same worker from each squad's roster document, joined with one
`ls` read for presence. Its rows carry their squad, so talk goes to that
squad's room and a jump is the ordinary `tmt focus`. The all tab's rows are
squads, not members: their `tab` action opens the squad's tab, and member
bindings don't apply there. `jump lead` (`L` in the tmux preset) resolves a
lead name in `App::lead`: the document's `squad.lead` on a squad tab, the
selected row on the leads tab, the row's `lead` field on the all tab; it then
takes the ordinary jump request, so the popup closes and `back` returns. Moving a tab (Shift+←/→, or a drag on the tab
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
`split` owns how panes sit: a tree of row and column
splits whose children have a percentage or a grow share (ratatui `Percentage`
and `Fill`), nested up to three levels; `layout` is its full form and the
`direction`/`panes`/`sizes` keys its one-level form, and split solves either
by one recursive walk before the board draws the resulting panes. The tree's reading order is the focus order, skipping folded panes. `split::Split::solve` receives
an explicit runtime fold set: vertical folded children reserve one title line,
horizontal folded children reserve a compact title-width column, and a fully
folded subtree propagates its title footprint. Expanded siblings divide the
remaining space by their configured percentage/grow shares. An empty fold set
uses the original constraints. The configured Board/Split never changes during
a toggle. `Config::board` strictly validates the initial `collapsed` pane list
for split mode, plus `fold_below = { width, panes }` with width 1–1000 and
panes present in the resolved layout. Team sets width 100 for detail and replies.
`App` resolves the effective fold set from board body width and immutable defaults;
per-pane user overrides win at either width. The terminal draw owner supplies the
full-width body measurement before rendering; the view only passes the resulting
set to `Split::solve`. `App` owns bounded per-tab session overrides, preserving them
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
title is clickable to expand. The detail pane appends full projected `row.fields` values for board columns not already represented by its header, task, note, activity or links, in column order; it escapes and wraps them without grid fitting, source lookups or provider calls. The notes pane shows the squad lead's own saved-identity notebook, read-only;
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
transaction; each reports what it applied, and a re-run converges. `squad.toml`,
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

Office's opt-in `playwright.visual.config.ts` reuses the local HTTP fixture and
real browser renderer for reviewed platform/furniture/HUD pixel baselines. Its
scenario-local read-only world is not a native admission or persistence oracle.
The browser partition verifier keeps these tests separate from standard CI and
capacity diagnostics; [Development](DEVELOPMENT.md#personal-office-milestone-acceptance)
owns execution, platform-specific baselines and explicit visual-review updates.
Geometry, gesture history and native durability retain their existing test owners.

Retained tests are organized under `typescript/test/native/`, `typescript/test/e2e/`,
`typescript/test/tooling/` and `typescript/test/support/`, with Rust unit/integration tests beside
their owners. The CLI's `tests/support` module owns the isolated environment and
direct-child lifetime shared by its stdin-signal and request-observer fixtures;
[Development](DEVELOPMENT.md#native-process-and-shared-tests) owns the isolation contract.
They use independent SQL/schema oracles for SQLite behavior and
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
close and confirmed group absence. A SIGKILL or initial group-probe permission
error is tolerated only after direct-child close and a subsequent ESRCH group
probe; live or unknown
groups still fail within the cleanup bound. An unconfirmed group is never
signalled; other initial probe or signal errors remain failures. Cleanup failure
is bounded and retains fixture files for diagnosis. Sandbox disposal cancels outstanding runs before
removing files. This is not containment of descendants that create new sessions,
and does not replace the separate Docker harness or release verifier.

Real-companion native scenarios live in Office-owned `office-*.test.ts` suites;
core-only uninstall, legacy skill recovery and hook cases stay in their core suites.
Shared setup lives in `test/support`, while assertions remain in the scenarios.

The native process suite proves parser, configuration, identity, notes,
response, exchange, talk, installation and skill contracts through the real executable.
Docker E2E supplies private tmux, caller, lifecycle, transport and cross-process
evidence. The Office command and terminal-inspection scenarios are associated with Office
through the component map's `selectedBy` entries; retained-release setup uses the private installer, while
frozen public acquisition refusal is covered by the native Office lifecycle suite. Storage adapter tests prove migrations, transaction rollback,
contention, crash cleanup, retention, acknowledgment and late-final behavior.
Tooling tests prove release-script policy and bounded command wrappers; they do
not count as native runtime or release-archive proof.
The public-install smoke keeps short issue reasons and separate bounded command
diagnostics in its run log and result artifact; the packed runner owns stream capture.

Docker E2E `harness.ts` retains scenario imports; `harness/fixture.ts` owns
fixture resources and process registries.
`harness/readiness.ts` observes caller-supplied events, panes and process state;
`harness/cleanup.ts` stops and checks owned process groups and clients. Unknown
group inspection remains pending within the one-second cleanup bound; unresolved
inspection or surviving groups fail cleanup. The fixture retains cleanup ordering
and error precedence. `harness/types.ts` owns
their suite-local result, event and option shapes; the helpers do not own a second
fixture lifetime.

`typescript/test/tooling/architecture.test.ts` guards literal test import directions:
shared support imports no suite; native and E2E import neither each other nor tooling;
harness helpers do not import the fixture, and root/extension E2E scenarios enter the
root harness through `harness.ts`. Focused tooling tests may import suite-local helpers.
The shared `test/support/source-imports.ts` AST extractor includes no-substitution
literal templates and the first argument of dynamic imports with options; computed
loaders stay outside this static guard. TypeScript module resolution uses the root
compiler options for these test boundaries.

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

`typescript/scripts/merge-queue-metrics.mjs` is read-only developer tooling,
not a CI selector or queue controller. It owns bounded REST evidence collection,
local cache reuse and metric calculation, using the existing bounded command
process owner. Its tests own deterministic API/timeline fixtures; production
job and step evidence stays in local report artifacts. The reporting definitions,
limits and invocation belong to [DEVELOPMENT](DEVELOPMENT.md#merge-queue-metrics).
It never changes workflows, rulesets or PR state; unknown causes/inclusion remain
explicit rather than becoming inferred delivery decisions.

## Release boundary

`dist-workspace.toml`, `scripts/build-native-artifact.sh`,
`scripts/native-cargo.sh`, `typescript/scripts/native-artifact-policy.mjs` and
`typescript/scripts/verify-native-artifact.mjs` are developer/release tooling. The
workflow builds the four supported cargo-dist targets, creates target-filtered
third-party notices (including Vite's bundled frontend inventory for Office), and verifies runtime bytes, linkage, checksums, archive
inventory and executable behavior on matching hosts. CLI runs additionally
verify exact managed-skill contents and the generated bootstrap.

`project-release.mjs` owns release-to-Project delivery evidence, separately from
publication. `project-release.yml` runs on completion of `Native release artifacts`,
daily catch-up and manual replay. All execute on main so the release environment
can mint a short-lived release-App token scoped to org-project writes and tmt
repository reads. Direct release-tag events cannot use that main-only environment. It executes main's tooling, never code
from release tags or artifacts. Automatic runs reconcile the latest ten published
supported releases under one project-wide concurrency group, recovering replaced
pending events. The release-please links or same-product compare range identify
merged PRs; GitHub's paginated `closingIssuesReferences` is the issue authority.
Only existing issue items in pj-tmt organization project 1 are updated: distinct component/version
lines append to `Released in`, then Status becomes terminal `Released`. No issue,
release, membership or earlier status is written. Replays preserve existing text
and skip completed entries; partial text writes can be retried before status.
The full read plan precedes bounded batched mutations and a single readback.
Request/window caps fail visibly with manual replay guidance, never silent
truncation. DEVELOPMENT owns token setup, budgets and dry-run commands.

The release workflow is a product-selected preparation, verification and publication
workflow; publication is authorized by the owner: the standing trunk-based alpha authorization
in the release skill covers the pipeline publishing an alpha draft that passes every gate, and
nothing else. `native-release.yml` is the per-product
run (one queued concurrency group per product) and calls `native-release-bundle.yml`,
the build, assemble, verify and publish pipeline, once per draft release that lacks a verified
bundle or is complete and waits for its publication; the state lives on the draft itself (`release-publication.json` marks a complete
bundle, `verification-failed.json` parks a failed draft), so a replaced or cancelled run
loses nothing and a known-bad commit is not rebuilt. `release.yml` runs release-please
(the CLI pinned in `.github/release-please`, configured by the generated
`release-please-config.json`) on every push to `main`, documentation included: it
opens one release pull request per component, enables auto-merge on at most one (through
required checks and the merge queue), creates the draft release for a merged
one, and starts the per-product run for each product that has a draft without a bundle. A
GitHub App token, created only in that job and only in a live run on `main`, is what lets
the release pull requests run the required checks; the job runs in the `release`
Environment and the App credentials are secrets of that Environment, restricted to `main`.
Until they exist every push is a dry run that opens, merges, creates and starts nothing.
`typescript/scripts/release-please-queue.mjs` owns paginated release-PR discovery and
single-active auto-merge selection under the workflow's existing concurrency group.
The workflow-token pre-check skips `release-pr` while an open release PR is queued,
preserving the candidate and continuing `github-release`. Complete discovery precedes
auto-merge enabling; an existing enabled or queued release blocks another. Otherwise
the oldest eligible same-repository main release PR is enabled with its observed head
SHA as a fence. Multiple already-active releases fail with reconciliation guidance.
The owner does not update BEHIND branches: the queue verifies the merged result against
current main. Query and release-please errors remain failures. Discovery is not atomic
with external enqueues or a later branch update; [Development](DEVELOPMENT.md#queued-release-pull-requests)
owns bounds, token and recovery behavior.
`release-pr-safety.mjs` owns the read-only release PR safety gates. `Code quality`
checks PR notes on PR updates and merge groups: the compare base must be the
component's newest published tag, and each linked commit must descend from that
tag and be an ancestor of the candidate base, excluding the tag itself. It reuses
release version/policy owners and does not regenerate or count expected changelog
entries. Pending squash queue commits have no REST commit/PR associations yet;
the gate resolves their GitHub-appended PR numbers and identifies release branches
before verifying their title/repository/base metadata and current notes in the
cumulative pending range. Ordinary PR metadata mismatches do not fail this gate.
Body/title edits do not restart full PR CI; the merge-group REST read gates the current body.
Missing or inconsistent anchors, notes, queue data and bounded discovery fail visibly.
The same owner checks every manifest component version before release-please:
a visible matching draft without an exact git tag skips `release-pr`, while `github-release`
and build dispatch remain available. Published releases and tagged drafts do not
hold creation. REST reads use explicit workflow credentials and bounded pages;
no release or tag is mutated by either gate.

`release.yml` never publishes. `native-release-upgrade.yml` proves, for a draft or
published release, its upgrade from the last published release of the same product on the
four matching hosts. It only reads releases: a write-token job on `main`'s code fetches the
assets, and read-only jobs run the release commit's scripts on them. When a draft's bundle is
attached the pipeline evaluates the publication gates (channel, commit, immutability,
monotonic, migration, upgrade) in write-token jobs that run `main`'s code and only read the release
commit's data; a failed gate leaves `publication-held.json` on the draft. A draft that
passes them is published by `typescript/scripts/release-publish.mjs` in a write-token job on
`main`'s code (it reads the draft again and refuses a version that is not an alpha, a
component with `release: false`, a draft without the bundle and one with a hold or failure
marker, and the planner leaves the drafts of such a component alone; one `gh release edit` applies the product policy's explicit draft,
prerelease and latest flags), and a job without write access to contents reads the release back: public, immutable,
the policy's flags, the tag on the release commit and GitHub's attestation for the release and
every asset. A failed check opens an issue and fails the run; nothing is rolled back. A read-only `native-release-smoke.yml` then installs the published release as a user does, on the four hosts in an isolated environment: the public installer and `tmt upgrade` for the CLI, the newest published CLI's extension install for an extension; its failures are reported on the same issue by a separate job. CLI, Office and Squad runs share the four-target cargo-dist build and
archive verifier, while keeping product-qualified bundles, independent versions and separate
immutable tags.
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

## Remote extension pilot

`extensions/tmt-remote/rust/tmt-remote` is a local-build-only executable reached
as `tmt remote`. `main` owns style/foreground composition and two bounded
startup calls: capabilities and `storage.root`. `core::CoreClient` owns fixed public `api`/`ls`
subprocesses through the supplied absolute `TMT_EXECUTABLE`; no PATH fallback.
`rust/crates/tmt-invoke` is a TMT-dependency-free leaf owning executable discovery helpers and bounded waited byte captures, deadlines, per-stream caps, cancellation and explicit process-group cleanup; Remote keeps public command choices and error interpretation and depends only on it and the shared `tmt-cli-style` leaf. Request-carried launch options preserve environment inheritance by default or explicitly clear it and copy only named allowlisted caller variables, preserving OS-string bytes and leaving the caller environment unchanged. This policy is configured through the existing invocation entry point; it supplies no memory sandbox or resource-limit guarantee. `LaunchOptions::process_group` defaults to `New`, preserving owned group creation/termination. Explicit `InheritCaller` omits group creation; a started failure detaches and returns `Cleanup::CallerOwned` without signalling or waiting for cleanup. Pre-start failures remain `NotStarted`. The caller must supervise that group. Squad's context wrapper retains its live group-leader check and whole-group abort on a started failure; capture/deadlines/caps remain invoke-owned. Ordinary Squad, Remote and Colab calls use `New`. The shared 20 ms `PULSE` bounds stop-flag observation latency; each wait is also bounded by the remaining request deadline.

`http::Door` is the colab loopback door relocated under remote (#1039). It owns
IPv4-loopback sockets, joined workers, strict HTTP/1.1 framing, exact numeric
Host admission (no alias, so DNS rebinding fails), the origin-form target
grammar that isolates `/r/` from future `/x/<extension>/` prefixes,
header/connection bounds, a door-owned maximum body that handlers can only
narrow, a 32 MiB in-flight body budget reserved before any body byte is read
(bounding unauthenticated memory), absolute acquisition/response deadlines and
shutdown that closes retained sockets before joining workers. It has no
CoreClient/storage reference. A `Handler` admits each framed head (route,
Origin, cookie and body limit) before any body byte is read. `routes::Routes`
is that handler for the machine's stable `/r/<prefix>/` binding routes and the
20-attempt-per-minute unauthenticated budget; `limits` names the binding bounds.
`transport::Transport` moves append/subscribe/ack envelopes to one message
owner, which currently denies every request. Startup discovery is not a remote
operation. The pilot cannot pair, adopt a request, approve, send or subscribe;
no grant, journal or core DB is created. The foreground door has no default
deadline; it runs until interrupted. Colab keeps its own copy of the door
until its routes mount on the remote door.

`state` owns remote's private `<dataRoot>/remote/` subtree, relocated from the
colab keyring: an owned 0700 directory, owned 0600 regular files opened without
following symlinks, a lock-guarded create-only Ed25519 machine key
(`machine.key`, a software file with no hardware claim) and one foreground
serve lock per data root. `store::Store` owns `remote.db` (SQLite) and opens only
with the `state::Serving` proof that the serve lock is held: while serve runs it
is the database's only opener and writer, and every other path (pairing, device
management) reaches remote state only through serve, over the owner-only control socket that pairing adds. Its
schema history uses core's `_migrations` table (append-only, recorded names must
match, a newer history refuses) with `foreign_keys=ON`. Unlike core's shared
WAL database it keeps `journal_mode=DELETE`, since there is no concurrent
reader, and `synchronous=FULL`, so committed grants and receipts survive power
loss. Schema 1 creates the machine identity once: a UUIDv4 machine ID and the
`/r/<32 lowercase hex>` route prefix, both stable across restarts and neither a
credential. Unsafe state fails closed before the door binds. Colab keeps its own
copy of the layout code until a shared leaf exists (#1041).

`site::Site` is the door's handler: `/x/` paths go to `mount::Mounts`, all
others to the `/r/` binding. Mounts forward `/x/<extension>/` to
`<dataRoot>/<extension>/door.sock` only for allowlisted extensions (exactly
`colab` in this slice; a general enabled-extension registry is later work)
and only when that socket and its directory are owned by the user, grant
nothing to group/other and are not symlinks. Remote owns admission: the
door's exact Origin for every request except top-level GET navigation and
for every upgrade, a method allowlist and per-extension request/reply bounds.
It forwards the path below the prefix, a small header allowlist and a
`tmt-mount` header, never the door-session cookie; it adds `tmt-device-context`
(ASCII JSON of the extension channel API's owner device context) only when the
`mount::Sessions` port resolves an owner session, and never copies one from a
client. Until pairing supplies sessions, every forwarded request is non-owner.
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
`browser` with the door's exact loopback origin, and `cli`; the device proposes
no agents, scopes, mode or expiry), the pairing-code text codec (26 base32
symbols, separators limited to ASCII spaces and hyphens) and the four-word key
fingerprint over the pinned BIP-39 English list in
`extensions/tmt-remote/rust/tmt-remote/assets/bip39-english.txt`. `crypto` owns strict Ed25519
verification, full HMAC-SHA256 verification and pure `K_response`/`serverProof`
derivation. Neither module has I/O, clock, storage or CoreClient access, and
neither is wired into the deny-all door. Remote-generated IDs remain UUIDv4.
Byte construction and valid signatures establish no authority.
Rust tests consume the independent Python canonical fixtures read-only; Rust-owned
RFC/Python/WebCrypto vectors exercise cryptographic validity separately. The codec
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
encryption profile are specified. Core never owns a listener or remote state. Official
product/release registration is deferred. Its private component owner excludes
remote versions from real-product releases; cargo-dist excludes this pilot binary.
For shell ownership, see the [browser add-on shell](#browser-add-on-shell).

The private [`remote-client`](extensions/tmt-remote/typescript/remote-client/README.md)
TypeScript module owns decoded-value envelope, device enrollment and possession
signing-byte builders, the `K_response`/`serverProof` HMAC inputs, pairing-code
decoding and fingerprint indexes, with independent exact-byte/SHA-256 fixtures
shared with the Rust tests. Remote-generated IDs remain UUIDv4, as defined by the
channel contract. Syntax validation establishes no authority.
It uses standard UTF-8 and WebCrypto SHA-256 primitives and runs byte conformance in the existing Code
quality job: the independent Python oracle must pass before the workspace-pinned
Vitest suite runs. It implements no wire decoder, signing, key persistence, transport,
runtime authority or browser-shell wiring; the proposed contract remains the wire
and authority definition owner.

## Colab extension proposal

**Status: persistence and foreground deny-all executable and model foundation implemented;
authentication, sync, decoder, browser and backend work remains proposed.** The local-build-only pilot lives under
`extensions/tmt-colab/`. Its [normative colab-v1 contract](extensions/tmt-colab/contracts/colab-v1.md)
owns envelopes, membership, page/epoch state, sync, renderer, enrollment, pairing,
bridge policy and acceptance gates. The #828 design owns product/UI choices;
#829/#830 are bounded spike evidence. The executable is local-build-only; no deployment or official release is registered.
The [channel boundary](extensions/tmt-colab/contracts/colab-v1.md#channel-boundary) marks which colab-v1 sections move to remote, stay or retire.

Current executable dependencies are `tmt-invoke`, `tmt-cli-style` and reviewed
workspace pins. The proposed `tmt-colab-model` integration will own pure values,
canonical bytes/codecs/crypto and policy, without I/O or core access. The
executable owns CLI composition, foreground door, SQLite/files and keyring;
the embedded app and bridge remain proposed. Core access is only through the absolute invoking
`$TMT_EXECUTABLE api` and documented JSON commands via the invoke leaf; no
`tmt-core`, `tmt-adapters`, Office or Remote behavior dependencies, core SQLite
or pane scraping. Shared crypto extraction requires actual consumers and review.

The extension-relative `typescript/colab-client` is a private pnpm member for
client primitives; log verification, Yjs state and SyncBinding are planned additions.
`typescript/app` (trusted React/Vite chrome and renderer) and backend packages
remain separate proposals joining the workspace/lockfile when implemented. Shared
workspace/component edits follow the two-lead rule; architecture guards and
full runtime CI-scope coverage include the persistence library. Runtime
consumers require their own adoption review. The private component
(`release: false`) excludes all colab files from CLI releases; the Rust rule
retains full native and Office coverage. #841 gates yrs adoption. Official
registration/packaging is separate.

`rust/tmt-colab-model` under the extension is the pure Rust foundation: value
syntax, deterministic Ed25519/X25519 public derivation, bounded LP framing,
strict Ed25519, sign-in HMAC/possession, management
bytes, namespace-bound cuts and immutable object codecs/seal/open. Its only OS
operations are crypto-only entropy for internal object IDs and fresh HPKE
ephemeral keys. Long-term key generation stays in the executable keyring; no
filesystem, process or network use. No core or extension behavior
crate depends on it yet; the architecture guard rejects runtime/core dependencies
and unreviewed consumers. Envelope syntax/signature success does not establish
log, session, role, epoch or sequence authority; callers admit those before open.
The model also owns device/chain syntax, purpose-separated link keys and
owner-authenticated HPKE Base wraps. Caller-owned live-issuer/history/transition
policy still gates application. Strict bounded operation payloads and owner
statement hash-chain fencing also belong to the model. Retained heads pin the
revision-1 editor management member and reject successor reuse of its ID/keys;
signing derives revision and previous hash from that head. Verification does not
apply a transition. Shared baseline descriptor admission and exact management
frame decoding stay here; sync/control transport DTOs belong to the server.
The private browser client owns canonical values/LP/JSON, strict Ed25519,
immutable object envelopes and sign-in/management bytes, using WebCrypto only.
Its typed statement/certificate/cut ports mirror the model; fixed-suite HPKE
opening keeps native recipient keys opaque. The caller supplies the pinned URL
root and highest retained head, live issuer/recipient bindings and epoch policy.
Its test-only three-engine harness fails closed on incomplete engines/corpora and
checks ciphertext both ways through a developer-only Rust example. No app or
transport is implemented. Pairing/send/baseline builders remain later L1 work. Frozen vectors are contract-owned;
Rust tests read them without Python. Regeneration uses an independent Python
cryptography oracle; the retained #829 corpus tests all 148 strict policy rows.

All extension state stays in `<core-reported data root>/colab/`, with 0700
directories and 0600 files, separate from core SQLite and provider configuration.
The extension owns its ciphertext database/blobs, keyring, machine grants and
bridge ledger. Colab is an app on remote: the
[remote channel contract](contracts/remote-channel-v1.md#extension-channel-api) owns
owner-device identity, door route mounting, the opaque relay, agent operations and
backends/deploy, and colab-v1 marks which of its sections move there or retire.
Colab keeps page membership, content keys, epochs and before-effect verification;
the public core API retains dispatch/final ownership.

Servers never decode Yjs; foreign-writer decoding/merging runs in a bounded
`tmt-colab` child through `tmt-invoke` (deadline/caps/confirmed cleanup), or a
budgeted browser Worker terminated on overrun, as defined by the contract.
That boundary contains decoder failure, without claiming an OS/key sandbox.
Local acceptance precedes Firestore then Cloudflare; protocol, renderer and
containment details/gates live only in the linked contract. DEVELOPMENT
documents the current local build and foreground run commands.

### Persistence implementation

`extensions/tmt-colab/rust/tmt-colab` is a private, local-build-only library
slice for #847. `keyring::Layout` owns the injected absolute data root's
`colab/` subtree, with owned 0700 directories and no-follow, bounded regular
0600 files. It preserves existing root permissions and touches no core database,
configuration or provider settings. `Keyring` publishes one software owner seed
with create-only, synced file publication; existing invalid keys fail closed.
The temporary space-ID builder follows the contract's domain-framed derivation and will
move to the separately owned L1 model when that API lands.

`store::Store` owns real SQLite ciphertext, durable create-only stream receipts,
conflict freezing and epoch fencing. Namespace checkpoints prune only their
covered prefix and superseded unpinned checkpoint payloads in the same
transaction; receipts and concurrent tails survive. New checkpoint prefixes
advance monotonically; exact retries never republish pruned bytes. The
`pin_checkpoint` seam preserves authority-cut ciphertext for the later verified
owner-log caller.
Per-page capacity returns an error instead of evicting history. Envelope
signatures, identity grammar, roles and owner-transition authorization belong
to the future model/admission caller; this library creates no network authority.
Tests own isolated directories and SQLite oracles. Sync, authentication, decoder and model integration remain later slices.
The executable depends on the reviewed invoke/style leaves and pinned
storage/network/crypto primitives, never core, adapter, Remote or Office crates. Its component is excluded from release;
workspace checks and Docker build contexts include its manifest.

### Foreground composition and loopback door

`main` owns `serve` and read-only `spaces`, style/JSON output, signals and one
foreground service lock. `core` makes one fixed `storage.root` public API call
through the absolute invoking `TMT_EXECUTABLE` and `tmt-invoke`, with deadline,
stream caps and cancellation; missing/invalid roots fail before state creation.
`spaces` creates nothing. `serve` opens the private keyring/store before printing
a working IPv4-loopback placeholder URL. Keyring publication and stale temporary
cleanup share a short-lived private lock; matching files require bounded owned
regular-file admission without following symlinks. Typed state/schema faults
reach machine-readable CLI errors.

`http::Door` has no keyring, store or core reference. It owns finite loopback
sockets and joined workers, exact Host/Origin admission, strict HTTP/1.1 framing,
body/header/connection bounds and absolute read/write deadlines. It serves only
the static placeholder and denies every API/WebSocket upgrade, including forged
cookies. Shutdown closes retained sockets before joining all workers. Accepted
WebSocket framing, sign-in and sessions are parked for relocation into the
Remote application boundary; the merged placeholder/store remain. Tests use real
sockets and isolated CLI processes, with readiness channels and explicit kill/wait guards.

### Isolated Colab decoder

`decoder::Decoder` is one exclusively owned child runner per page. It accepts
only caller-admitted plaintext `decoder::UpdateBatch` values, uses `tmt-invoke` with an empty environment
allowlist, and returns untrusted projection/merged-update bytes for later typed
authority checks and atomic application. It has no door, core, keyring or store
handle. The existing ciphertext server never calls it.
Only `decoder/child.rs` imports pinned yrs 0.28.0 in production, enforced by the
architecture guard. The private `__decoder` entry runs before CLI/data-root
routing. It declares namespace roots, checks materialized types and projection
bounds, and merges only supplied author updates, never the baseline/shared document.
The caller retains live-log/role/operation admission and owns one runner per page.

The [contract limits](extensions/tmt-colab/contracts/colab-v1.md#decoder-isolation-compaction-and-limits)
are enforced before decoding and before returning output. Linux sets the pinned
address-space limit in the child before stdin; setrlimit failure rejects the job. Other platforms, including macOS, run with time/output containment and
report `memory limit unavailable`. This is crash/resource containment with
residual user filesystem authority, not a network/filesystem sandbox.
Invoke owns the process group and cleanup. A runner remains reusable when
`Cleanup::NotStarted` proves no child existed, or after `Cleanup::Confirmed`.
States where a child may survive block that runner. Invalid output,
panic or timeout returns no application result. The archived #830 seeded corpus
and six exact hostile dumps are owned by the decoder tests, which run the corpus
twice with fixture-specific one-second/45-second budgets and positive controls.
