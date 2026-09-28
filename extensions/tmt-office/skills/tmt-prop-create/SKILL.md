---
name: tmt-prop-create
description: Create and validate bounded data-only TMT Office prop-pack JSON files; use for custom local Office furniture or prop artwork, not executable extensions or remote assets.
---

# Create TMT Office props

Produce one UTF-8 `.tmtprop.json` document. A prop pack is ordinary indexed-pixel
data. Never add scripts, markup, URLs, external asset references, filesystem
paths, prompts, tools, or executable behavior. Choose v2 for authored directional
furniture and optional tint/text; v1 remains useful for simple rotated sprites.
Check the installed companion supports the chosen version. This is a v1 example:

```json
{
  "formatVersion": 1,
  "label": "Signal set",
  "credit": "Example artist",
  "license": "CC0-1.0",
  "palette": ["#00000000", "#ff5533ff"],
  "props": [
    {
      "key": "signal-lamp",
      "label": "Signal lamp",
      "footprint": { "width": 2, "height": 2 },
      "pixels": ["010", "111", "010"]
    }
  ]
}
```

Keep the source within 128 KiB and the v1 limits: 1–16 props; a palette of 1–16
lowercase `#rrggbbaa` colors whose first entry is transparent `#00000000` and
whose remaining entries are opaque; raster width and height from 1 through 64;
no more than 65,536 total raster cells; and footprint width and height from 1
through 8. Root `label`, prop `label`, and `credit` are non-control UTF-8 text;
labels are 1–80 bytes and credit is 1–120 bytes. `license` is 1–64 ASCII letters,
digits, `.`, `+`, or `-`. Use unique lowercase keys matching
`[a-z][a-z0-9-]{0,31}`. Each pixel row is a same-width lowercase hexadecimal
index string, and every index must exist in the palette. Unknown or duplicate
fields reject at every level.

For v2, keep the envelope but set `formatVersion: 2` and replace each prop's
`pixels` with `frames`: exactly four rasters in **South, West, North, East** order.
Use two hex digits per pixel (`00`, `01`, …, `ff`), not v1's single digit.
Bounds are 512 KiB source, 256 palette entries, 128 pixels per raster side,
16,384 cells per frame, 131,072 cells across the pack, and 16 tiles per footprint
side; the 16-prop limit and other metadata rules are unchanged. Each frame must
contain visible pixels. Never mix `pixels` and `frames`.

Author upright views rather than rotating a finished image. Odd rotations swap
the footprint dimensions. Prefer 8 source pixels per tile across all views;
preserve consistent scale, material, perspective and contact placement. Keep
transparent padding where the object should not touch its boundary. Symmetric
props may intentionally reuse views.

V2 may declare `customization` with `tint`, `text`, or both:

- `tint: {"indices":[1,2]}` selects distinct nontransparent palette indices.
  Every selected index must be used somewhere, and every frame must contain
  the channel. Keep outlines, highlights and trim outside it when appropriate.
- `text: {"color":"#fff0c2","regions":[...]}` provides four integer
  `{x,y,width,height}` rectangles in source-pixel coordinates, one per frame.
  Each rectangle must fit its frame. Choose a quiet, contrasting region away
  from ornamental borders and check legibility at normal zoom.

Capabilities contain no user text or chosen tint. Those values belong to each
placement, so two instances can look different without modifying the pack.

Validate the completed file with the installed companion:

```sh
tmt office prop validate --file <pack.tmtprop.json> --json
```

Validation is read-only and also returns advisory `warnings` with prop key,
rotation, code and explanation. Resolve unintended stretching, changing pixel
scale, clipped edges and small text regions. Intentional tiling may touch edges;
an empty warning list is not proof of visual quality. Inspect every direction,
normal-size placement, contrasting tint and text before handing off artwork.

Treat the returned digest as the identity of the exact bytes. Do not hand-edit
or predict it. Preview only when the user asks and the
local Office service is already running; do not start or restart it implicitly:

```sh
tmt office prop preview --file <pack.tmtprop.json> --json
```

Preview is temporary and writes neither the catalog nor a block. Discover the
current catalog revision, install with authorization, and confirm the immutable
digest:

```sh
tmt office prop list --local --limit 20 --json
tmt office prop install --local --file <pack.tmtprop.json> --if-revision <catalogRevision> --json
tmt office prop show --local <sha256:digest> --json
```

To place a validated prop, first read the local world. Use the returned immutable
`<digest>/<key>` reference and the definition's exact footprint. Add an object
to the existing world's `objects` list; this fragment is not a complete layout:

```json
{
  "id": "30000000-0000-4000-8000-000000000001",
  "kind": "decoration",
  "surface": { "type": "floor" },
  "extension": null,
  "placement": {
      "prop": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/signal-lamp",
      "footprint": { "width": 2, "height": 2 },
      "x": 4,
      "y": 6,
      "rotation": 0
  }
}
```

```sh
tmt office layout show --json
tmt office layout apply --file layout.json --if-revision <revision> --json
```

Copy the read result's `.layout` (`version`, `map`, `objects`) into the file;
preserve the topology and other objects. Generate a unique UUID for each new
placement; never reuse the example UUID. `layout` needs no identity or scope
selector. At revision 0, also pass `--legacy-basis <legacyBasis>` from that same
read; omit it at later revisions. Keep all placements on valid floor or wall
surfaces; the entire world is bounded to 4 MiB and 4096 objects.

For a prop that declares the corresponding capabilities, add
`"customization":{"tint":"#803060","text":"Studio"}` inside `placement`.
Tint is lowercase `#rrggbb`; text is nonblank, single-line, at most 24 Unicode
scalar values and 64 UTF-8 bytes. Omit unused fields; remove the placement's
`customization` entirely to restore the original art. Rotation still selects
one upright authored view; the world remains version 1 with or without customization.

On a revision conflict, reread and reconcile; never advance a revision
automatically. Removing a pack is another authorized catalog mutation:

```sh
tmt office prop remove --local <sha256:digest> --if-revision <catalogRevision> --json
```

Removal never rewrites saved layouts. Existing references render bounded
unavailable placeholders; reinstalling the exact original bytes restores them.
