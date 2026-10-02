# Directional furniture source artwork

These generated source sheets are authoring inputs for the directional workstation,
lounge, reception and facilities v2 packs.
The built-in image-generation tool produced transparent PNGs using the retained
modular-v1 furniture and mounted-functional sheets as identity/style references.
The four `*-import.json` manifests own reviewed crop bounds, source hashes and
direction order. `scripts/art/encode-props.py` reproduces their contract bytes.
Each prop uses one shared scale across its four views; narrow side views are not
enlarged to fill their cells. The reception variants reuse the same source at
their retained footprint sizes. Native/browser admission remains authoritative.

## Generation specification

Use case: `stylized-concept`. Produce one matching furniture asset per transparent
sheet, with four upright cardinal views in a two-by-two arrangement: South front,
West side, North rear, East side. Keep a fixed elevated orthographic camera,
physical scale, warm upper-left lighting, crisp pixel clusters, honey oak,
emerald upholstery, dark teal outlines and brass details. Preserve the referenced
object's identity; do not rotate its raster sideways. Keep generous transparent
padding, no labels, checkerboard, floor, room, external shadows or extra objects.
Generated cell order must be inspected rather than trusted.

| Source                  | Subject-specific prompt constraints                                                                                                                                                                                                              |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `desk-source.png`       | Empty writing desk; rectangular oak top, right pedestal drawer/cupboard with brass pull, open knee space and square legs. Real side panels and closed rear; no monitor or chair.                                                                 |
| `terminal-source.png`   | Dark teal monitor, ivory keyboard and mouse, no desk. Rear opaque casing and stand, keyboard occlusion; narrow side edge with equipment in front.                                                                                                |
| `bookcase-source.png`   | Oak cabinet, books, plant, drawers and brass pulls. Rear closed planks; narrow cabinet sides, same height without side-view enlargement.                                                                                                         |
| `sofa-source.png`       | Three-seat emerald velvet sofa, rounded arms and short wood feet. Corrected side views retain three cushions along the foreshortened ground-depth axis and a long side backrest, never a single-armchair silhouette. Preserve front/rear design. |
| `armchair-source.png`   | Emerald lounge armchair, not rolling chair: padded back, thick arms, single cushion, short wood feet and closed upholstered rear.                                                                                                                |
| `table-source.png`      | Empty oval oak coffee table, thick rim, wood grain and sturdy short legs. Side views turn the long axis into ground depth; legs remain below the tabletop.                                                                                       |
| `plant-source.png`      | Tall leafy plant in terracotta pot. Same plant branching arrangement turned in the ground plane; stable pot, height, scale and lighting.                                                                                                         |
| `whiteboard-source.png` | Silver/dark-teal framed blank whiteboard with marker tray. Opaque metal rear and narrow metal sides; do not add a stand.                                                                                                                         |
| `discussion-source.png` | Thick oak-framed cork board, four ivory notes with colored pins in front; closed wood rear and thick side frame, no rear notes or stand.                                                                                                         |
| `radio-source.png`      | Oak-cased teal radio, brass knobs, display and desktop microphone on one base. Rear ventilation and cable without front controls; consistent side occlusion.                                                                                     |

The historical packs remain immutable. Directional packs use new content-addressed
identities; existing placements do not change on read. The first completed rotation
of a supported static floor prop selects its compatible successor within the same
world edit. Undo restores the original reference as well as placement. The existing
rolling chair already has authored views and does not need a successor.

Reproduce one pack with the optional pinned dependencies from
`scripts/art/requirements.txt`:

```sh
python scripts/art/encode-props.py docs/office/references/furniture-rotation/lounge-import.json
python scripts/art/test_encode_props.py
```

The encoder checks exact bytes by default; `--write` regenerates only the manifest's
contract output. Verify all four orientations in the native furniture-rotation
browser gallery after changing source art, crop bounds or encoding policy.
