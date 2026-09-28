# Block document v1

Implemented decoration contract. The browser edits the owner-only home block;
[agent grants](agent-grant-v1.md) reuse this exact document at UUID block IDs.
Agent commands and pairing are not defined by this contract.

`worlds/{worldId}/blocks/home` contains exactly `version: 1`, `revision`,
`objects` and `updatedAt`. Authority comes from the existing immutable world's
owner and current tester admission. There is no second owner/agent identity
field. UUID blocks use the separate scoped authorization contract; other block
IDs, enumeration and deletion remain denied. Clear a room by
saving an empty object list, not resetting its revision counter. Previous layout
versions are not retained as an edit-history feature.

Revision starts at 1 and advances by exactly one, up to JavaScript's maximum safe
integer. `updatedAt` is a Firestore timestamp equal to the write's server request
time. The adapter accepts an expected revision (0 for missing). An online
transaction applies only at that revision. At expected+1, identical ordered
objects constitute an exact retry, with no new write or timestamp renewal.
Anything else conflicts. A subsequent server read supplies canonical data and
may observe a newer revision; it does not prove the writer still owns the latest
edit. Errors after a commit remain uncertain, never an automatic replay with a
new revision. The UI retains its original draft for explicit retry or discard.

## Bounded scene

The room is 32 by 32 tiles with a fixed, curated 16-color palette. Application
inputs and projections use an ordered list of at most 16 exact maps:

| Field      | Constraint                                   |
| ---------- | -------------------------------------------- |
| `asset`    | `desk`, `chair`, `plant` or `rug`            |
| `x`, `y`   | Integer tile origin, starting at zero        |
| `rotation` | Integer 0 through 3, clockwise quarter turns |

Unrotated footprints are desk 4x2, chair/plant 2x2 and rug 6x4. Odd rotations
swap dimensions; the complete footprint must fit in the room. Overlap is
intentional decoration layering, painted in list order. There is no seat
reservation or pathfinding claim. List positions are edit-local indices, not
stable agent/object identities.

Only bounded enums/numbers are accepted, with no arbitrary strings, artwork,
URLs, markup or extra fields. This small fixed schema is well below the broader
64 KiB decoration ceiling without relying on a nonexistent JSON byte-length
check in Rules. Curated SVG primitives are repository code, not stored markup.

Firestore's `objects` field stores that same ordered list as four-character
ASCII tokens, not maps: asset (`d`, `c`, `p`, `r`), rotation (`0`..`3`), X and Y
as one lowercase base-32 digit each (`0`..`9`, `a`..`v`). For example `d1us`
is a desk rotated once at (30, 28). `block-contract.ts` implements the browser
codec; `tmt-office-model::office_block` implements the pure native codec against the same
literal conformance vectors and this contract. Native adapters own readable JSON
and Firestore envelopes; command inputs retain readable named fields. There is
no second stored layout or cache. Unknown tokens/fields reject, never truncate.

Encoding permits one bounded regex per slot to enforce asset-specific rotated
footprints within the Rules expression budget. Vectors contain literal expected
tokens independent of the encoder and test both directions plus actual Rules.

Rules explicitly check all 16 possible slots because they cannot iterate over
an arbitrary list. The independent vectors in `block-v1.vectors.json` run both
against pure client validation and real Rules. Overflow, malformed last-slot and
metadata/revision tests supplement the vectors; schema tests alone are not
authorization evidence.

## Lifecycle and cost

Opening an admitted world subscribes to its single home block. Only confirmed
server snapshots become saved state; a separate local draft is never presented
as saved. Pointer placement and form/keyboard edits cause no remote writes.
Save performs a transaction read, at most one document write, then one server
read; transaction retries and Rules-dependent reads may add reads. Reset takes
the latest server-confirmed layout and discards the local draft explicitly.

Native show performs one authorized document read. Apply reads, conditionally
commits at most one write using the observed server update time (or `exists:false`
for creation), then reads canonical state. An exact retry needs only the read.
Failed preconditions may require another read to distinguish a conflict from an
identical concurrent retry. No automatic write retry or revision rebasing occurs.
Authentication refresh, eligible lease renewal and pending retirement cleanup
can add separate operations; ordinary layout reads do not enumerate the world.

Leaving the world or losing admission disposes the editor, its listener and
private draft; late callbacks cannot repopulate it. Reload still signs out under
the existing memory-only auth policy. Physical room decoration says nothing
about an agent's connectivity, availability or execution.
