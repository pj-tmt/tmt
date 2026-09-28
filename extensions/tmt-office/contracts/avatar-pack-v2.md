# Office avatar pack v2

Status: implemented locally; not release availability.

V2 adds higher-detail static character art to the same avatar catalog and profile
selection flow as [v1](avatar-pack-v1.md). The root and avatar member names,
strict file/JSON acquisition, text rules and immutable reference syntax are
unchanged. Unknown fields remain invalid. There is no animation, behavior, URL,
script, customization command or new authority in a pack.

| Property                                  | V1      | V2                        |
| ----------------------------------------- | ------- | ------------------------- |
| `formatVersion`                           | `1`     | `2`                       |
| Raster                                    | 16 × 24 | 32 × 48                   |
| Rows / encoded characters per row         | 24 / 16 | 48 / 64                   |
| Lowercase hex digits per pixel            | 1       | 2, including leading zero |
| Palette entries                           | 1–16    | 1–256                     |
| Avatars within the 6,144-cell pack budget | 1–16    | 1–4                       |

Both versions retain the 32 KiB exact-file limit, 6,144 total raster-cell limit,
64-pack/256-avatar retained catalog limits and fully transparent index zero. Every other
palette entry is opaque. At least one pixel per avatar is nontransparent. V2
uses the existing two-digit indexed-art validator and renderer shared with prop
v2; it does not widen generic indexed-art defaults.

V2 digest framing uses `TMT-OFFICE-AVATAR-PACK-V2\0`, followed by the exact file
byte length as an unsigned eight-byte big-endian integer, then the original file
bytes, all hashed with SHA-256. V1's framing and existing digests are unchanged.
No storage migration, rewritten pack or profile reference is required.

Higher source resolution does not enlarge the character's world footprint.
Catalog selection and preview preserve the encoding width through both SVG and
GPU paths. Shirt-mark and identity labels remain inert host overlays. Missing
art still falls back to the profile's saved default; exact reinstall restores
the original selection without rewriting the profile.

Advisory lint interprets whole palette indices, not individual hex characters.
`small-silhouette` is fewer than one eighth of the canvas cells: 48 in v1 or
192 in v2. Edge and actual-color checks retain the same semantics. Warnings
never prevent installation or change exact source bytes.

The [sample](avatar-pack-v2-sample.tmtavatar.json) and
[mutation vectors](avatar-pack-v2-vectors.json) are shared native/browser
conformance fixtures. The sample deliberately expands a grid-authored robot
and uses palette index `10`; it proves encoding, not finished art quality.
Its exact-byte digest is
`sha256:371a80e4b0e24cd0d8f5b948e506074ab3be536529c8220db3819e0a34d76831`.
