# Release index v1

This contract owns release discovery documents for #2371. Publication tooling emits
and verifies release records and advances channel pointers after verification.
Writer activation and bootstrap execution require owner authorization. The native
client consumes fixed-origin channel pointers and verified records without an API fallback.

## Origins and identity

The fixed index base is
`https://raw.githubusercontent.com/pj-tmt/tmt/release-index/`.
A channel pointer lives at `channels/<product>/<alpha|stable>.json`. A new release's
record is its `tmt-release-record.json` asset at
`https://github.com/pj-tmt/tmt/releases/download/<tag>/tmt-release-record.json`.
An explicitly reviewed bootstrap may instead place a record at the index base's
`records/<tag>.json`; it never adds assets to immutable historical releases.
Only the newest published release per product/channel is backfilled. An older
exact `--to` without a release record refuses clearly, without an API fallback.

Product keys and tag prefixes come from native release policy. Versions are
canonical `X.Y.Z` or `X.Y.Z-alpha.N`, with nonnegative safe integer parts and no
leading zeroes. Alpha selection follows version, not GitHub's prerelease flag.
Every document requires `schemaVersion: 1`; unknown additive fields are ignored.
A higher schema version refuses without partial interpretation and tells the user
to rerun `install.sh`. Product, version and tag must agree.

## Documents

A record has these required fields:

- `schemaVersion`, `product`, `version`, `tag`.
- `releaseId`: the real positive safe-integer GitHub release ID, retained in receipts
  and handoff protocol 1; never synthesized from a tag.
- `sourceSha`: the lowercase 40-character release commit SHA.
- `manifest`: `{name, url, size, sha256}` for `dist-manifest.json`.
- `archives`: an object keyed by the four native target triples, each with
  `{name, url, size, sha256}` for the product's canonical archive.

The target keys are `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`. Each archive is
`<archive-prefix>-<target>.tar.gz`. Asset names and URLs must match the fixed
repository, exact tag and native publication policy, with no aliases or redirects
encoded in the documents. SHA-256 is 64 lowercase hex characters without a prefix.
Sizes are positive safe integers: manifest at most 4 MiB, compressed archive at
most 64 MiB. These are the existing native artifact bounds.

A pointer has `schemaVersion`, `product`, `channel`, `version`, `tag`, and
`record: {url, size, sha256}`. Its channel must match the version. The record URL
must be one of the two exact locations above for that tag. The pointer digest
binds the selected record; the record binds the manifest and archives.
The cargo-dist manifest remains application-schema, version, checksum and
inventory authority; the record does not replace those checks.

Pointer bodies are capped at 16 KiB and record bodies at 256 KiB. A client checks
these bounds while streaming, before parsing. It checks declared size and digest
before using each fetched document or asset, then retains the existing manifest
checksum, archive safety and receipt checks. Exact `--to` obtains the record
from the release asset directly; channel selection obtains it from the pointer.

## Publication ordering

Draft assembly uploads and digest-checks the archives, manifest and installers,
then constructs the record from the actual release ID, captured source SHA and
those bytes. It uploads the record, checks its stored digest and reads back exact
bytes before uploading `release-publication.json` last. A record upload or
readback failure leaves the completeness marker absent. Existing failure, hold
and owner-authorized retry policy is unchanged.

Publication readback requires a record and compares its identity with the actual
release and resolved tag commit, and its manifest/archive fields with downloaded
bytes and cargo-dist metadata. Existing immutable state, flags, tag, release and
asset attestation checks stay required. A missing record fails distinctly;
pre-record tags are expected to fail this full verifier. Published assets are
never amended, and a failed readback cannot roll back immutable publication.

The index writer may advance a pointer only after every publication
check succeeds. It executes trusted main tooling under the minimum contents-write
Release App token, never release-tag or PR code under writer credentials.
The writer requires an owner-initialized `release-index` branch protected so that
only the Release App can advance it, without force push or deletion, and runs only
from the main-only release environment. Non-force races re-read
and re-apply against the latest tip, preserving unrelated products and monotonic
version selection, with at most five non-force update attempts. Equal-version
differing identities refuse. Failure leaves the preceding pointer intact and is
reported visibly. An ambiguous readback does not claim rollback of a possibly
completed update. The top-level `release-index` job in `native-release.yml` re-verifies into its own
download directory before obtaining the App token, then the writer rechecks those
same bytes; no flag file substitutes for verification. Reusable workflows do not
read the release environment App secrets. Smoke and the Project reconcile dispatch
complete before this job; a failed index write leaves the release published and
the overall run red. When index alone fails, owner recovery reruns only that job: lower/equal-identical
versions are no-ops, while a higher verified version advances. Historical bootstrap
separately performs the existing immutable/tag/attestation/digest gates; it does not
pass a recordless release through the new full verifier or mutate old assets.

## Client and trust boundary

The client uses one compile-time index base constant, with injection only
through its existing test fixture seam, never environment/configuration override.
Network and HTTP errors name the failing host; oversize, malformed, unsupported,
404/5xx and digest failures do not change installed state. Redirects remain within
the reviewed GitHub download hosts, including the index's raw host. Normal release
paths never fall back to `api.github.com`; PR channels are a separate contract.
A pointer below the installed version is stale and never installed; existing
explicit-version downgrade rules remain authoritative. One pointer read per
product per command suffices; this layout is not a shared aggregate index.

Trust remains HTTPS to GitHub origins plus mandatory sizes and SHA-256. It is not
a new client-side attestation or origin-compromise defense. Binaries published
before the client change still use the API for their first upgrade.
