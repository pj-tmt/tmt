# Local block layout v2

Status: implemented in the #238 source branch; installed availability still
requires a coordinated verified release. It does not change the immutable remote
[block v1](block-v1.md) contract or Firestore Rules. Version 2 extends the existing local block owner with immutable prop
references while preserving its 32×32 room, ordered paint model, 16-object limit,
revision semantics, identity ownership, and one storage row per block.

## One layout and renderer owner

There is no custom-prop placement table or second scene. `office_local_blocks`
continues to store the complete ordered layout. The pure block domain decodes
legacy v1 built-in input or stored tokens into one v2 `PropPlacement` model.
Every genuinely changed local write stores canonical v2; local writes never
downgrade storage to v1. A logical current no-op or exact retry against a stored
v1 row does not rewrite its bytes or timestamps or advance its revision. A local
read always projects canonical v2 without rewriting the stored row or advancing
its revision. Remote block operations and Rules remain v1 and reject v2 until a
separately approved remote contract exists.

Browser and native projections consume shared literal vectors. The browser
resolves every placement through one catalog snapshot and renders every resolved
prop through the indexed-pixel renderer defined by
[prop pack v1](prop-pack-v1.md). The hard-coded v1 Sprite renderer is removed
after the repository-owned built-in pack passes visual review. Retaining the v1
decoder is compatibility support, not a second scene or rendering pipeline.

## Apply document

The exact v2 `.layout` value is directly reusable as an apply file:

```json
{
  "version": 2,
  "objects": [
    {
      "prop": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/reading-lamp",
      "footprint": { "width": 2, "height": 2 },
      "x": 4,
      "y": 6,
      "rotation": 1
    }
  ]
}
```

The envelope and every placement reject duplicate or unknown fields. `objects`
is an ordered list of at most 16 entries. `prop` is the exact immutable reference
from an admitted [v1](prop-pack-v1.md) or [v2](prop-pack-v2.md) prop pack.
`footprint` is the referenced prop's unrotated footprint,
copied into the placement so a missing, removed, or corrupt pack cannot erase
its geometry. Width and height are integers from 1 through 16; v1 prop definitions
retain their original maximum of 8. `x` and `y` are
integer tile origins from zero. `rotation` is an integer from zero through three;
odd rotations swap footprint width and height. The complete rotated footprint
must fit in the 32×32 block. Overlap and list-order painting remain intentional.

The complete readable layout file remains limited to 64 KiB. SQLite stores the
strict v2 JSON value rather than adding a second placement store or short token
codec. Existing local v1 apply files containing `{objects:[{asset,x,y,rotation}]}`
remain accepted at the command boundary and are normalized to v2 before the
conditional write. An apply file never accepts resolution or catalog metadata.

## Canonical local show and mutation output

Local show always returns this exact envelope, regardless of stored v1 or v2:

```json
{
  "identityId": "11111111-1111-4111-8111-111111111111",
  "identityName": "Alice",
  "exists": true,
  "blockId": "22222222-2222-4222-8222-222222222222",
  "revision": 3,
  "updatedAtMs": 1234567890,
  "layout": { "version": 2, "objects": [] },
  "resolutions": []
}
```

`resolutions` has exactly one entry for each same-index layout object:

```json
{ "index": 0, "status": "available", "label": "Reading lamp" }
```

or:

```json
{ "index": 0, "status": "unavailable", "digestPrefix": "0123456789ab" }
```

The fixed 12-hex prefix is presentation only and cannot select content. Available
labels are untrusted display text. Resolution never appears inside `.layout`, so
the exact returned `.layout` can be written to a file and applied without
field-stripping. A successful apply returns the same canonical envelope plus
transactional `changed`. The canonical local output change must be documented in
the Office skill and command guide and covered by native compatibility tests.

For a missing block, `exists:false`, `blockId:null`, `revision:0`,
`updatedAtMs:0`, empty canonical v2 `.layout`, and empty `resolutions` are exact.

## Resolution and placement admission

Structural decode validates reference syntax, retained footprint, rotation, and
room bounds without consulting a catalog. One bounded catalog snapshot then
resolves each distinct digest at most once. Resolution yields exactly:

- `available`: embedded or installed bytes pass current validation, the key
  exists, and the retained footprint equals the immutable prop definition;
- `unavailable`: the digest/key is absent, removed, corrupt, or footprint-mismatched.

Unavailable placements remain in their original list position and retain their
exact reference, footprint, coordinates, and rotation. The scene draws one
repository-controlled bounded placeholder inside that rotated footprint with an
accessible label containing only `Unavailable prop` and the fixed digest prefix.
It never renders unvalidated labels or pixels, fetches a URL, substitutes a
similarly named prop, changes block revision, or drops neighboring objects.

A layout mutation may introduce a reference only when it is available at the
catalog state observed inside the existing serialized block transaction and its
footprint matches. Unavailable occurrences already present in the saved revision
may be retained byte-for-byte, reordered, or removed while other objects are
edited. They cannot be moved, rotated, resized, or duplicated.

This rule is multiset-aware: for every exact canonical unavailable placement,
the proposed layout count must be less than or equal to its count in the current
saved layout. Each retained occurrence consumes one matching prior occurrence;
one old object cannot authorize multiple copies. Validation or catalog failure
preserves the complete previous layout.

Catalog install/remove and block apply use the same existing immediate SQLite
transaction owner, so a block apply observes one latest committed catalog state.
It does not accept a caller catalog revision and does not claim that a concurrent
catalog mutation conflicts. If removal commits first, an old reference is subject
to the unavailable-retention rule; if block apply commits first, a later removal
leaves its placement as a placeholder. Installing a pack never edits a block,
and saving a block never changes catalog membership.

## Built-in compatibility and quotas

The read-only embedded built-in pack is always part of the resolver and never an
installed catalog row. It consumes no mutable catalog quota and cannot be removed.
Block-v2 fixtures pin its exact JSON bytes and digest and map legacy assets:

| v1 asset | Built-in prop key |
| -------- | ----------------- |
| `desk`   | `desk`            |
| `chair`  | `chair`           |
| `plant`  | `plant`           |
| `rug`    | `rug`             |

Legacy SQLite token lists and v1 command input map to these digest-pinned
placements with their existing footprints. A read performs that mapping only in
memory. The first later genuinely changed local mutation stores the complete v2
document under the same block revision CAS; a no-op or exact retry leaves the v1
storage bytes untouched. The built-in pixels must preserve recognizable desk,
chair, plant, and rug geometry at desktop and narrow sizes. Their expected digest
comes from independently reviewed fixture bytes, never implementation output.

## Revision and failure behavior

Block creation, update, current no-op, exact retry, conflict, maximum revision,
timestamp, identity retirement, and uncertain-write behavior remain the existing
local block policy. Read-time catalog corruption changes only resolution status;
it does not mutate the block, renew its timestamp, or claim a new revision. An
exact reinstall of removed valid bytes, or of the original valid bytes after a
corrupt row is removed, makes the unchanged placement available again. A
different digest never repairs by label or key.

The existing command remains the only placement mutation surface:

```text
tmt office block show --local --identity <name> --json
tmt office block apply --local --identity <name> --file <layout.json> --if-revision <revision> --json
```

`OFFICE_LAYOUT_INVALID` covers malformed v2 data and footprint/bounds failures.
`OFFICE_LAYOUT_UNSUPPORTED` covers v2 sent to the remote v1 operation.
`OFFICE_PROP_NOT_FOUND` covers a newly introduced absent reference;
`OFFICE_PROP_CORRUPT` covers a newly introduced reference whose installed row
fails validation. `OFFICE_REVISION_CONFLICT`, `OFFICE_BUSY`, and
`OFFICE_LOCAL_UNCERTAIN` retain their existing meanings.

## Acceptance mapping

- Shared literal vectors independently exercise canonical v2 values,
  rotation/bounds, immutable references, full prop/pixel/object capacity, and
  unknown fields in Rust and TypeScript. Existing v1 vectors retain legacy
  token/input migration coverage. Rust raw-byte tests independently reject duplicate
  v2 fields before typed decoding.
- Storage tests prove reads do not rewrite, every changed write normalizes v2
  under the existing revision transaction, multiset-aware unavailable
  retention/removal, and complete rollback on validation or block conflict. An
  independent SQL assertion distinguishes a v1 no-op/exact retry that preserves
  bytes and timestamps from the first changed mutation that normalizes to v2.
- Native CLI plus an independent SQLite oracle proves install/remove/reinstall,
  v2 placement, restart, missing/corrupt placeholders, valid-neighbor preservation,
  exact retry, conflict, retirement, and same-name replacement.
- The existing BlockScene/editor and native browser fixture prove one
  built-in/custom rendering path, list-order overlap, selection/removal of a
  placeholder, escaped labels, desktop/narrow geometry, and no external request.
- The browser Docker acceptance runs the representative installed-pack lifecycle
  twice before CI. A renderer mock, fixture-only catalog, or screenshot existence
  without primary inspection is insufficient.
