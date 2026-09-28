# Office avatar pack v1

Status: implemented for strict local admission, catalog storage, preview, profile selection
and unavailable-selection fallback.

[V2](avatar-pack-v2.md) adds higher-detail static rasters without changing this
version's dimensions, digest framing or retained catalog behavior.

An avatar pack is one UTF-8 `.tmtavatar.json` regular file of at most 32 KiB. Readers
must not follow symbolic links and reject a UTF-8 BOM, invalid UTF-8, duplicate JSON
members, and members not named below.

The root has exactly `formatVersion`, `label`, `credit`, `license`, `palette`, and
`avatars`. `formatVersion` is `1`. `label` and avatar labels contain 1–80 UTF-8 bytes;
`credit` contains 1–120. Those strings reject control characters. `license` contains
1–64 ASCII letters, digits, `.`, `+`, or `-`.

`palette` contains 1–16 lowercase `#rrggbbaa` entries. Entry zero is exactly
`#00000000`; every later entry has alpha `ff`. Each of 1–16 avatars has exactly `key`,
`label`, and `pixels`. Keys are unique and match `[a-z][a-z0-9-]{0,31}`. `pixels` is
exactly 24 rows of 16 lowercase hexadecimal palette indices. Every index resolves and
each avatar has at least one nontransparent cell. A pack contains at most 6,144 cells.

The immutable pack digest is lowercase SHA-256 over these bytes in order:

1. `TMT-OFFICE-AVATAR-PACK-V1\0` as ASCII bytes;
2. the exact file byte length as an unsigned eight-byte big-endian integer;
3. the exact original file bytes.

The digest is written as `sha256:<64 lowercase hex>`. An avatar reference appends `/`
and its key. Avatar digests and catalog cursors use a namespace distinct from props.
Catalog capacity is 64 retained packs and 256 retained avatars. Corrupt retained rows
still consume capacity and may be explicitly removed. Catalog revisions, conditional
mutations, exact retry, pagination, and stale-cursor behavior follow the typed local
avatar protocol; they never change the prop catalog revision.

The bundled modular robot pack is immutable and does not consume retained capacity.
Native `list` exposes it in `builtins`, separately from paginated installed `packs`;
`show` resolves either source. Snapshots include `builtin` and `installedAtMs`
(null for built-ins, positive for installed packs). Installing identical bundled
bytes at the current revision is a no-op; removing a built-in returns
`OFFICE_AVATAR_INVALID` without changing the catalog. Reads never seed SQLite.
The browser merges both sources into the existing catalog envelope, bounded by
65 packs and 260 avatars, without changing default or explicitly saved appearances.

Preview requires an already-running authenticated loopback Office service, is bounded
and expiring, and does not install a pack or alter a profile. Raster data is validated
before reaching the inert indexed renderer; labels and credits remain untrusted text.

The optional profile `avatarRef` is exactly `<digest>/<key>`. Selecting a new reference
requires an admitted bundled key or installed catalog row and key in the same immediate transaction as the profile
write. Removing a pack never edits profiles. A retained missing or corrupt reference renders
the profile's stored default robot appearance, and exact reinstall restores the art without a
profile revision change. The browser receives one bounded authenticated catalog projection at
startup; animation, movement, arbitrary URLs and runtime plugins are outside this contract.

## Authoring warnings

CLI `avatar validate` appends `warnings` after strict admission. Each warning has
`avatar` (the source key), stable `code`, and `message`. `opaque-edge` means an
opaque cell touches a canvas boundary; `small-silhouette` means fewer than 48 of
384 cells are opaque; `single-color` means the visible cells resolve to just one
distinct RGBA color, excluding unused palette entries. Intentional cropped art,
small characters and flat silhouettes remain valid.

Warnings are read-only advice, not visual approval or another admission gate.
They do not alter source bytes, digest, catalog or profile. Install/show summaries
and the companion protocol remain unchanged. Preview at actual display size to
judge aesthetics, silhouette and the optional shirt-mark overlay.

[`avatar-pack-vectors.json`](avatar-pack-vectors.json) contains shared projection
values. It does not claim to test duplicate-member, raw-byte, file-kind, or symlink
admission; those remain native raw-input tests.
