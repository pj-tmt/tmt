# Directional prop pack v2

Status: local implementation in progress for the modular workshop. Not released.
This extends the existing prop catalog; it is not a second asset loader or an
extension runtime. Optional data-only customization capabilities are described
below; their placement values use [local layout v3](block-v3.md).

## Document and orientation

The envelope retains `formatVersion`, `label`, `credit`, `license`, `palette`
and `props` from [v1](prop-pack-v1.md). Version 2 uses `formatVersion: 2`.
Each prop contains `key`, `label`, `footprint` and `frames`, plus optional
`customization`. No other fields are accepted.
`frames` contains exactly four indexed rasters, in clockwise placement order:

| Rotation | View  | Display footprint |
| -------- | ----- | ----------------- |
| 0        | South | width × height    |
| 1        | West  | height × width    |
| 2        | North | width × height    |
| 3        | East  | height × width    |

The renderer selects the authored view and keeps it upright. It does not rotate
the selected raster again. Each complete raster fits the corresponding rotated
footprint. Workshop assets should use 8 source pixels per tile consistently;
this authoring recommendation is not an extra admission requirement.

Each pixel is exactly two lowercase hexadecimal digits. `00` is transparent;
other indices select opaque palette entries. Rows must have equal, even lengths.
At least one visible pixel is required in every frame. See the small asymmetric
[shared fixture](prop-pack-v2-sample.tmtprop.json) for an executable contract
example, not production artwork.

| Bound                                   |    Version 2 |
| --------------------------------------- | -----------: |
| Exact source file                       |      512 KiB |
| Props per pack                          |         1–16 |
| Palette entries, including transparency |        1–256 |
| Raster side                             | 1–128 pixels |
| Cells per frame                         |       16,384 |
| Cells across the complete pack          |      131,072 |
| Unrotated footprint side                |   1–16 tiles |

V1 labels, credits, keys, license spelling, UTF-8 rules and unknown/duplicate-field
rejection remain in force. `pixels` is not a v2 field, including when null or
empty. `frames` is not a v1 field. No image URL, file path, script, HTML, SVG,
shader, font, animation or executable instruction is admitted.

## Local pixel workshop

The browser authoring surface creates a small flat pixel-art prop, not a new
asset format: a 16×16 or 32×32 indexed raster, up to 16 palette entries including
transparency, and four identical v2 views. One source raster deliberately faces
all directions; imported directional packs remain unchanged. Label, credit and
license are explicit metadata; the displayed footprint follows eight pixels per
tile (2×2 or 4×4). New work defaults to owner-local
`LicenseRef-Private`, not an implicit public license or publication.

Drawing, erasing, palette edits and undo/redo stay in a bounded memory draft.
Blank/incomplete work can be previewed but cannot be saved as an invalid pack.
Preview uses the existing indexed renderer; Save uses native `validate_pack`
and the existing catalog revision-CAS install. Never persist a browser-only
pixel schema, HTML, SVG, URL or executable extension. Closing retains the draft
for the mounted page; New drawing explicitly replaces it and its history.
Neither action removes saved artwork. Failed validation/conflict retains the draft.

Open Edit layout → Furniture and wall objects → Pixel workshop and art library.
Pointer capture commits a completed stroke once; cancelled capture discards it.
Arrow keys move the canvas cursor, Space/Enter paints and Delete/Backspace erases.
The eraser is palette index zero. Preview uses the same admitted indexed renderer
as imported art. Layout additions report success only after draft admission.

`POST /api/v1/local/props/list` accepts `{cursor?}` and returns a bounded page
`{revision,entries:[{digest,label}],excluded:[{digest,reason}],nextCursor}`.
Built-ins already belong to the app catalog; entries list installed custom packs.
`POST /api/v1/local/props/install` accepts `{expectedRevision,document}`, where
document is exact UTF-8 pack JSON, and returns `{revision,digest,changed}`.
Both use the existing browser bearer and exact JSON Origin policy. Only the exact
install route admits the 512 KiB pack's escaped JSON envelope. Invalid input is
rejected before storage. Native catalog quota, exclusion, cursor and retry rules
remain authoritative; no second registry or install receipt is introduced.

Freeze document/revision while a save is pending or unconfirmed. Retry preserves
the exact intent; a conflict requires explicit catalog refresh before a new
attempt. Closing or discarding cannot undo a potentially accepted install.
Saving artwork does not alter any placement. Adding a saved prop to the world
edits the current world draft; Save layout commits it separately. Cancel layout
or remove placement retains the catalog pack. The paged custom-art library lets
owners find saved artwork again after restart, without loading every raster.

Acceptance includes native/browser admission, exact-byte install and retry,
conflict/no partial writes, hostile fields, auth/Origin, bounded listing and
missing/corrupt artwork; real-browser drawing/erase/undo/preview/Save, placement
and restart, desktop/narrow input, and preservation of existing layouts/packs.

## Customization capabilities

Optional `customization` contains `tint`, `text`, or both, never null or an empty
object. `tint` contains only `indices`, a nonempty unique list of nontransparent
palette indices used by the prop. Every index must occur in at least one frame;
every frame must contain the channel. `text` contains only `color` (lowercase
opaque `#rrggbb`) and `regions`, four `{x,y,width,height}` pixel rectangles in
frame order. Coordinates and sizes are integers; every rectangle has positive
size and fits entirely within its own raster. V1 admits neither capability.

Validation summaries expose only `customizable: ["tint"]`, `["text"]`, or
`["tint","text"]`, in that order, when declared. Pixel regions and palette
indices remain part of the admitted pack, not repeated in command summaries.
The [shared capability vectors](prop-customization-vectors.json) bind native
and browser admission to the same acceptance cases.

## Identity and compatibility

CLI `prop validate` appends advisory `warnings` after the shared admission gate.
Each warning identifies `prop`, `rotation`, stable `code` and `message`. Codes
cover `nonuniform-scale`, `directional-scale-drift`, `opaque-edge` and
`small-text-region`. Warnings do not modify source, alter digest, reject valid
tiling/symmetry or certify visual quality. They are computed over the admitted
pack, not by a second JSON validator. Install/show summaries and the companion
protocol remain unchanged.

V2 hashes `UTF-8("TMT-OFFICE-PROP-PACK-V2\0")`, the unsigned 64-bit big-endian
source byte length, and the exact source bytes. References retain the existing
`sha256:<digest>/<key>` spelling. V1 uses its original domain and byte limits;
the frozen v1 built-in pack is not replaced or reinterpreted.

Both formats resolve through the same immutable catalog and raster projection.
Existing local placements need no new fields to select directional artwork:
their rotation selects the view. Missing/corrupt/removed packs retain placement
geometry and become placeholders; exact reinstall restores them. No read writes
a layout or catalog revision.

## Process, storage and presentation

Prop admission owns the 512 KiB acquisition ceiling and derives its base64 and
child-envelope bounds. The Office companion uses the same prop-specific input
sentinel. Preview's prop route uses that ceiling; unrelated HTTP bodies remain
64 KiB and avatar limits remain unchanged. V1 still rejects source over 128 KiB.

Schema 18 replaces only the prop BLOB size constraint and copies exact bytes,
digests and installation revisions. The catalog singleton and layout rows are
unchanged. Bounded reads continue excluding oversized or invalid rows before
their content can render. A 20-row candidate page is bounded by 10 MiB of source;
a 16-digest browser resolution is bounded by 8 MiB of source. The unchanged
256-prop catalog quota admits sixteen 16-prop packs, not twenty. A full 20-row
page can instead contain eight-prop packs at the same byte/cell ceilings.
Capacity verification must exercise these reachable cases separately, preserve
bytes and revisions on overflow, and measure serialized responses and browser
resources. These arithmetic bounds are not performance proof.

Validate/install/show/list summaries return no pixels. A v2 prop summary replaces
v1's single `raster` dimensions with four `frames` dimension objects in the order
above. Browser preview uses those same four views. Both SVG and Pixi consume one
frame-selection function and one indexed-cell interpretation. Adjacent same-color
cells share horizontal runs; SVG groups those runs into one inert path per color
(at most 255 visible paths per v2 frame), while canvas paints the same runs.
This bounds DOM node count, not rasterization time. Texture keys include
the immutable reference and frame, and the existing scene owner releases unused
textures; no continuously running animation loop is introduced.

## Verification contract

Required delivery evidence includes native install/preview/restart integration,
directional browser editing, malformed/full-capacity vectors, migration-history
preservation, authoring lint and skill guidance, and production asset contact-sheet
review. Native capacity scenarios exercise full source/cell budgets, the 256-prop
overflow boundary, a 20-row candidate page and 16 distinct browser resolutions.
Record serialized sizes and resource observations with their environment and
limitations; one headless run is not a peak-memory, GPU or cross-machine benchmark.
Passing the small fixture is not reference-image fidelity or release readiness.
