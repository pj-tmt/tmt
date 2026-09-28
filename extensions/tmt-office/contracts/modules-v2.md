# Versioned modular topology

Status: native projection, strict codec/storage, browser rendering and
source-preserving edits are implemented locally. New installations use v8
platforms. Older versions retain their geometry until explicit draft conversion.
Module removal uses placement-impact review and the native Save gate. Retained
editing tools support conversion-blocker repair. This is not visual sign-off.

The [rooms and walls contract](rooms-and-walls.md) owns the product behavior.
This document owns the versioned module map value and its deterministic topology.
[Literal vectors](modules-v2-vectors.json) define shared metrics, starter bounds
and independently specified passage samples for native/browser conformance.

## Authoritative value

The existing world envelope keeps its ordered objects and revisioned commit.
Only its `map` value changes shape:

```json
{
  "version": 4,
  "primaryLobbyId": "10000000-0000-4000-8000-000000000001",
  "modules": [
    {
      "area": {
        "id": "10000000-0000-4000-8000-000000000001",
        "name": "Lobby",
        "binding": { "type": "lobby" }
      },
      "slot": { "type": "lobby" },
      "material": "workshop"
    }
  ]
}
```

Each module owns one area, one slot and one material. There are no editable
`floor`, `doors` or duplicate `areas` arrays in this value; extra fields fail
admission. The existing area binding rules still apply: canonical UUIDs, one
personal assignment per saved identity, one spatial area per canonical room,
and explicit `identityId: null` for an unassigned personal office. Identity
eligibility and room state are checked by the existing commit owner.

Versions 2–7 use the following slot/binding pairs; v8 generalizes the grid slot
as described below.

| Slot      | Binding    | Fields                                               |
| --------- | ---------- | ---------------------------------------------------- |
| `lobby`   | `lobby`    | No coordinates; exactly one primary Lobby.           |
| `office`  | `personal` | Signed integer `column` and `row`.                   |
| `meeting` | `meeting`  | Nonnegative integer `index` in the independent wing. |

Materials are `workshop`, `moonlight` and `copper`. Changing material does not
change geometry, UUIDs, assignments or resource bindings. The renderer's material
implementations are presentation only; they cannot change topology or admission.

## Projection

Current module metrics are 48×40 floor units, an 8-unit passage, and grid steps
of 56×48. Version 4 Lobby occupies columns 0–1 and rows 0–1, including their
intervening gaps (104×88). Starter offices are `(0,-1)`, `(1,-1)`, `(0,2)`,
`(1,2)`. The horizontal center corridor is y=40..48; its vertical counterpart
is x=48..56. Both continue outside the Lobby independently of paired rooms.

The public lattice contains the grid gaps inside the main modules' bounding
rectangle, excluding the Lobby's owned interior and the reserved meeting strip.
An unoccupied cell remains empty space, not floor or an implicit private area.
The lattice does not extend indefinitely or depend on passing through private
rooms. Each room gets a centered opening wherever its edge meets public floor;
the Lobby entrances lie on its centerlines. Projection works on bounded row runs
and enforces floor/span budgets while expanding the lattice. Module overlap,
whole-map occupancy and reachability retain the existing native admission owner.
[Central-grid vectors](modules-central-grid-vectors.json) lock the new dimensions,
roomless corridor samples, central entrances and sparse meeting-wing behavior.

Version 3 derives full-face lanes between adjacent rooms and their junctions;
version 2 retains
its original isolated short links so existing saved placements remain readable.
The five-module starter has 14,144 physical floor units in version 3 versus
12,224 in version 2, with identical room extents and centered door openings.
Native common-floor connectivity tests exclude private rooms from traversal.
Topology parity is not visual acceptance: the rendered lanes must remain visible
behind cutaway walls, with picking and placement using the same projection.

Versions 2 and 3 retain a 104×40 Lobby and their original room coordinates.
Version 4 is not a silent reinterpretation of either saved representation.

The meeting wing starts at x=120. Meeting index `i` has y=`i*48`; the common
spine occupies x=112..120 and connects to Lobby. The nonnegative-y strip
x=112..168 is reserved from personal-office expansion so later meetings cannot
displace offices or lose access. Eligible-slot UI must expose that exclusion,
not offer a placement that only fails on Save. Main expansion elsewhere remains
subject to ordinary adjacency, collision and coordinate budgets.
In version 4 the Lobby connector is centered at y=44; the spine reaches both
the first and last occupied room's existing centered doorway. Meeting indices
and coordinates do not move when the Lobby height changes.

Removing an earlier meeting leaves later indices and their extents unchanged.
The common spine crosses empty indices; it is derived circulation, not a new
meeting or resource. Removing the last spatial meeting removes its generated
spine, not canonical room data. Placement support and content preservation remain
the whole-world commit's responsibility.

The V4 creation entry offers the next index after the last occupied meeting slot,
not a repacked or list-sorted position. Its cyan room and new public floor are
disposable previews derived from the same bounds and wing projection. Explicit
name submission saves a canonical room first, then adds its furnished module to
the existing world draft. Layout Undo/Cancel retains the canonical room; an
existing unplaced room can be attached without creating or editing a roster.
Placement retries retain the confirmed room UUID and area ID. An uncertain room
save offers explicit readback and review before adopting a stored room. Neither
retry path invents a replacement UUID or automatically overwrites newer state.

The editor previews removal before changing its draft. It protects the central
Lobby and lists every retained object lacking spatial support in the candidate,
including objects on disappearing common floor or mounted on removed partitions.
Move or explicitly remove those placements first. Empty-module removal keeps
all other source slots and object order; occupants fall back through the existing
population projection. Canonical identities, room membership and linked resources
are not deletion targets. Undo and Cancel retain their normal meaning. Native Save
remains authoritative for remaining-room connectivity and full layout validity.

## Ownership and migration boundary

`tmt-office-model::office_map::modules` is the pure projection owner. An admitted
`OfficeMap` retains the immutable module source and its derived geometry.
`tmt-office-model::codec::office_map` retains source version 2 through 8, never the derivative
as another editable input. The world codec and storage path reuse this map codec.

Browser `world-map/map-source` decodes the source union and supplies a cached,
read-only geometry projection to rendering, population and object discovery.
`module-geometry` follows the shared literal vectors; decoding rejects module
geometry that cannot be rendered, while native Save remains authoritative for
connectivity, placements and resource admission. The existing world history and
revisioned Save retain source modules and ordered objects together. Material
changes count as draft changes even when their geometry is identical. Modular
drafts cannot replace their derived floor with independently authored geometry.

Version-1 maps remain readable and retain their original representation during
the transition. Reads do not create modules, reposition objects or write storage.
The browser offers explicit conversion that preserves placements and resources,
or rejects conversion without changes. Freeform terrain authoring is retired;
object edits and area removal remain available to repair retained input.
**Preview modular layout** creates a single v4 world draft from an older source;
it does not change saved data until the ordinary revisioned Save succeeds.
[Rooms and walls](rooms-and-walls.md) owns placement policy and conversion blockers.
Undo, Redo and Cancel use the existing history. Native Save validates the complete
candidate, including windows whose old exterior support changed. Unsupported
placements remain explicit errors, never silently removed or admitted by relaxed
validation. The map value contains only module sources after conversion; derived
floor and doors are not persisted alongside them.

## Version 5: compact circulation

Version 5 preserves the v4 104 × 88 central Lobby and 48 × 40 office cells.
Saved v5 worlds retain this layout until explicit conversion.
Only occupied offices generate public branches: an eight-unit horizontal route
from the room's entrance to the central vertical spine, then a spine connection
to the Lobby centerline. Routes are merged and clipped against the Lobby and
reserved meeting wing before ordinary topology admission. Empty intermediate
rows do not gain horizontal spurs. Removal derives the remaining routes afresh.

Meeting slots use a 40-unit vertical step rather than 48, so consecutive rooms
touch. Slot indices remain stable: deleting a room does not move other rooms.
The common meeting spine still reaches every occupied slot. Canonical room
membership, history and resource bindings are independent of physical spacing.

**Preview compact layout** explicitly converts v4 into one v5 world draft using
the existing placement relocation owner. Room-owned objects retain their IDs,
art and resource bindings; wall mounts follow the corresponding room edge.
Ambiguous support or out-of-room objects reject the conversion without mutation.
The ordinary Save gate checks all new wall support and connectivity; Undo and
Cancel restore the previous source. Reading v4 never applies v5 dimensions.

## Version 6: cosmic platforms

The approved visual model is an open floating floor slab, not a cutaway room.
All v6 floor coordinates use one projection without an upright wall reserve.
Exposed edges have a thin metal rim and a downward front fascia; entrances and
skybridges remain flush with the floor. There are no visible walls, doors, wall
styles or new wall-mounted decorations in the platform editor. The logical map
boundaries still own support and public-route connectivity; they are not rendered
as architectural walls. Construction ghosts show the floor slab, not a tall box.

Existing mounted content requires an explicit draft conversion before removal
of its historical admission path. Functional object IDs and resource bindings
must survive; conversion must not delete a board, whiteboard or broadcaster.
The explicit conversion uses the same retained-layout admission and history path.

V6 retains the central Lobby and office bounds. Every immediately adjacent
cardinal pair has one eight-unit-wide bridge centered in its shared side overlap,
with an opening at each endpoint. Platforms may be traversed to reach subsequent
offices. No perimeter bypass, diagonal link or bridge across an empty slot is
generated. Pair ordering is deterministic. Removing a connecting platform is
rejected by native admission if it disconnects the remaining world; the renderer
must not invent a replacement route. This pre-release correction replaces the
earlier v6 perimeter routing; existing v4/v5 geometry is unchanged.

Meeting pods begin at x=136 and return to a 48-unit vertical step. The public
spine stays at x=112..120; a 16-unit horizontal branch connects each occupied
pod's west doorway. Only one Lobby connector is generated. The reserved wing
extends through x=184. Missing indices keep spine access but add no branch.
[Skybridge vectors](modules-skybridge-vectors.json) supply independent samples
shared by native and browser tests.

**Convert to platforms** uses the existing relocation owner and draft history.
Room-owned contents move with their room and keep IDs, order and bindings;
ambiguous corridor objects block conversion. Save remains the only durable
transition. Mounted objects become floor decorations without replacing their
IDs or linked resources. This conversion is not a visual sign-off.

## Version 7: independent meeting platforms

V7 preserves v6 room bounds, materials, personal Office/Lobby connections and
platform artwork. Meeting index `i` still occupies `(136, i*48, 48, 40)`;
there is no meeting spine, Lobby connector, meeting branch or meeting opening.
The reserved wing remains excluded from personal-office expansion. Removing
a meeting does not repack survivors. Expansion offers the next index after the
highest occupied meeting slot, and its wireframe has no passage preview.
The reserved wing projects these slots with fixed visible gaps, including vacant
indices; see [rendering ownership](../../../docs/office/architecture.md).
[Island vectors](modules-island-vectors.json) specify independent native/browser
floor samples, dimensions and budgets.

Only this versioned module projection admits independent meeting components.
Each meeting area must still be internally connected. All personal offices and
common floor must remain connected and accessible from the primary Lobby;
freeform maps and versions 2–6 retain their original connectivity rules. This
exception grants no access, membership or resource authority.

Retained v7 worlds remain readable but are no longer a conversion target. Reading
a saved world never removes floor or moves content. The explicit v8 alignment
below supersedes the island preview; physical bases, not artwork envelopes,
determine ownership. Removing a spatial module never deletes a canonical room or
resource.

## Version 8: unified area uses

The primary Lobby remains `(0, 0, 104, 88)`. Every other area occupies
`{type: "office", column, row}` at `(column*56, row*48, 48, 40)` and can bind
either `personal` or `meeting`. The retained wire tag identifies a grid slot,
not the area's use. Legacy `meeting` slots are invalid in v8, and the reserved
wing restriction no longer applies. Overlap, coordinate and budget limits remain.

Every adjacent cardinal pair gets one centered eight-unit-wide bridge; missing
neighbors generate no bypass, spine or branch. Separate platform components are
valid for either use. The private modular admission still verifies internal area
connectivity and accessibility of all generated common floor. Freeform admission
is unchanged. [Unified vectors](modules-unified-vectors.json) are shared by native
and browser tests. Slots, not binding types, own bounds and openings.

**Use unified areas** is an explicit, undoable alignment: legacy meeting index `i`
becomes grid column `2`, row `i`; other slots are unchanged. The shared relocation
owner retains furniture order, IDs, bases and resource attachments. Physical
support, including shallow bases beneath overhanging art, determines ownership.
Ambiguous corridor content blocks conversion without dropping any object. The
complete candidate must pass native admission before its durable acknowledgement.

After alignment, changing an area's use edits only its binding. The primary Lobby
cannot change use. Selecting Meeting room links a saved canonical room; room
creation and layout attachment keep their existing separate acknowledgement and
retry owners. Switching to Office clears the area assignment, not the former
canonical room or members. Undo restores the former binding. Furniture and linked
whiteboard/discussion resources are not automatically replaced or retargeted.

V8 display spacing uses a fixed lattice of 72 horizontal and 64 vertical units
before depth projection. The Lobby covers two cells, including their internal
bands. One-cell platform interiors stay 48 by 40; empty slots reserve the same
display space. Changing occupied rows, columns or use cannot shift survivors.
Meeting use adds violet inset lamps and a nameplate icon; the teal mechanical
housing, floor finishes and mint selection perimeter are shared.
