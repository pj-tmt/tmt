# Local Office world layout v1

Status: native value, SQLite and authenticated HTTP with a production whole-world
browser editor and renderer. Full wall tools, CLI cutover and visual/native E2E
acceptance remain in progress; this is not a release-completion claim.
This is the current world envelope. Module slots and platform finishes belong to
the versioned map inside it, not additional fields on this envelope.
The [user-built Office goal](rooms-and-walls.md) owns behavior and visual references.
The [map contract](map-v1.md) owns topology. This envelope combines that map with
ordered placements so persistence can validate and commit one candidate world.
Pre-launch callers may change; an obsolete block API is not a compatibility goal.
Existing layout and resource data must still be preserved through the cutover.

## Document

The exact envelope is `{version: 1, map: MapDocument, objects: WorldObject[]}`.
Native admission permits at most 4 MiB of JSON and 4,096 ordered objects. It
reuses map admission and the existing prop placement codec, not parallel copies.
Unknown/duplicate members, unsupported versions and invalid numeric values reject.
Whole JSON numbers have browser-compatible value semantics.

Each object contains exactly:

- `id`: a canonical lowercase non-nil UUID, unique within the world. This is a
  placement identity, independent of the artwork reference or linked resource.
- `kind`: `decoration`, `window` or `wallLight`.
- `placement`: existing immutable `prop` reference, `footprint: {width, height}`,
  signed integer `x`/`y`, quarter-turn `rotation`, and optional existing
  `customization`. Artwork, footprint and customization admission reuse the prop
  owner. Catalog existence and permitted customization remain repository checks.
- `surface`: `{type: "floor"}` with optional `base: {x, y, width, height}`, or exactly
  `{type: "wall", axis: "horizontal" | "vertical", face: "positive" | "negative", elevation}`.
- `extension`: explicit `null` for decoration, or `{definition, binding}` using
  the existing [extension resource bindings](extension-v1.md). It stores no
  content, grants, executable code or dispatch instructions. Missing definitions
  remain inert references; a known definition cannot bind another resource kind.

For floor objects, placement x/y is the artwork envelope's upper-left lattice
cell. Without a base, that entire envelope is occupied as before. An explicit
base is a positive-sized integer rectangle inside the unrotated envelope;
its nonnegative x/y offsets and dimensions must fit the placement footprint.
Quarter turns rotate this rectangle with the placement. Unknown/duplicate base
fields, null, fractional values and out-of-envelope rectangles reject.
The base controls physical floor support, not art size, culling or picking.
Complete visible artwork remains clickable and can overhang a supported base.
Stored object order remains paint and frontmost-pick order.

The base belongs to the floor surface, not to immutable raster content. Bundled
authoring recipes supply it during explicit placement, movement or rotation on
platform maps (v6 and later); earlier map projections do not automatically adopt
these recipes. Existing reads never add one. Custom bases are retained. Native and browser
validation share the same bounds and rotation semantics. It persists with the
object even if its art is missing, and changes atomically with placement through
the existing world revision/history owner. No prop pack bytes or digests change.

For wall
objects, x/y is the starting lattice edge corner, using the map's edge spelling.
The rotated footprint width extends east on horizontal walls and south on vertical
walls; height occupies wall-local elevation units. Positive faces are south/east
of their edge, negative faces north/west. The selected face must have indoor floor.
Wall height is 16 local units, independent of screen scale or apparent wall thickness.

## Whole-candidate validity

- Floor props require every base tile (the whole footprint when base is absent),
  to exist. They cannot straddle partitions or occupy either cell beside a door.
  Overlap between ordinary furniture remains intentional; paint order is retained.
- Wall props require a continuous, closed boundary on the selected indoor face,
  with their entire elevation inside wall height. They cannot cover door openings.
- Windows require exterior walls. Windows and wall lights require wall placement.
  Windows cannot overlap other wall objects; validation reports both affected
  placements. Other wall artwork may layer. Window conflict work is bounded by
  occupied wall cells, not all-pairs comparisons.
- Invalid candidates return affected object IDs and reasons. No invalid furniture
  is filtered, cropped, relocated or silently removed. The previous world stays
  unchanged. Removing a placement does not remove its artwork or resource.

`tmt-office-model::office_world` owns this policy using `office_map::Geometry` and existing
prop validation. `tmt-office-model::codec::office_world` owns the strict composed JSON boundary.
Existing block readers/writers still serve old callers during local development;
they must be removed at the caller cutover, not retained as dual writers.
Web-link attachments use the existing extension binding, admission and guarded
destination review; they create no resource records. The built-in wall collection
adds ordinary window, lamp, poster, sign and link-plaque placements. Browser
creation suggests a supported wall in the selected area without moving existing
objects. Suggestions do not overlap existing mounted silhouettes; manual
edits remain subject to the native admission above. Window/light art resolves
through the same prop catalog as decoration; the light's glow is static.
Coordinate input is applied as one draft edit, and signs use admitted prop text
capabilities. Complete CLI cutover and wall-face visual acceptance remain required
before release acceptance.
The production browser edits the whole world.

`office_extension::ResourceBinding` owns native resource-reference validity.
The extension adapter reuses it for authoring preflight and world attachments;
`world-map/world-contract` reuses browser appearance and extension decoders.
`world-draft` keeps topology, assignments, surfaces and attachments in the same
bounded history. Erasing floor retains invalidated objects until the user handles
them; native Save admission remains authoritative. Literal
[world vectors](world-v1-vectors.json) distinguish editable drafts from committed
admission and preserve the shared work limits.

## Atomic storage and migration

`storage/office_world` owns the installation's existing world row. Schema 28 adds
one layout revision, complete JSON document and update timestamp; it introduces
no second world or resource store. `show_local_world` is a consistent, read-only
snapshot. `apply_local_world` admits the entire candidate in one immediate SQLite
transaction. The expected revision must match; stale requests fail even when their
payload equals the current value. At the current revision, an unchanged candidate
preserves both revision and timestamp. Callers must read back an uncertain Save
before deciding whether to retry, rather than silently rebasing it.

Personal assignments require an active saved UUID; an unchanged retired saved
assignment may remain attached to its existing area without admitting a new one.
Meeting bindings require an existing canonical room. Artwork resolution reuses
the local prop catalog owner. Lost artwork may remain or move under its original
placement ID, but cannot be copied under a new ID or receive forged customization.

Before the first Save, retained block rows project into deterministic connected
areas, preserving object order, appearance and relative placement. Temporary and
retired identities leave unassigned retained areas; identities without stored
blocks create no rooms. An explicitly empty Lobby furniture override stays empty.
The three previously bundled functional Lobby entries become ordinary stable-ID
placements, preserving their bindings to the existing board, lobby whiteboard
and announcement composer. They occupy separate reserved floor, not the retained
furniture footprint. After first Save, reads never reseed removed objects.
Compact legacy
tokens and versioned placements share the existing decoder. Invalid or oversized
legacy layouts fail without dropping records.
The area count is not a guarantee that every legacy inventory fits: padded rooms
and connecting floor also consume the map's total occupied-tile budget. An
over-budget conversion remains an explicit error with its original rows intact.

The unsaved snapshot includes a source fingerprint (`legacy_basis`), checked
alongside revision zero to fence intervening legacy edits. The first Save writes
the complete world and removes superseded block rows in the same transaction.
Schema triggers forbid subsequent block writes. Neither database upgrade nor
preview performs this cutover. Failures roll back the world and old rows together;
identities, meeting memberships, artwork and resource contents are untouched.

## Local transport

`GET /api/v1/local/world` returns `{worldId, revision, legacyBasis, layout,
updatedAtMs, changed}`. Revision zero has a SHA-256 source basis and timestamp zero;
the world UUID can be null before the installation creates its singleton. Stored
worlds have a UUID, positive revision/timestamp and null basis. Reads set changed
false and do not create content or dispatch requests.

`PUT` at the same exact path takes `{expectedRevision, legacyBasis, layout}` and
returns the same snapshot shape. It uses the existing browser bearer, exact
loopback Origin and JSON content type. Only that exact PUT route receives the
4 MiB + 512 byte envelope budget. Typed decoding preserves duplicate-member
rejection inside the nested map, appearance and attachment; no intermediate
untyped JSON conversion can erase duplicates. Unsupported methods, paths and
invalid payloads are rejected before opening storage.

Stale saves return HTTP 409 `WORLD_REVISION_CONFLICT`. Ineligible assignments,
missing rooms/artwork, exhausted revisions and unconvertible retained layouts use
distinct `WORLD_*` conflict codes. Corrupt stored state and storage failures are
server errors, not empty worlds. Invalid candidate geometry returns HTTP 400
`WORLD_INVALID`, a bounded explanation and affected `{objectId, reason}` entries;
an invalid identifier is reported as null, never reflected as arbitrary text.
The browser port retains these diagnostics, applies bounded reads/cancellation,
checks the returned revision and canonical layout, and never retries, rebases,
acknowledges inbox items or dispatches work automatically.
