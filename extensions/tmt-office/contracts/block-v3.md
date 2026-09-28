# Customizable local layout v3

Local implementation in progress. This contract defines the data path; renderer,
inspector and authoring acceptance must pass before the feature is delivered.

The existing local block document gains version `3`. Placements retain their
immutable `prop`, `footprint`, `x`, `y` and `rotation`, with an optional nonempty
`customization` object containing `tint`, `text`, or both. Unknown fields and
explicit null are invalid at every level. Version 2 rejects customization;
legacy remote token layouts do not support it and must never silently drop it.

`tint` is an opaque lowercase `#rrggbb` color. `text` is a nonblank single-line
Unicode string, limited to 24 Unicode scalar values and 64 UTF-8 bytes. Control
characters and directional embedding/override/isolate controls are rejected.
It is inert display text, never HTML, a prompt, a URL or executable instructions.

The resolved prop must declare each requested capability in its v2
`customization` definition. Tint specifies unique nontransparent palette indices;
every declared index must exist in the artwork and every direction must use the
channel. Text specifies one in-frame pixel rectangle per authored direction and
an opaque lowercase text color. No custom fonts or code are accepted.

Layout admission owns value syntax and bounds; prop resolution owns capability
authorization. The same native transaction handles CLI and HTTP. A missing or
corrupt pack preserves the exact placement, including customization, as an
unavailable item. Existing exact-occurrence retention, no-op, retry and conflict
rules apply. Editing an unavailable item's custom values is not retention.

The canonical encoder emits v3 when any placement has customization, otherwise
v2. Accepting an uncustomized v3 document does not preserve a redundant version
marker. No-op detection compares admitted placement values, not input formatting.
Schema20 preserves rows, revisions, lobby uniqueness and identity ownership while
raising the canonical layout ceiling to 8192 bytes. Acquisition/HTTP/child limits
remain 64 KiB; unrelated resource limits do not change.

Both renderers must consume the same admitted customization projection. Tint
affects only declared indices and preserves their brightness variation; it does
not tint the full texture. Text stays in the declared rectangle with trusted
font rendering. Texture variant identity includes custom values; normal scene
sweeping releases replaced variants without adding an idle render loop.

Acceptance includes native/browser parity, all four text regions, capability
rejection, full-capacity durable save, exact retry/conflict, unavailable retention
and reinstall, independent tint/text texture variants, explicit Save/discard,
restart and production-browser visual proof.
