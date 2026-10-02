# Modular Office visual package v1

Status: historical high-wall design package and retained generated art sources.
The current [platform contract](../../../../contracts/rooms-and-walls.md)
supersedes its wall geometry, doorway, mounted-authoring and wall-HUD requirements.
This package owns source artwork and reproducible imports, not editor workflows
or a shipped skin system. Unimported views require extraction-time validation
and admission; source-sheet appearance alone does not certify runtime geometry.

The local workstation derivative now admits a desk, terminal, bookcase and four authored
chair views through the existing v2 prop catalog. It does not replace the old
packs or certify unimported views. Its
[import manifest](workstation-import.json) owns reviewed crop bounds and source
hash. `extensions/tmt-office/scripts/art/encode-props.py` reproduces the contract JSON; install its
optional authoring dependencies from `extensions/tmt-office/scripts/art/requirements.txt`, then pass
the manifest path to check exact bytes (add `--write` only to regenerate).
`extensions/tmt-office/scripts/art/test_encode_props.py` checks source fences and reproducibility.
Native/browser validators remain the admission authority.

The [mounted-object import](mounted-import.json) uses the same encoder for a
front-facing celestial window, brass sconce, picture frame and planted shelf.
These derivatives retain historical mounted-instance artwork; they do not
authorize a Walls library in the current platform editor. The source glow is
not baked into a room. Side-facing source views are not admitted by this import.

The [lounge import](lounge-import.json) admits the source sofa, armchair, coffee
table and tall plant as ordinary editable props. The native new-world preset
combines them with existing resource-bound facilities, leaving both Lobby
centerline passages clear. Existing saved layouts are never refurnished by reads.

The [reception import](reception-import.json) provides larger armchair and coffee
table variants for the new Lobby. It preserves the original lounge pack and its
digest, so retained furniture stays resolvable. Both variants use the same source
crops and ordinary v2 admission; no per-instance scale or special renderer exists.

The [facilities import](facilities-import.json) admits the source whiteboard,
cork discussion board and broadcast radio through the same prop validator and
built-in catalog. The new-world preset stores these appearances on the existing
three resource-bound objects, preserving their canonical host bindings. An art
pack alone grants no capability: placing a decorative copy does not create a
board or broadcast endpoint. Existing worlds and extension default appearances
remain unchanged. Compare the larger whiteboard and its transparent frame in the
actual world, not only in the source sheet.

The [robot import](robot-import.json) uses the same encoder for four static front
portraits in the existing 32×48 avatar v2 format. It does not claim directional
animation. The derivative passes native/browser admission,
profile selection and restart persistence; GPU fixtures cover all four variants
and missing-pack fallback. The local native catalog now supplies all four as
built-in choices without installation or SQLite seeding. Existing default and
explicitly selected appearances remain unchanged until the user saves a choice.
The [avatar-pack contract](../../../../contracts/avatar-pack-v2.md)
owns installation and selection. Side and rear source views remain unimported.

Prop derivatives use 8 pixels per tile within guarded square frames; avatars use
the fixed 32×48 portrait frame. Both preserve aspect ratio, quantize to 255 opaque
colors plus transparency, and apply an alpha-128
cutoff. Original PNGs remain unchanged. This is explicitly not lossless alpha
admission. Lounge furniture and desk/terminal/bookcase are static billboards; only the rolling chair has four authored
views. Compare actual rendering before expanding this policy to other assets.

[The product contract](../../../../contracts/rooms-and-walls.md) owns
behavior. [The manifest](assets.json) owns dimensions and the row-major component
inventory. [Generation prompts](generation.json) record built-in tool inputs and
corrections, not additional product requirements.

## Main views

| View                                          | Purpose                                                                             |
| --------------------------------------------- | ----------------------------------------------------------------------------------- |
| [Overview](01-office-overview.png)            | Lobby, four equal personal modules and an independently extensible meeting wing.    |
| [Material skins](06-material-skins.png)       | Workshop, Moonlight and Copper over the same geometry.                              |
| [Office operations](07-office-build-flow.png) | Add a whole office, inspect removal blockers, switch material, place a wall object. |
| [Meeting operations](08-meeting-flow.png)     | Empty-state ghost, required name, participants and safe spatial removal.            |

The [approved style input](00-approved-style.jpg) is preserved for material and
mood comparison only. Its Zone/6×4 workflow is obsolete. Named actors and two
meetings in the overview are a populated demonstration, not default identities
or rooms. The real starter has four unassigned offices and no meetings until
explicit creation.

## Reusable art

Four transparent source sheets contain 64 illustrated components or directional
variants. Grid positions identify artwork, **not** verified crop rectangles:
each source is 1254×1254, not the requested 2048×2048. Do not divide them into a
guessed runtime atlas or silently resize them to fit the current 32×48 avatar
contract. Admit reviewed derivatives through the owning art pipeline.

| Source                                              | Contents                                                                                                               |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| [Architecture](02-architecture-source.png)          | Tall wall, low sill, side faces, corners/junctions, open portals, floor, corridors and bridges.                        |
| [Furniture](03-furniture-source.png)                | Desk, monitor, bookcase, lamp, four chair orientations, sofa, armchair, two tables, plant and three rugs.              |
| [Mounted objects](04-mounted-functional-source.png) | Three window views, sconce, poster, signs, shelf, whiteboard, discussion board, radio, notebooks, mug, plant and glow. |
| [Robots](05-robots-source.png)                      | Four accent colors × four static orientations. No movement or seating animation is implied.                            |

Plain wall sources have no baked windows or sconces. Bridge-edge lights are
structural styling, not selectable lamp instances. Windows, lamps and functional
objects stay separate. Editable text and resource contents are not supplied by
decorative pixels. Whiteboard art is not a document; radio art is not live state.

## Using the sources

Validate crop bounds, directional views, alpha on dark/light backgrounds, scale
and palette limits before admitting another derivative. Compare the assembled
runtime at Fit and close-up scale; source sheets cannot prove seam or occlusion
correctness. Never bake a room background to conceal geometry defects.

Visual baselines must identify exact assets, camera, seeded layout, viewport and
DPR, with explicit raster tolerances. Check narrow views, missing art, culling,
texture disposal and idle rendering separately. Generated bitmap lettering is
not an approved UI font; use real controls and the owning interaction contracts.
This package does not replace runtime source, installed previews or user layouts.
