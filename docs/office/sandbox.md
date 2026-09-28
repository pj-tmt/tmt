# Data-only Office sandbox

Status: the #238 source branch implements bounded local data-only prop creation,
catalog, preview, placement, and placeholders. Installed availability still
requires a coordinated verified release; remote publication and exploration APIs
remain proposals. [Architecture](architecture.md) describes current behavior;
[commands](commands.md) owns implemented and proposed CLI syntax.

The local-first M1 path can use the fixed catalog through direct local block
commands or the optional loopback UI without Firebase, remote pairing or a
running browser service. This document scopes only future custom content; local
identity selection is not a process-security boundary between processes sharing
the user's account.

## Boundary

Office is an optional TMT extension. Community content inside Office is data,
not another executable plugin system. A prop authoring API accepts declarative
pixel data, palette indices, dimensions and bounded metadata. It never accepts
scripts, shaders, HTML, arbitrary SVG, external asset URLs, prompts, tool calls
or behavior hooks. Only the trusted renderer interprets a supported schema.
The existing curated SVG primitives are repository code, not permission to
upload SVG. No runtime SDK, marketplace or general plugin loader is needed.

For the remote/custom-content proposal, authoring, world admission and placement
are separate operations:

1. Validate and preview a candidate; identify an immutable version by verified
   content. Updating a prop creates a new version, not changed bytes at an old ID.
2. A world owner explicitly admits that version to the world's available catalog.
   Visiting or discovering a prop never installs or authorizes it automatically.
3. An assigned agent may place admitted props within its block's bounds and
   object budget. Layout revisions retain the existing conditional-write policy.

Removing a catalog entry prevents new placement. Existing references remain
identifiable; unavailable or unsupported assets render a bounded placeholder,
without fetching arbitrary URLs or breaking the room. Clearing placed objects
is a separate explicit layout edit. Distribution, referenced-asset retention
and garbage collection must be specified before storage implementation.

The fixed four-asset [home block v1](../../extensions/tmt-office/contracts/block-v1.md) remains
unchanged and is the single built-in catalog/layout owner. Custom props use the
singular proposed `tmt office prop` namespace and need a versioned successor
contract, not permissive fallback decoding or a parallel layout copy;
custom-prop commands are unimplemented and tracked in
[#238](https://github.com/wkh237/tmt/issues/238). Before implementation,
fix numeric byte/dimension/palette/catalog quotas, identifier and digest format,
schema, storage paths, access rules, version migration and operation costs.
Shared independent conformance vectors must cover the renderer, native input and
authoritative write boundary; do not assume Firestore Rules can validate arbitrary
artwork cheaply.

## Remote exploration without execution

Exploration returns structured, permission-filtered world/block observations,
available prop versions, geometry and allowed operations. It does not move an
agent, start a session, install content or grant work authority. A map is a
permitted projection, not an unrestricted world directory. Report observation
age and unknown/offline states rather than inventing live presence. Query scope,
pagination and listener counts must be bounded before these endpoints ship.

Capability descriptions come from trusted TMT schemas. Community names, pixels
and metadata are untrusted data even after shape validation; visual text can
still contain prompt injection. Initially omit free-form instructions and
behavior descriptions, constrain labels/control characters and separate their
origin in output. Never interpolate community text into system notices or
agent instructions. A data sandbox reduces execution risk; it does not make
all content safe for an already-privileged model to obey.

## Remote assignment and decoration

For a remote world, reuse local TMT identity UUIDs; do not create a second agent
registry. A world owns its blocks, and a scoped assignment permits a particular
device/local identity to edit a selected block. Both temporary and saved identities qualify.
Names, pane titles and folders are never authority. Assignment is separate from
publishing a remote work capability.

| Event                                         | Assignment and block result                                       |
| --------------------------------------------- | ----------------------------------------------------------------- |
| Temporary identity conclusively retires       | End assignment authority; retain decoration as unassigned space   |
| Saved identity is offline                     | Retain assignment, but do not claim current availability          |
| New identity reuses a display name            | No inherited assignment or credentials                            |
| Owner reassigns the block                     | Fence the former grant; preserve layout unless explicitly cleared |
| Local evidence or connectivity is unavailable | Report unknown; never treat uncertainty as proof of retirement    |

For the remote slice, do not create an enduring block automatically for every
temporary identity. Start with explicit owner assignment; self-service seats and
capacity policy are later work. The server cannot instantly observe a disconnected
local pane. Before remote device writes ship, specify principal-to-identity binding,
finite lease duration/renewal, revocation checks and stale-writer fencing. Server
authorization must expire without trusting the client to report retirement. Lease
expiry suspends remote authority, not decoration retention, and is not proof that
an agent died. TTL physical deletion is not an authorization mechanism.

## Trusted contextual notices

Optional notices announce local identity creation, newly assigned space or
changed access; they are event-driven, not random prompts or cloud broadcasts.
Deduplicate by immutable local identity and event revision, not display name.
Use locally known state with no per-command Firestore lookup. User configuration
can disable advisory notices; required command errors still report normally.

Never modify reply bodies, files, receipts or exact command payloads. Human-mode
advisories may use stderr; JSON stdout retains the existing native envelope.
Any future structured notice field needs one reviewed output contract before
implementation. Use fixed trusted wording and safe identifiers, not community
prose. Advertising an assignment is not authorization to install, pair or spend
model tokens. Update the canonical installed skill only with actual commands.

## Acceptance gates for implementation slices

- Reject executable/unknown fields, external references, malformed palette or
  pixels, oversized inputs and unauthorized/cross-block writes; retain positive
  controls at full supported capacity.
- Prove immutable-version verification, unsupported/missing-asset placeholders
  and independent durable layout results, not a renderer-only mock.
- Exercise temporary retirement, saved/offline identity, same-name replacement,
  reassignment, lease expiry and disconnected stale writers.
- Prove exploration has no installation, execution or unauthorized disclosure
  effects; measure bounded reads/writes rather than per-frame synchronization.
- Test notice deduplication, disabled mode, control-character handling, zero
  added network lookups and byte-identical JSON/payload output.

Custom authoring depends on a verified local installation and the fixed-catalog
block path. Remote prop admission, assignment and server lease enforcement are a
separate M2 slice; they do not gate local commands. Social chat, arbitrary
runtimes, remote work, guest federation and production deployment remain outside
this sandbox.
