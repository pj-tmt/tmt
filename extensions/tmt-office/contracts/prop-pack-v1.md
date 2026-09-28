# Local data-only prop pack v1

Status: implemented in the #238 source branch after contract review. Installed
availability still requires a coordinated verified release. Remote publication
and world admission remain M2.

## Boundary

A prop pack is one bounded UTF-8 JSON file containing untrusted declarative
presentation data. It cannot contain or reference executable code, HTML, SVG,
shaders, fonts, URLs, prompts, tool calls, behavior hooks, animation, audio,
paths, or network resources. Unknown fields reject. This structural boundary
does not claim to detect malicious prose in allowed labels or credits; those
strings remain untrusted display text and never become instructions.

The optional `tmt-prop-create` skill belongs to #238. It may author this ordinary
text format and is delivered through the existing managed-skill installation
path shipped by #213. It is not an installer. #238 adds no marketplace, plugin
loader, provider copy, archive extractor, or second installation framework.
Prop packs never enter or reuse the native executable-artifact archive, manifest,
allowlist, extraction, or activation pipeline.

## Exact JSON document

One `.tmtprop.json` file contains between 1 and 16 props:

```json
{
  "formatVersion": 1,
  "label": "Studio basics",
  "credit": "Example author",
  "license": "CC0-1.0",
  "palette": ["#00000000", "#24313aff", "#dca77dff"],
  "props": [
    {
      "key": "reading-lamp",
      "label": "Reading lamp",
      "footprint": { "width": 2, "height": 2 },
      "pixels": ["0000000000000000", "0000000110000000"]
    }
  ]
}
```

The example is a complete 16×2 raster: every prop's `pixels` array has exactly
its raster height rows, and every row has one lowercase hexadecimal palette index
per pixel. The raster width is the row length and must be identical across rows.
The raster height is the row count. No separate width, height, path, binary file,
archive, or alternate encoding exists.

The complete file is at most 128 KiB. The existing bounded no-follow regular-file
reader acquires it before JSON decoding. JSON rejects a BOM, invalid UTF-8,
duplicate keys, unknown keys, non-integer numbers, and trailing data. Equivalent
JSON with different whitespace or object-key order is intentionally different
content with a different digest; there is no canonical-JSON claim.

Duplicate-key rejection belongs to bounded Rust raw-file admission, before typed
decoding. The repository's default `serde_json` last-key-wins behavior alone is not
sufficient, and #238 does not introduce a second general JSON framework. The browser
receives already-parsed values, so its independent object decoder proves exact fields
but cannot observe duplicate source keys. Shared literal vectors cover value, field,
geometry and full-capacity behavior in Rust and TypeScript; Rust-only raw-byte tests
cover duplicates in the envelope, footprint and prop objects.

| Value                               |             Limit |
| ----------------------------------- | ----------------: |
| Complete UTF-8 file                 |           128 KiB |
| Props                               |      1 through 16 |
| Total palette-index characters      |            65,536 |
| Palette-index characters per prop   |   1 through 4,096 |
| Raster width or height              |      1 through 64 |
| Palette entries                     |      1 through 16 |
| Unrotated footprint width or height | 1 through 8 tiles |

`label`, `credit`, and each prop `label` are plain single-line Unicode strings.
They reject Unicode control characters and are limited to 80, 120, and 80 UTF-8
bytes respectively. They are preserved exactly, never trimmed or truncated.
`license` is 1 through 64 ASCII bytes matching `[A-Za-z0-9.+-]+`; it is display
metadata, not proof of legal compatibility.

A prop `key` is 1 through 32 lowercase ASCII bytes matching
`[a-z][a-z0-9-]*`, unique within the pack. Props remain in file order for display
only; immutable references use the key, not its list position.

Palette entries are lowercase `#rrggbbaa`. Index zero must be transparent
`#00000000`; every other entry must have `ff` alpha. Each pixel character is one
of `0` through `9` or `a` through `f`, and its numeric value must be less than
the palette length. The trusted renderer scales the complete nearest-neighbor
raster into the unrotated footprint and applies only the block placement's four
clockwise quarter turns. There are no alternate orientation frames.

## Immutable digest and reference

The pack digest is lowercase SHA-256 over this framed exact content:

```text
UTF-8("TMT-OFFICE-PROP-PACK-V1\0")
u64be(file byte length)
exact validated .tmtprop.json bytes
```

The immutable textual reference to one prop is:

```text
sha256:<64 lowercase hexadecimal digest>/<prop-key>
```

Uppercase hexadecimal, a missing `sha256:` prefix, an unknown digest or key, or
any alternate spelling is not the same reference. Editing whitespace, metadata,
palette, geometry, or pixels creates a new pack identity by design.

This uses a dedicated framed prop-pack digest helper. The existing raw-content
SHA-256 helper and native artifact digest verifier are intentionally incompatible
and must not be reused for this identity.

## Built-in pack

The four existing built-ins ship as one read-only repository-owned JSON pack
conforming to this schema. It is embedded product data, not an installed SQLite
row; it does not consume the 64-pack or 256-prop local quotas and cannot be
removed. Literal block-v2 fixtures freeze its exact bytes, expected digest, keys,
geometry, and legacy mapping before implementation.

Production resolves built-ins and installed packs through one validated
prop-definition interface and renders both with one indexed-pixel renderer. The
current hard-coded Sprite branches are migration input, not a permanent second
custom-prop pipeline.

## Local catalog, revision, and atomicity

The existing local SQLite database is the only mutable M1 catalog authority. A
versioned migration adds one installation-wide catalog singleton and immutable
pack rows. The singleton has fixed key `1`, nonnegative `revision`, and nullable
`previousKind`, `previousDigest`, `previousBaseRevision`, and
`previousResultRevision`. A pack row is keyed by its validated digest and stores
the validated exact JSON bytes, admission-time prop count, installation revision,
and installation timestamp. No unpacked directory, filesystem cache, or
provider-specific copy exists.

Every installed row counts toward the 64-pack quota even when its bytes later
fail read-time validation. The 256-prop quota sums each row's separately stored,
admission-time prop count; reads never recalculate that count from corrupt bytes.
An out-of-range or otherwise corrupt stored prop count makes operations that can
increase catalog membership fail closed. Install fully reads, bounds, strictly
decodes, validates, and hashes the candidate before opening the existing
immediate SQLite transaction. It checks the caller's expected revision before
evaluating any no-op. The transaction validates all quota metadata, checks the
current quotas, inserts the exact bytes, and advances catalog revision once. The
embedded built-in pack is outside both quotas. Installing an already valid
identical installed digest is a no-op with `changed:false` and does not renew
revision or timestamp. Installing the exact embedded built-in bytes is likewise
a no-op and never creates a shadow row or consumes quota. A digest collision with
different exact bytes fails closed. Validation, quota, or storage failure leaves
the catalog unchanged.

Remove accepts a syntactically valid exact pack digest and an expected catalog
revision. It rejects the embedded built-in digest. A present installed row is
deleted and advances catalog revision once without decoding or trusting its
payload or admission-time prop count, so the target corrupt row remains removable.
Because removal only reduces membership, invalid quota metadata on other rows
does not block it. Revision exhaustion still rejects a changed removal. Removing
an absent digest at the current revision is an idempotent `changed:false` no-op.
On a changed mutation, the same transaction replaces the singleton's previous tuple with exactly
`(kind, digest, baseRevision, resultRevision)` and advances `revision` from the
base to the result. For install, digest matching also requires the newly validated
candidate bytes to equal the installed row bytes; a collision fails closed.
Repeating that exact install candidate or remove digest with `--if-revision`
equal to `previousBaseRevision`, while current revision equals
`previousResultRevision`, is an exact retry with `changed:false`. A current no-op
does not replace the previous tuple. Any other stale revision conflicts. There is no
update, automatic removal, recursive garbage collection, download, or remote
distribution. Reinstalling the exact same valid bytes after removal restores the
same digest under a new catalog revision.

Placed block references are independent of membership. Remove never edits a
block: its placements retain their digest, geometry, order, and revision and
become placeholders until the same valid bytes are reinstalled.

## Bounded read-time validation and cache

Every list, show, preview, or browser catalog load runs the same domain validation
used by install. A query reads validated digest keys and SQLite `length(bytes)`
before loading a payload. Acquisition uses the largest supported pack-version
ceiling; rows above it are excluded without loading their BLOB. V1 admission
still rejects sources over 128 KiB. Each catalog page scans at most 20 rows;
the [v2 storage bounds](prop-pack-v2.md#process-storage-and-presentation) cover
mixed-version catalogs. Stored row/prop quotas bound a complete traversal.

Each list, show, or browser refresh opens one bounded read transaction, observes
its catalog revision, loads the required bytes, and rehashes them into a validated
request/render snapshot. Within only that snapshot, `(catalogRevision, digest)`
deduplicates resolution and rendering performs lookups without per-object reads
or hashes. The cache is never trusted across a new command, request, or normal
browser refresh: a new observation rereads and revalidates bytes even when the
revision did not change. An already-rendered browser snapshot may remain visible
until its next normal refresh. Corrupt packs are then excluded while valid
neighboring packs remain usable. Tests tamper with stored bytes without advancing
the revision and prove the next observation detects the change.

A page returns `excluded` entries containing only a syntactically validated row
digest and one fixed reason: `oversized`, `digestMismatch`, or `invalidDocument`.
It never returns corrupt labels, credits, palette, pixels, or parser details.
Read-time validation does not repair or delete a row.

List order is digest byte order. One page examines at most the next 20 stored
rows after its exclusive cursor, whether each row becomes a valid result or an
excluded entry. If more rows remain, `nextCursor` advances to the last examined
row, not the last valid result, so corrupt rows cannot repeat or stall traversal.

The opaque cursor is unpadded base64url of exactly 41 bytes: byte `0x01`, the
observed catalog revision as unsigned u64 big-endian, and the exclusive last
digest as its 32 raw bytes. Any other length, version, alphabet, padding,
overflow, or non-row digest rejects. Callers must not construct or interpret it.
A different current revision returns
`OFFICE_CATALOG_CURSOR_STALE` rather than mixing snapshots.

## Proposed command contract

These commands require a compatible installed Office companion. They never
resolve or mutate an identity and never contact Firebase or a remote catalog.
JSON success is one object on stdout, with empty stderr and exit zero.

```text
tmt office prop validate --file <pack.tmtprop.json> --json
tmt office prop preview --file <pack.tmtprop.json> --json
tmt office prop install --local --file <pack.tmtprop.json> --if-revision <catalog-revision> --json
tmt office prop remove --local <digest> --if-revision <catalog-revision> --json
tmt office prop list --local [--limit <1..20>] [--cursor <opaque-cursor>] --json
tmt office prop show --local <digest> --json
```

Validate performs no durable mutation and returns the pack projection below,
plus a CLI-only `warnings` array of advisory authoring findings. The
[directional pack contract](prop-pack-v2.md#identity-and-compatibility) defines
warning fields and codes for both formats. Warnings do not reject valid art;
v1 inspects only its single raster and has no directional-scale requirement.

```json
{
  "digest": "sha256:...",
  "formatVersion": 1,
  "label": "Studio basics",
  "credit": "Example author",
  "license": "CC0-1.0",
  "fileBytes": 1234,
  "pixelCount": 32,
  "props": [
    {
      "key": "reading-lamp",
      "label": "Reading lamp",
      "footprint": { "width": 2, "height": 2 },
      "raster": { "width": 16, "height": 2 }
    }
  ]
}
```

Install returns that projection plus `builtin`, current `catalogRevision`,
nullable `installedAtMs`, and transactional `changed`; the built-in no-op uses
`builtin:true` and `installedAtMs:null`. Remove returns exact `digest`, current
`catalogRevision`, and `changed`.

List's `--limit` defaults to 20, accepts 1 through 20, and bounds stored rows
examined rather than only valid results. It returns current `catalogRevision`, a
separate stable `builtins` summary array outside that limit/cursor/quota, valid
installed `packs`, bounded `excluded`, and nullable `nextCursor`. The built-in
summary exposes its digest, label, and prop key/label/footprint/raster projection
through the same catalog interface but never appears as an installed row.

Show accepts one exact `sha256:<hex>` installed or built-in pack digest and
returns its currently validated projection without pixel rows, plus `builtin`,
current `catalogRevision`, and nullable `installedAtMs`. An uncertain mutation
therefore obtains the authoritative current revision by listing or showing its
digest before retrying the identical file/digest and original revision.

Expected errors remain distinct: `OFFICE_PROP_INVALID`, `OFFICE_PROP_CORRUPT`,
`OFFICE_PROP_NOT_FOUND`, `OFFICE_PROP_LIMIT`, `OFFICE_PROP_BUILTIN`,
`OFFICE_CATALOG_REVISION_CONFLICT`, `OFFICE_CATALOG_CURSOR_STALE`,
`OFFICE_SERVICE_NOT_RUNNING`, `OFFICE_PROP_PREVIEW_LIMIT`,
`OFFICE_NOT_INSTALLED`, `OFFICE_BUSY`, and `OFFICE_LOCAL_UNCERTAIN`. Unexpected
storage failure does not become invalid content.

## Process-local browser preview

Preview requires an already-running compatible local Office service. It never
starts or restarts Office implicitly. A missing or stale service returns the
actionable `OFFICE_SERVICE_NOT_RUNNING` error with no retained candidate.

The CLI validates the candidate, then sends its exact bytes as the request body
to `POST /control/v1/prop-previews` on the existing control bearer plus nonce
boundary. Request parsing first bounds and validates the request line and headers,
then selects a route-specific body cap: exactly 128 KiB for this path and the
existing 64 KiB cap everywhere else. The preview route requires
`Content-Type: application/json`, rejects transfer encoding, and validates the
document again inside the service. It is the only exception to the current
empty-body control rule. Health and stop retain empty bodies and all existing
Host/authentication checks. This uses no new listener, process, receipt, token
type, or authentication system.

The service keeps at most four validated previews in process memory. Each has a
random bounded preview ID and expires exactly five minutes after acceptance.
Expired entries are discarded lazily before capacity checks and reads. Repeating
the same digest returns its existing ID and expiry without renewal. At four live
distinct entries, another preview returns `OFFICE_PROP_PREVIEW_LIMIT`; it does
not evict or replace a live entry. Service stop or process exit disposes every
preview.

Success returns `digest`, `previewId`, `expiresAtMs`, and
`http://127.0.0.1:<port>/local/props/preview/<previewId>#token=<browserToken>`.
The page synchronously reads the existing fragment token, removes the fragment
with `history.replaceState`, retains the token only in memory, and then fetches
`GET /api/v1/local/prop-previews/<previewId>` with the existing browser bearer.
URL fragments are not sent in HTTP requests or referrers; the response retains
the service's existing private/no-referrer headers. No pack bytes, labels, or
pixels appear in the URL, history, or referrer. The API returns the validated
in-memory candidate through the same catalog projection and indexed-pixel
renderer as installed content but performs no SQLite write, catalog admission,
block placement, or external request.

## Acceptance ownership

- Literal valid/full-capacity/invalid JSON files and digest/reference vectors
  live in `extensions/tmt-office/contracts`; Rust domain and browser decoders consume them
  independently.
- Rust unit tests own file bounds, duplicate/unknown fields, exact digest,
  palette/pixel validation, catalog quotas, install/remove/reinstall, corrupt-row
  removal, bounded reads, exclusion reasons, cursor revision, contention, exact
  retry, and migration history.
- HTTP parser tests prove the preview route alone accepts bodies through 128 KiB,
  all other routes retain 64 KiB, and oversized/transfer-encoded bodies reject
  before allocation or dispatch.
- Native process tests own actual grammar, bounded file acquisition, JSON/errors,
  stopped-service validate/install/remove/list/show, preview service requirement,
  and independent SQLite observation.
- The existing native Office browser fixture owns CLI to SQLite to catalog to
  placement to visible renderer proof, missing/corrupt/removed placeholders,
  reinstall restoration, restart, and unchanged valid neighbors.
- Desktop and narrow screenshots require primary visual review. The established
  browser Docker target runs representative acceptance twice before CI.
- Tests independently prove that pack content is never executed or sent to
  unrelated processes, network, prompts, skills, board/exchange bodies, identity
  state, or Firestore.
