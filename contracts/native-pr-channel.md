# Native PR channels

The native installer owns `pr<N>` channel selection, acquisition and receipts.
`N` is canonical positive decimal, without leading zeroes, at most 2147483647.
`tmt upgrade --channel pr234` explicitly selects that channel; subsequent upgrades
retain it. Stable/alpha discovery never selects a PR. Existing exact-version pins,
ordinary pinned no-network behavior, unpinning and SemVer downgrade guards apply.
The executable keeps its archived version. PR identity is `(pr, headSha, runId)`;
a changed candidate on the same PR requires a newer run. A different candidate
at the same archived version refuses, even for a newer run. Equal-version bytes
may change only on an explicit transition to a different channel. No synthetic
PR version or build-metadata ordering is introduced.

Target-aware online installation uses this same resolver. Published-release
metadata discovery refuses a PR channel instead of searching ordinary tags.
An unknown compiled producer authority reports `NATIVE_PR_CHANNEL_UNAVAILABLE`;
missing credentials report `NATIVE_PR_AUTH_REQUIRED`. Unknown schema, schema
ahead without consent and older-than-local schema at parent admission report
`NATIVE_SCHEMA_UNKNOWN`, `NATIVE_SCHEMA_AHEAD` and `NATIVE_SCHEMA_DOWNGRADE`,
respectively. A later child refusal retains its bounded protocol error message.
Repair refuses
a PR source until authenticated reacquisition is available; it cannot substitute
a published release or unproven local archive.

## Acquisition and eligibility

Only same-repository, open PRs in `pj-tmt/tmt` are eligible. The exact `rc-build`
label must be present with the same numerical label ID as the catalog. A complete
one-page issue timeline, at most 100 events, must prove a unique latest UTC-second
`labeled` transition for that label; its ID/time must match the catalog's enable
epoch. Latest removal, remove/re-add, label recreation, ties, missing evidence,
denied requests or pagination refuse. Manual dispatch produces one generation
only after the operator explicitly adds the label; the producer never adds it.
Disabling means removing the label. The installer never dispatches or edits labels.

Authenticated Actions acquisition reuses `GITHUB_TOKEN` or `gh auth token` for
github.com during this invocation. No credential or signed download URL enters
a receipt, log or diagnostic. Fixed official API routes select the exact named
`tmt-pr-rc-catalog-v2-pr<N>` inventory. Discovery is bounded to 32 metadata
requests, 2 MiB cumulative metadata, eight producer generations, and the existing
60-second acquisition deadline. Incomplete or ambiguous discovery refuses.

Producer trust is an installer-compiled, externally reviewed tuple of workflow
ID/path, workflow SHA-256 and tooling commit. The API run must use that tuple on
main, workflow_dispatch, completed/success, and the current attempt. Its tooling
commit is distinct from the candidate PR head. The exact workflow blob is checked
against that approval. Candidate JSON, successful unrelated CI and latest main
cannot grant trust. The newest trusted generation alone is considered; invalid,
expired, pending or missing current-head data never falls back to an older one.

Catalog JSON rejects duplicate/unknown keys, nulls and wrong scalar types, with
a 1 MiB raw limit and depth 12. A catalog ZIP contains one root regular file,
catalog.json, and is at most 2 MiB. Each payload ZIP is at most 69 MiB and has
exactly two root regular files: dist-manifest.json and the native tar.gz. Exact
API ZIP size/digest, raw member size/digest, and existing cargo-dist
product/target/version/inventory rules all agree. No generic ZIP extraction,
links, duplicate names, special entries or extra files are admitted. Native
manifest/archive/expanded bounds remain 4/64/128 MiB.

## Application-schema authority

The candidate catalog and its manifest's `tmt_application_schema` must contain
the same application-schema v1 record: schema_version, product, source_sha,
sorted unique databases (`domain`, `version`) and sorted unique source_files
(`path`, `sha256`). Records contain at most eight known domains and 64 source
files. Paths are canonical repository-relative ASCII components, at most 256
bytes; digests are lowercase SHA-256. This is application schema, independent
of executable SemVer and the receipt format version.

Candidate evidence, the latest immutable published alpha manifest for the same
product, and the actual local database owner must all supply known values.
An absent database or metadata field is unknown, never assumed zero. A candidate
above alpha refuses unless this invocation explicitly supplies
`--allow-schema-ahead`, with a warning that alpha return may be unavailable until
alpha catches up. Consent never permits local data downgrade. Local schema above
the candidate always refuses; return to a published channel also requires known
manifest evidence at least as new as local data. Installation does not migrate,
initialize, checkpoint or delete application data.

CLI requires exactly `tmt-core-db`. Its local observer opens SQLite read-only and
validates the authoritative contiguous `_migrations` ledger and known prefix;
user_version is not its version. Unknown/future local versions remain observations
that can refuse an older candidate. Extension domain sets and local observers
require their owners' reviewed exports before live PR installs; unsupported
coverage refuses rather than inferring another product's schema.

### Compiled CLI exporter

The internal, model-free `__native-schema --source-sha <40 lowercase hex> --json`
prints one compact UTF-8 JSON application-schema v1 object followed by newline,
with empty stderr on success. It needs no database, configuration initialization,
network or credentials. The version comes from the compiled typed migration
owner. The complete declared source closure includes migrations.rs, every SQL
input of that table, migrations/host_names.rs and schema/006_indexes.sql. Paths
and hashes come from compiled bytes, not files beside the caller. Adding/changing
migration inputs must update this closure and its completeness controls together.

The source-sha argument is descriptive, never evidence of which commit compiled
the executable. The unprivileged preparation owner invokes the exact prepared
binary and checks every exported path/hash and domain/version against its captured
source cut. Version injection uses the existing exact tracked-source snapshot
contract: only the approved package version and implied local lock changes are
allowed. Retain prepared source, binary and output digests. Final matching-host
verification independently re-exports each extracted executable and compares the
entire record to the final manifest. The trusted publisher consumes verified
evidence; it must not execute PR code with catalog or cleanup write credentials.

## Admission, receipt and activation

After acquisition, take one fresh bounded admission snapshot: open/current-head
PR, current labels and complete enable timeline, complete named catalog inventory,
current producer run/attempt, exact catalog/payload metadata, latest-alpha manifest
and actual local schema. Changes refuse without retry or reselection. The receipt
retains exact PR/run/producer/catalog/payload identities, enable epoch, initial
pull/timeline hashes, fresh API-response hashes, alpha evidence, local domain
values and invocation schema consent. Tokens and redirect URLs are excluded.
The existing expected-current UUID fence and installation lock protect local
publication; local schema is checked again before publication. GitHub and SQLite
are not a distributed transaction. Later close, disable or expiry leaves installed
bytes and channel intent intact and makes further acquisition unavailable.
Historical receipt reads judge the captured admission, not today's remote state.

CLI PR installation and return use separately probed handoff version 2. Probe
success is exactly `{"protocol":2}`, exit 0, empty stderr. Input/output are bounded
to 64 KiB; existing 5-second input, 10-second probe and 60-second child bounds
remain. The request has protocol=2, `transfer` (the protocol-1 byte-transfer
descriptor), typed `provenance` and Boolean `explicit_channel`. PR transfer has
release_id=0; a published release retains its positive ID. Provenance is a
strict tagged record (`kind`, `evidence`): github-actions carries PR evidence;
github-release carries release ID and manifest digest. The candidate verifies
the manifest schema against its own compiled export and uses the existing
inventory, publisher, rollback, skills and finalization owners. Protocol-1
requests, responses and normal release receipts remain unchanged. Unsupported
candidates refuse safely; no old-process installer fallback is used.

The native protocol fixture archives its actual development executable under
its unchanged version on a PR channel. It tests the child, receipt and read-only
database fences; it grants no producer trust or released-archive qualification.
The existing synthetic-alpha fixture requirements still govern stable/alpha
installation proofs.

## Bootstrap dependencies

The production producer approval set is empty until an actual immutable publisher
implementation and its inputs receive shared review. Synthetic fixture IDs never
become live approvals. A published schema-bearing alpha and reviewed product-owner
exports are independent prerequisites; absent evidence refuses. The channel
implementation does not build candidates, publish an alpha, mutate immutable
releases, install globally or restart live agents to manufacture those inputs.
