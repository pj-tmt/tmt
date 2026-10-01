# User-built Office platforms, objects and communication

Status: local visual and interaction refinement in progress; not a released feature.
World editing, platform furnishing, scoped delivery and compact browser conversations
and room-scoped discussion, saved-notebook entry points and browser pixel-art
authoring are implemented locally. Earlier functional checks do not constitute
acceptance of the platform finish, starter layout or canvas-first editing experience.
Publishing and user-preview replacement remain separate actions.
The local room CLI, scoped direct delivery, inbox fan-out and listen behavior are
defined in [meeting rooms](meeting-room-v1.md). Local direct chat now projects
durable requests/replies and recovers unconfirmed sends.
This is the consolidated implementation goal. The target is an owner-built,
single-floor, modular Office centered on a Lobby. Fixed-size personal-office cells
extend cardinally; a separate meeting zone extends without repacking the office.
Identity creation never builds a room. The [visual reference index](../../../docs/office/references/rooms-and-walls/README.md)
identifies the current modular design package. Illustrations
are not runtime evidence; this document overrides incidental generated details.
Do not publish or replace the user's running preview as part of this local stage.

## Shared ownership

- Separate world topology, area definitions, personal occupancy, object placement
  and communication membership. Identity creation does not generate floor or rooms.
- Reuse existing state/revision/resource owners; no second world, chat, notes or
  presence database. Remove superseded projection paths rather than maintaining two.
- This work is pre-launch: breaking API, command and module refactors are allowed.
  Do not preserve obsolete block interfaces through compatibility wrappers or
  dual writes merely to avoid changing callers. Update callers, tests and guidance
  together. This does not authorize discarding existing user layouts or resource
  contents; migrate retained data while replacing its old implementation owner.

- One identity may join many rooms. Reuse the existing durable room UUIDs and
  many-to-many membership; personal workspaces are not meeting rooms.
- Promote room membership and scoped delivery into transport-neutral core use
  cases. CLI and authenticated local HTTP are adapters, not separate services.
  Room CLI use must not require installing or starting the optional Office.
- Retain the current request/inbox/reply store and operation receipts. Room
  context belongs to canonical exchanges, not a second message/chat database.
- Platform objects reuse admitted artwork and placed-instance/resource definitions.
  Historical wall mounts are retained data, not a second platform authoring mode.
- The world paints projections and emits object/identity/room IDs. It does not
  infer membership, execute shell strings, scrape terminals or own request state.

## Room semantics and CLI

The first version is communication scoping, not a security boundary.
The installation owner can inspect and manage all local rooms. Do not silently
implement member ACLs or claim that room filters protect secrets.

Use stable room UUIDs. Friendly names may resolve only when an exact name has
one match; duplicate existing names must produce an ambiguity error with IDs,
never route to the first result. Do not introduce a persistent ambient current
room because an agent may participate in several rooms simultaneously.

Room grammar shares the existing typed parser and dispatch owners:

```sh
tmt room create "Design"
tmt room ls
tmt room show <room>
tmt room join <room> --identity Alice
tmt room leave <room> --identity Alice
tmt room rm <room>
tmt ls --room <room>
tmt talk Alice "Review this change" --room <room>
tmt room send <room> "Review this change"
tmt room broadcast <room> "The review is ready"
tmt x listen --identity Alice --room <room>
```

Join/leave omit identity only when normal caller resolution proves it. Membership
edits are atomic set changes; concurrent joins cannot replace an unrelated member.
Joining twice or leaving an absent membership is a no-op. Offline saved agents
remain members; retirement removes them from effective membership, and same-name
replacement does not rejoin. Preserve existing conditional full-roster editing.

The agent Info panel offers Add to meeting. Choose a room and review the existing
roster plus the selected agent in the normal room editor; only Save room writes.
Already-joined and full-room states are explicit. Closing the panel retains the
draft, and switching agent/room cannot replace it without finishing or discarding
it. This entry never assigns a home, moves furniture or dispatches a message.

`ls --room` restricts the existing identity/presence result. A direct `talk` room
option scopes the target and records context; it never silently becomes fan-out.
`room send` explicitly queues one replyable request per effective member and
returns per-recipient receipts. `room broadcast` uses existing no-reply
announcements. Exact audience/revision fences and operation replay retain their
existing behavior; empty rooms fail before any request is written. General
identity routing without a room remains unchanged.

Incoming room filters use stored request context, not the recipient's current
membership. Leaving a room cannot hide previously delivered work or prevent its
reply. Room retirement preserves history and references while excluding the room
from new selection/delivery. Define help, JSON, exit codes and rejection of
incompatible options together in the existing parser/dispatcher owners.

## User-built world and area lifecycle

Start a new layout with a furnished Lobby and four equal personal-office cells,
two above and two below a fixed 2×2-cell Lobby preset. Personal cells start
unassigned; illustrative named robots are not automatically created identities.
The owner extends the layout explicitly and rearranges its contents; the preset
is a starting point, never a recurring reset. Initial reads
must not overwrite existing data or materialize unrelated resources.

The enlarged Lobby is a furnished shared center, not an empty oversized cell:
compose a central sofa-and-table lounge, legible entry signage, discussion,
whiteboard and broadcast stations, with perimeter shelving, plants and warm
lights. Keep cardinal circulation and functional-object discovery clear. All
furnishings remain ordinary editable instances; this preset only initializes a
new world and never overwrites a saved layout.

Use fixed-size module slots along north/south/east/west directions. Add an office
by selecting an eligible adjacent slot, not by painting individual floor tiles
and subsequently zoning a rectangle. The Lobby is the layout origin; its wider
preset occupies four cells, not a variable brush region. Its horizontal and
vertical centerlines align with the gaps between office rows and columns.
Short bridges connect immediate Lobby neighbors. Distant offices use shared
public bridge spines without crossing private interiors; empty neighboring slots
do not generate dangling corridors. The separate meeting wing uses one public
spine with perpendicular branches to occupied pods. Keep bridge widths narrow
and independent of Lobby dimensions. Derive floor, openings and exposed slab
edges from one module arrangement. Platforms and bridges share one floor plane:
no doors, raised thresholds, arches or ramps. Only offer slots
with valid access; additions must not
strand a room, sever circulation or block the meeting-zone connector. Keep one
connected, bounded floor: no diagonal-only attachment, stairs, multi-floor system
or simulated walking/pathfinding in this stage. The former Floor/Erase/Zone
workflow is replaced, not retained as a second editing mode.

Project module occupancy and generated circulation into one floor/area geometry;
derive exposed platform edges and access openings from it. Do not persist a
competing editable floor or wall graph. Rendering, picking and placement
validation share geometry, including clear bridge entrances. Areas must be
non-overlapping, supported by the projected floor, with
usable access verified structurally rather than through agent movement.

Keep one primary Lobby as fallback. A saved identity may occupy at most one
personal area, with at most one resident per personal area; empty offices are
valid. Active unassigned saved identities appear in the Lobby. Offline identities
retain assignments but are not rendered as active workers. Temporary identities
are Contractors: no personal assignment, automatic room or persistent notebook
entitlement. They can use the Lobby, join meetings, communicate and report status.
Promotion preserves UUID and makes assignment eligible, without building a room.
Retirement removes active projections through the existing lifecycle and does
not erase retained content.

The editor provides inspection, module creation/removal, platform finishes and
furniture without a separate edit mode or layout Save/Cancel bar. A selected office's
anchored properties expose its label and optional saved resident. Changing its
purpose through arbitrary zoning is not part of this model. Meeting creation
and membership use their own contextual controls.
There is no Walls tab, wall-height setting or mounting action on platforms.
Furniture and functional stations use the admitted object catalog.
Completed valid gestures and property changes auto-apply through one serialized,
revision-fenced queue. A drag preview does not write; invalid or cancelled drops
change neither durable state nor history. Undo/Redo changes use the same queue
and affect layout only, not messages or independently saved resource contents.
Failed or uncertain writes retain local changes and pause for explicit retry or
reload. Acknowledgements preserve newer edits and session-local history.

The revised editing experience is canvas-first: select modules and their eligible
extension slots directly on the world. Preview the entire added module and its
derived connections before confirming creation. Do not make a persistent HTML inspector or a large
configuration modal the primary creation path. Small anchored HTML controls may
serve text entry, accessible choices, resource selection and necessary safety
confirmation. They must preserve the same layout/history/write owner and not resize
the world. Creation cards must use their measured size and the full hologram
bounds to avoid covering the selected module whenever the viewport permits;
keep the name field focused, location selection expandable, and Escape a
non-saving cancellation. Tight viewports retain scrollable, reachable actions.
Creation previews are cyan holographic slabs with a translucent floor grid,
thin trim and downward front-edge thickness, not tall wireframe boxes.
Show the access connection and keep the preview behind its anchored name form.
The same visual language applies to office expansion and the first/next meeting
slot. The deep-space background must have visibly distinct star brightness and
depth; neither stars nor preview glow may require a continuous idle render loop.
Logical openings remain available to topology and placement validation but have
no portal artwork; their walking surface is flush with adjacent platforms.

Define one revision-aware commit boundary for topology, dependent placements and
personal assignments. Validate the whole candidate state before atomic commit;
stale revisions fail without overwriting concurrent changes. Keep drafts after
failure and do not silently rebase. Preserve existing room/furniture data through
an explicit tested migration or compatibility projection. Missing world-map data
is not permission to erase old layouts.

For existing v2/v3 module layouts, explicit modular conversion creates one undoable
v4 change through the auto-apply queue. The Lobby expands to 2×2 cells; southern office rows and their interior
contents shift one row south. Meeting slots stay fixed. Preserve area IDs,
resident assignments, materials, object IDs and resource attachments. South-wall
Lobby mounts follow the enlarged boundary. Objects without an unambiguous room
interior owner block the preview with their ID; do not guess a new location or
discard them. Native writes perform normal full placement admission and revision checks.
For free-form v1 layouts, the preview reuses existing eligible module slots to
place personal areas near their former positions relative to the primary Lobby.
Meeting areas enter the separate wing in stable spatial order. Terrain shapes
and corridors become generated module geometry; area IDs, names, assignments and
contents remain. Conversion is explicit, not a read-time mutation. Empty areas,
additional Lobbies, ambiguous object ownership and contents too large for their
destination block conversion rather than being discarded. This is not a reset
to the new-install preset and does not create extra identities or offices.

The browser no longer offers free-form floor painting, zoning, new area
designations or manual door editing. Retained layouts can still be inspected,
have objects moved or explicitly removed, and have redundant area designations
removed before conversion. An empty area must be removed explicitly, not silently
dropped. Native write admission remains the authority for valid placements. Conversion and
repair share the whole-world history; no map-only history or alternate write
path is retained.

### Module removal is not identity or resource deletion

Show affected area, occupants and objects before confirmation.

- Removing a personal module explicitly unassigns its resident to Lobby, preserving
  identity, notes, appearance, status and exchanges. There is no intermediate
  unzoned-floor editing step. Preview the affected placements and require them to
  be moved or explicitly removed before removing their supporting module; do not
  create a second inventory or silently lose furniture.
- Removing a meeting area detaches its spatial projection, not its canonical room,
  membership, whiteboard, discussion or requests. A future area may bind the same
  room UUID. Core room retirement remains a separate explicitly named operation.
- Removing a module or its derived partition can
  invalidate furniture, windows or doors. Block invalid changes and identify the
  affected instances until the user explicitly moves/removes their placements.
  Never silently crop, discard, relocate or erase contents.
- Removing a placement deletes only that instance, not artwork packs or linked
  resources, and never activates links.
- The primary Lobby cannot be removed without a valid replacement in the same
  draft. Its furnishings and material style are editable. Reject removal that
  disconnects other modules or their meeting-zone connector; keep a valid
  connected floor. The standard preset keeps its Lobby at the center.
- Cancellation changes nothing durable. Invalid or stale drafts cannot partially
  apply assignments, floor or placements.

Large populations use bounded avatar previews, an accessible searchable roster
and jump-to-agent/area controls. Existing positions do not repack as identities
change. Visible-world rendering and bounded caches support expansion; do not
promise unlimited storage or draw all rooms at once.

## Spatial meeting rooms

The starter's main office contains a Lobby and four personal offices. Meeting
rooms belong to a visibly separate, independently extensible meeting zone, not
one fixed room embedded in that office footprint. Group meeting rooms together
and allow further rooms to extend that zone without consuming personal offices
or repacking the main office. Existing meeting modules retain their slot positions
when another is removed. With no meeting modules, show an in-world Add meeting
room affordance; with existing modules, retain an extension affordance. Clicking
it opens an anchored name field. Cancel creates nothing. Canonical creation and
spatial placement are coordinated explicitly: preserve an accepted room UUID if
placement/save fails and retry without creating another room. Never imply that
layout Undo also undoes separately committed room membership or creation.
Show extension/placement in the world rather than
requiring a large configuration modal. This is owner-driven expansion, not
unbounded storage or automatic identity-count growth.

Meeting areas remain parts of the same canonical world with shared circulation;
visual independence does not create a second world database. Position and extent
come from explicit layout, not room-list order. Bind the canonical room UUID and
display its name; core rooms remain usable without a spatial area. The six-room
concept with one central meeting room is superseded and must not drive the preset.
Offer table, chairs, room whiteboard/discussion and broadcast entry as a reusable
starter preset through existing catalogs/bindings. Applying a preset is an
explicit layout change; merely opening a room creates no resources.

Member avatars are projections of the same identity appearance. Showing Alice in
two rooms does not clone her identity, online state, inbox or private notes.
Online/offline is real identity presence; membership is not physical position.
Both an agent's anchored panel and a meeting's member panel offer membership
changes through the same canonical membership owner. Closing/retiring a room and
removing its spatial module are visibly distinct actions; explain retained
history and affected members before a retirement confirmation.
Empty rooms, large rosters, offline members and unavailable artwork need honest,
bounded views and accessible member lists. No walking or proximity listening.

Room whiteboards use the existing document service with a room-bound identifier.
Room discussions extend the existing board scope and storage; do not copy threads
into a new room board store. Default facilities resolve through registered host
bindings like the lobby. Room removal does not delete shared content implicitly.

Discussion scope is an explicit category: general, repository or canonical room
UUID. A room-bound object opens that room's category; moving or rebinding an area
does not retarget the object. Post/list CLI selection uses `--room <uuid-or-name>`
instead of `--general` or `--repo`, with the same unambiguous room resolver as
other commands. Replies inherit their thread's category. New room threads require
an existing room, not membership; unknown room categories cannot silently fall
back to General. Existing content and exact receipts remain retained after room
removal. Switching board scopes must not lose an unsent or unconfirmed operation.

## Platform materials, backdrop and lights

Selection drives the right-hand inspector: floor selection exposes area controls,
object selection exposes its properties, and no selection exposes furniture.
Undo/Redo and apply feedback stay visible with that layout owner. Directory
and meeting management remain accessible through a collapsed Office menu;
in-world meeting creation and room panels are the primary spatial entry points.
Lobby details do not repeat the global Edit layout action.

### Modular material skins

Use one slab kit for floor surfaces, thin edge trim, downward front fascia and
flush bridge connectors. A module arrangement owns geometry once; its skin
selects surface art and material parameters only. Shared edges have one
deterministic owner, not overlapping room sprites. Rendered bounds, picking,
entrance clearances and native admission derive from that same arrangement.

The default Workshop finish uses teal metal trim, oak and brass.
Moonlight and Copper reuse the same geometry and admitted material choices.
Material changes cannot alter footprints, opening sizes,
assignments, identity UUIDs, room membership or extension bindings. Adjacent
different finishes need a deterministic connector treatment, not
independently conflicting geometry. No executable theme/plugin loader is
introduced by this design.

The platform is a board in space, not a room with shortened walls. Perimeter
trim stays on its top plane; visible front thickness extends down below it.
No rear plaster face, side wall, corner pillar or doorway may be layered over
that plane. Bridges meet interrupted trim without gaps, wooden exterior strips
or doubled frames. Amber guide lights run along bridge edges and never
span an entrance. Furniture and functional stations remain independently
editable across finish changes; boards and whiteboards are freestanding objects.
Names, board contents, live status and action cues are rendered from their
existing owners, never inferred from or baked into decorative pixels.

The [modular visual package](../../../docs/office/references/rooms-and-walls/modular-v1/README.md)
owns the art inventory and visual acceptance details. Generated sheets are source
art, not admitted sprite atlases; verify alpha, extraction, authored scale,
directional views, palette bounds and seam alignment before runtime integration.
No source sheet may bypass the existing data-only custom-art boundary.

### Backdrop and furniture

Use a quiet dark-space backdrop: sparse stars, low-contrast nebula, prominent warm
interiors and readable cream HUD. Avoid continuous star animation or live
lighting/shadow simulation. Background is
presentation, not terrain or authority; changing it never changes floor data.

Show construction feedback with the active selection or gesture, not a separate
edit mode. Use visible-world projection, bounded reusable textures and
invalidation-driven painting. Visible bridge lights share one bounded pulse
clock; hidden, reduced-motion, empty and disposed scenes stop that clock.
A Motion selector in a concept is not a requirement to ship animation or a theme
marketplace. Decorative celestial scenery is not another simulated world.

New presets contain no mounted windows, lamps or wall decorations. Legacy
surface support is a retained-data concern, not a platform customization feature.
An explicit conversion preserves artwork, IDs and linked content as floor
decorations; ambiguous ownership or oversized objects reject the draft without
deleting anything. Explicit conversion uses the same undoable auto-apply path. Native validation
for retained layouts remains in force until their conversion.

## Furniture artwork and actions

Share the platform projection for painting, picking, bounds and editing.
Keep native floor coordinates authoritative. Resource bindings belong to placed
instances, not their art or former mounting surface. Art authoring uses the admitted pack/palette pipeline,
including a bounded browser pixel editor, preview, validation and explicit Save.
Do not create an unrestricted remote-image or HTML/SVG upload path.

An optional link is an explicit data-only action, separate from image pixels and
resource contents. Admit absolute HTTP(S) URLs only; reject credentials, control
characters and executable/file/data schemes. Show destination origin and an
external-link cue before activation. Only a deliberate user click opens a new
tab with no opener; rendering/importing never fetches or follows the link.
Agent-visible descriptions cannot grant tool execution. Link handling joins the
existing guarded interaction path rather than a renderer-owned click handler.

Offer bundled poster, text-sign and external-link objects. Existing whiteboard,
discussion, broadcast and saved-notebook entry points use admitted host bindings,
not shell commands. A notebook view reuses the saved-identity notes owner through
an explicit bounded interface, never arbitrary file access or another notes
database. Contractor objects cannot bypass eligibility. Functional objects use
consistent action cues; decorative text/pixels confer no executable capability.
Keep custom artwork validation and safe missing-art fallback.

## Agent status and mood

Expose one identity-owned typed projection shared by CLI and Office: short
self-reported activity, optional mood, update time and stale/expiry behavior.
The [identity status contract](../../../contracts/identity-status-v1.md) defines its commands,
atomic record and expiry rules. Do not copy status
into rooms or another scene registry. Observed endpoint presence stays separate
and never implies execution. Self-reported availability is not dispatch authority.

Saved and active temporary identities can report status. Omitted identity uses
existing verified caller resolution; non-tmux callers select explicitly. Define
bounded set/show/clear operations in the existing grammar and output contract,
then update help/skills. Do not advertise speculative command syntax as shipped.

Display name, small presence indicator and one short activity/mood bubble; stale
activity must not appear current. Details include update time. Reply-ready
attention takes priority rather than stacking bubbles. Unknown and offline states
remain honest; enqueue acceptance never creates fake typing/working signals.

## Direct communication from Office

Selecting an agent offers Message; a room offers Message room. Use the same
typed submission, result, timeout and error models as CLI adapters. Keep direct
and room destinations explicit, and display the exact fan-out audience before
send. Queue acceptance, read/ack, reply, timeout and unavailable are distinct.
Direct messaging is an ordinary chat input with message/reply bubbles: Send or
Enter submits, Shift+Enter adds a newline, and no second review screen is needed.
The compact agent-anchored HUD has Chat and Info. An absent/offscreen actor uses
a viewport fallback. Minimize submitted interaction to a truthful waiting/reply
cue; reopen on deliberate click. Overlays never resize the world.
Operation receipts remain internal; room fan-out retains explicit audience
confirmation. The local Message room entry uses the shared request composer;
these reference images are not implementation evidence. Overall visual and
end-to-end acceptance remains a separate gate.

The Message room entry opens a room-scoped request composer, not the announcement
station. Read the canonical roster, explicitly adopt it, then review the room,
revision, exact message and recipients before sending. Offline members remain
eligible; absent rooms and empty rosters cannot silently fall back to global agents.
Roster changes require rereading and re-preview, never automatic audience expansion.
Closing retains the room draft. Changing room protects a draft and blocks abandoning
an unresolved send until recovery or explicit discard. Reuse the pending-intent
journal with separate direct/roster scopes; a reload checks the same operation's
receipt rather than dispatching again. Removing the spatial area does not remove
the canonical room, its accepted requests or their replies.

The conversation view projects canonical request/reply records, not mirrored
chat messages. Browser result observation is bounded, cancellable and paused
when hidden or closed; minimizing retains bounded observation. Directory browsing
and layout editing suspend the retained HUD, and overview refresh preserves its
unsent draft. Never infer completion from terminal output or fake typing.
Reload/reopen must recover accepted requests and replies from durable storage.
An uncertain submission retries its original operation, never silently duplicates.
Existing loopback bearer/Origin/input limits remain in force; no generic command
executor is exposed. Remote HTTP/MCP deployment is not part of this local stage.

## Verification

Acceptance must include:

- Default Lobby plus four unassigned offices versus saved layouts; module-slot
  cardinal expansion/connectivity; empty and
  overlapping areas; no automatic growth; save/reload/restart and old-data migration.
- Saved assignment, Contractor rejection, unassigned/offline views, promotion and
  retirement, multiple meeting memberships and UUID-preserving projections.
- Module removal versus resource retention; last-Lobby protection; affected
  furniture and retained-mount handling; undo/cancel; invalid/stale-save atomicity.
- Platform/art/link admission and custom-art fallback; explicit user-click-only
  HTTP(S) links; saved-only notes; no arbitrary file or executable plugin access.
- Status update/expiry, honest presence, reply bubble priority, direct-only sends,
  explicit room audience, exact operation retry and no duplicate requests.
- Real browser send to inbox, real CLI reply, browser result/reopen and durable
  recovery, plus scoped room CLI and real-tmux regression where relevant.
- Keyboard/pointer and narrow screens, long names, crowded Lobby, missing art,
  visible-area culling/cache disposal and no unnecessary idle render loop.

Use isolated native/browser fixtures, deterministic mock agents and local checks,
not paid AI calls. Keep test evidence and per-run logs outside user manuals.
Update existing canonical skills for core room creation/membership, Office layout,
floor-prop authoring, status and passive inbox use; avoid duplicate provider copies.
Remove obsolete visual instructions from the active reference set; preserve any
unique retained functional requirements in their owning contract.

Complete the full consolidated outcome, not just one slice or attractive mockup.
No new remote transport/MCP, cross-office networking, multi-floor construction,
autonomous movement/proximity chat, arbitrary executable plugins or separate
memory/search engine belongs to this local scope. Do not replace the user's
running preview, commit, push, release or reinstall globally under this design task.
