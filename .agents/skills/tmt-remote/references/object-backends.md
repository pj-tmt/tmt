# Object backends

Follow [tmt-dev](../../tmt-dev/SKILL.md) and the
[Remote architecture](../../../../ARCHITECTURE.md#remote-extension-pilot). The
[storage proposal](../../../../extensions/tmt-colab/contracts/storage-v1-proposal.md)
owns the accepted behavior and bounds; this guide owns how the implemented backend
in `extensions/tmt-remote/rust/tmt-remote/src/objects*` works and how to add an
adapter. Nothing here is reachable from a route, the SDK or a setting, and no
production extension declares object storage (`mount::ObjectDeclaration`, disabled for
every entry of `mount::EXTENSIONS`): the lease-bound `object_service` below is library
code that tests exercise; the routed channel and consumers belong to #1852 and later
children.

## Modules

| Path                                                           | Owns                                                                                                                                   |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `objects.rs`                                                   | Opaque IDs, `BeginSpec`, `OriginalIntent`, `Receipt`, `TransferState`, `IoBudget`, `BackendError`, `Quotas`, the `ObjectBackend` trait |
| `objects/ledger.rs`                                            | The sole metadata and accounting owner: `<dataRoot>/remote/objects.db`, one short IMMEDIATE transaction per transition                 |
| `objects/tree.rs`                                              | Directory-handle, no-follow traversal of one extension's `<dataRoot>/<extension>/objects/{staging,blobs}` tree                         |
| `objects/local.rs`                                             | `LocalFs` (coordinator) and `LocalHandle` (one extension's backend), in-flight guards, the effect order below                          |
| `src/objects/*/tests.rs`, `tests/objects.rs`, `tests/objects/` | Crash windows and races at named milestones; the reusable conformance suite, an independent in-memory adapter, `LocalFs` safety tests  |

`ObjectService` (`src/object_service.rs`, not under `objects/`) is the one installation
owner above these: it opens `LocalFs` only when a static declaration enables an extension
(`ObjectService::open` refuses readiness when accounting cannot settle and creates
nothing otherwise), keeps at most one active bus and one setup candidate per extension and
eight buses in all, and opens a channel only on an explicit `activate` (no retry, no
polling). `activate` is not called by production code. Its extension names come only from
the declaration list and it names no extension itself.

Origins (`object_service/origins.rs`, shared with `Mounts` through the `mount::OriginSink`
trait, so mounts name neither the service nor the protocol) are the only owner of origin
phases: Pending while an upgrade to a declared extension with an active channel is
forwarded (`tmt-origin` is the id), established only after the owner session attached,
the browser has its 101 and `adopt` ran the tunnel, gone at the first close. A close wins
over a later establish, a channel's end removes its origins and a successor inherits none.
`established`/`closed` notices go through a bounded ordered queue to one announcer thread;
the registry never calls sessions or writes a frame, and a ticket drop never blocks. Nothing
yet answers a mounted origin's request (that is the next commit's session check).

Each running channel has one dispatcher thread, the only reader of the bus, two workers and one announcer.
The dispatcher queues requests and hands callback decisions to the worker that waits for
them, so a decision is never stuck behind a request. A worker answers `config` for a
local-extension origin from a snapshot of the delivered backend taken at `open` (backend
identifier, capabilities and `Quotas`; no usage, ledger row, path or secret): it asks for
`acquire` admission, then for `disclose` admission naming the projection, and refuses on
anything but an allow. A mounted origin is answered `denied` and the other six methods
`unavailable`, with no callback and no effect; they belong to later slices. No decision is
remembered. A request whose 30 s is spent before a callback is sent (queued too long, or used
up between acquire and disclose) is answered `unavailable` with no further callback and the
channel keeps serving. A sent callback unanswered at its bound (5 s, within the request's
remaining time) ends the channel without a result: the ledger refuses a result while a
callback is outstanding.
`shutdown`, `Drop` and replacement end the channel: threads are joined and the last one
drops the bus, which closes the socket. All of this is library code, not routed and not
reachable in production.

`LocalFs` borrows the `Serving` proof, so the lease cannot be released while it or a
handle exists, and `Store::open` and the ledger share only the private connection setup
(`store::open_connection`). The ledger is not `remote.db`: object transactions never
contend with authority writes and the grant migration history is untouched.
`ExtensionId` validates a name only; the trusted installed-extension registration that
admits it is #1852's.

## Semantics an adapter must keep

- **Original identity.** `IntentId` is service-derived (installed extension, actual
  principal, caller UUID). The caller-frozen `BeginSpec` (key, raw SHA-256/length,
  policy binding) is compared on every repeat; changed input is `Conflict`. Adoption
  time and the 24-hour staging deadline are generated once at first adoption and are
  never compared or renewed. Rows are never deleted, and the immutable columns are
  guarded by triggers.
- **Status** is observational: it never adopts, expires, trims, publishes or reconciles.
  Pending, committed, expired, discarded, not-observed, unavailable and unknown stay
  distinct; absence and a deadline never prove that a possible publication did not
  happen. Possible publication (`committing`, `unknown`) is `Unknown`; a removed
  namespace's originals are `Unavailable` with their binding retained. Only `staging`
  ever expires.
- **Parts** are canonical: every part but the last is exactly the capability's chunk
  size, in order. Non-canonical input is `Invalid` before any effect; an exact repeat of
  an acknowledged part compares the stored bytes.
- **Incomplete objects are never readable**; `stat` and `read` match committed originals
  in an open namespace only.
- **Terminal originals** (expired, discarded, deleted) can never become executable
  again. A key is reusable by a different original only after confirmed cleanup, with no
  uncertain publication and in an unfenced namespace.

## Order of effects (LocalFs)

1. `begin`: one transaction adopts the original, reserves its entry and bytes (limits
   checked in fixed order: active, retained, entries, then bytes at namespace,
   extension, installation) and refuses with `Capacity` and no effect.
2. `append`: payload write, file sync, then the checkpoint transaction, then the
   acknowledgment. An unacknowledged tail is cut back at readiness.
3. `commit`: full reread and SHA-256 verification, transaction `staging -> committing`,
   create-only hard link, directory sync, transaction `committed` (the receipt), then
   staging-name removal. A destination that already exists is accepted only when it is
   the original's own inode (device and inode equal to the staging name's, verified
   again on reconciliation); anything else, including identical bytes in another inode,
   closes the original as `unknown` (charged for every distinct body) and is never
   overwritten.
4. `discard` and expiry: `staging -> discarding|expiring`, remove the staging name, sync,
   then release payload and entry. `committing`, `committed` and `unknown` conflict.
5. `remove_namespace`: commit the fence, drain in-flight holders, mark rows `removing`,
   unlink exactly the files the ledger names, remove the directory, sync, then release
   charges in small batches. A foreign entry keeps the namespace charged and unsettled.
6. `LocalFs::open` settles every interrupted original by its original ID before
   readiness. A timeout or cancellation after a durable effect leaves effect, charge and
   state and suppresses the acknowledgment; there is no retry, detached job or timer.

Locks: the ledger mutex and the in-flight mutex are never held together, and neither
is held across file I/O. A namespace fence is committed before it is drained, and a
reader re-checks it after reading and before returning bytes.

## Charge formula

Recomputed from rows, never cached. Per scope: payload (rounded up to 4 KiB until its
removal is confirmed) + `OBJECT_RECORD_BYTES` per retained original + `OBJECT_FENCE_BYTES`
per namespace (its directory block, entry and fence row). Extension and installation scopes
also count `OBJECT_TREE_BASE_BYTES` once per extension that has any namespace (its fixed
directories), and the installation adds `OBJECT_LEDGER_BASE_BYTES`, which covers the empty
ledger and the largest rollback journal. The two staging/final hard links share one payload
charge. Constants live in `src/limits.rs`. Sums, products and conversions are checked: an
unrepresentable aggregate is unavailable accounting, never a wrapped number.

An `unknown` original is the one state whose charge can exceed its reservation. When
publication finds a destination that is not provably this original's own inode (device and
inode of the two admitted handles; equal bytes alone prove nothing), the original is settled
`unknown` and its charge becomes `max(reservation, measured)`, where measured sums the
rounded allocation of each distinct inode it retains (length and allocated blocks, whichever
is larger). The charge is never lowered, nothing is overwritten, unlinked or released, and
it may exceed a limit because the bytes already exist: the ordinary checks then refuse later
adoption in exactly the namespace, extension or installation limit it exceeds, and no other
scope is full. A destination without its staging name is not a recovery (the staging name is
removed only after the receipt). If an allocation cannot be examined (an unadmittable tree, destination or staged name),
measured, represented or recorded (the budget ended first), the
original stays unsettled, every handle of that `LocalFs` refuses new adoption (observation of
existing originals continues), and a restart refuses readiness while the unsettled original or
an unrepresentable aggregate remains. There is no manual resolution command.

The bound is demonstrated, not assumed: the footprint tests build populations (one
namespace per tiny object, block-straddling payloads, many objects in one namespace,
pending staging, discarded/expired/removed tombstones, unknown originals, and
differing, larger-than-intent and identical-but-different-inode destinations) and check
the modeled physical bytes (each distinct inode once, 4 KiB blocks, 4 KiB directories, 128
bytes per entry, real ledger growth) against the charge; the ledger test measures the
largest journal. It is a logical model, not `st_blocks`: a filesystem with larger blocks
may exceed it, and nothing here claims power-loss durability. The differing-destination
cases are created by out-of-band fixture corruption; `LocalFs` itself never creates foreign
bytes, and the tests prove its response, not that such bytes occur.

## Adding an adapter

Implement `ObjectBackend` for one extension scope, sharing one accounting across all
handles of the adapter, and run the shared suite from `tests/objects/conformance.rs`:
implement `Adapter` and `Session` (backend handle, clock advance, restart, installation
usage) and invoke `conformance!`. `tests/objects/memory.rs` is the minimal example. An
adapter that needs a different limit changes `Quotas`, not the scenarios. Consumers
must use only the trait; do not add provider branches.

## Verify

From the repository root, with `CARGO_BUILD_JOBS=2`:

```sh
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote objects)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-remote --test state)
(cd rust && CARGO_BUILD_JOBS=2 cargo clippy --offline --locked -p tmt-remote --all-targets -- -D warnings)
(cd rust && CARGO_BUILD_JOBS=2 cargo test --offline --locked -p tmt-cli --test architecture)
```

The crash-window tests observe named milestones inside the real algorithms (compiled
only for tests) and the SIGKILL probe re-executes the test binary; none uses a sleep to
infer an effect. After changing a guard, perturb it once and confirm a test fails.
