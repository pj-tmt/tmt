# Local Office map v1

Status: local foundation, not an exposed persistence API or released editor.
This describes the currently implemented sparse-floor value, not the replacement
modular-cell editor. The newer [modular product target](rooms-and-walls.md#user-built-world-and-area-lifecycle)
replaces freeform floor painting and zoning. Its implementation must keep one
authoritative topology representation and explicitly migrate retained layouts;
do not add independently mutable module and floor stores.
The native [module-only v2 value](modules-v2.md) now reuses this topology admission;
browser cutover and retained-layout conversion remain pending.
The [user-built Office goal](rooms-and-walls.md) owns product behavior and visual
references. This document owns the topology value boundary only. The
[whole-world contract](world-v1.md) owns dependent objects, migration and revisioned
storage. UI integration must be complete before replacing the existing projection.

## Value ownership

The document contains exactly `version: 1`, `primaryLobbyId`, `areas`, `floor` and
`doors`. All IDs are canonical lowercase non-nil UUIDs. It contains no identity
copy, room roster, message, artwork catalog, furniture or persistent wall graph.

- Areas have `id`, `name` and `binding`. Binding is exactly `{type: "lobby"}`,
  `{type: "personal", identityId: UUID | null}` or `{type: "meeting", roomId: UUID}`.
  An unassigned personal office is valid. The name is nonblank, at most 80 UTF-8
  bytes and contains no control, line-separator or bidirectional override characters.
- Floor is an array of `{y, start, end, areaId}` row runs, covering integer cells
  from `start` inclusive to `end` exclusive. Explicit `areaId: null` means common
  circulation; absence from floor means outside. Neither nullable field may be
  omitted. Runs must not overlap. Native encoding sorts and merges adjacent runs
  with the same row and area. Input order is not object paint order.
- Doors are `{x, y, axis}` lattice edges. `horizontal` extends east from the
  corner; `vertical` extends south. Adjacent cells use the same edge spelling.
  Exterior walls follow occupied-floor edges; partitions follow changes in area
  ownership, including transitions to common circulation. A door opens one
  partition, never the exterior hull or an edge within the same area.

Native save admission requires a primary Lobby, nonempty cardinally connected
areas, one connected floor and structural reachability from the primary Lobby
through openings. Personal identity assignments and spatial room bindings are
unique within a map. Assignment eligibility and canonical room existence are
transaction-time repository checks, **not** established by UUID shape. Temporary
identities must not gain personal workspaces through a map write.

## Admission and projection

`tmt-office-model::office_map` owns native topology validation and derived geometry.
`tmt-office-model::codec::office_map` owns strict versioned JSON; unknown fields, duplicate
JSON members and invalid number types reject. Whole JSON number values such as
`1`, `1.0` and `1e0` are equivalent, using the same narrow numeric decoder as
whiteboard documents; fractions and out-of-range values reject. The document has a 2 MiB
byte ceiling. Work is bounded to 262,144 occupied tiles, 16,384 row runs, 256 areas
and 4,096 doors. Floor cells range from -4,096 inclusive to 4,096 exclusive on
each axis; boundary edges can sit at the upper boundary. These are admission
safety budgets, not a promise that every full-capacity scene renders cheaply.
Sparse indexing must not allocate the potentially 8,192-square bounding rectangle.

Browser `world-map/map-contract` admits shape and work bounds;
`map-geometry` derives disposable preview edges, bounds and hit lookup. It does
not authorize Save. Disconnected floor, a temporarily absent primary Lobby or
invalidated door may remain visible while editing. Overlapping runs reject
instead of choosing an owner by array order. Invalid exterior door candidates do
not create rendered hull openings. Native admission remains the commit authority.

[`map-v1-vectors.json`](map-v1-vectors.json) contains literal native admission and
shared projection examples and exact limit-parity assertions. Native tests exercise every outcome; browser tests
exercise valid geometry, strict shape rejection and intentional draft-preview
differences. Raw duplicate-member rejection belongs to native byte decoding,
before a JavaScript object decoder can lose that information.

## Draft behavior

`world-map/map-draft` changes topology only: paint adds common floor without
overwriting assignments, erase removes only touched cells, assignment marks
existing floor without filling holes. Area removal converts its floor to common
space and detaches its binding. Removing the primary Lobby requires an explicit
other Lobby replacement. Neither operation silently removes dependent doors or
resources; whole-candidate validation must report unresolved placements at Save.

`editor/snapshot-history` is shared by map drafts and whiteboard edits. Each
domain supplies its own owned-value decoder. One completed gesture creates one
undo step; no-ops preserve redo, new branches discard redo, and retained snapshots
are bounded by 64 entries and 16 MiB of serialized content. The current value is
retained. This utility owns no revisions, transport, resources or persistence.
World transactions capture topology and dependent placements together; the
production editor must likewise retain them in one draft/history. A topology-only
history is not permission to save geometry separately from objects.
