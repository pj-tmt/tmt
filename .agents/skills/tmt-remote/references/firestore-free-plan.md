# Firestore free plan: limits and budget derivation

Layer 1 (page sharing and collaboration) must run on the Firebase no-cost (Spark) plan. The
guard code is `src/firestore_budget.rs`; its vectors come from the independent script
`tests/fixtures/firestore_budget/reference.py`. The semantics (thresholds, outcomes, error
code) are in the [contract](../../../../contracts/remote-channel-v1.md#proposal-free-plan-budget-guard).
This page holds the dated numbers: recheck them before relying on them, they change.
`tmt remote status --budget` shows the Firestore rows of the table below from `firestore_limits.rs`; change both together when you recheck.

## Official limits (read 2026-10-09; pages last updated 2026-10-07 UTC)

| Limit (no cost)                                            | Value                                                                                                                                     | Source                                                        |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Firestore reads, writes, deletes per day                   | 50,000 / 20,000 / 20,000                                                                                                                  | [quotas](https://firebase.google.com/docs/firestore/quotas)   |
| Stored data; egress                                        | 1 GiB; 10 GiB per month                                                                                                                   | quotas                                                        |
| Reset                                                      | around midnight Pacific                                                                                                                   | quotas                                                        |
| Free databases per project                                 | exactly one                                                                                                                               | quotas                                                        |
| Composite indexes; single-field index configs (no billing) | 200; 200 (1000; 1000 with billing, re-read 2026-10-09)                                                                                    | quotas                                                        |
| Document size; Rules `get`/`exists` per request            | 1 MiB; 10 (20 batched)                                                                                                                    | quotas                                                        |
| TTL deletes                                                | need billing                                                                                                                              | quotas                                                        |
| Listener reads                                             | one per document added or updated in the result, one per document leaving it because it changed, a new-query charge after >30 min offline | [pricing](https://firebase.google.com/docs/firestore/pricing) |
| Rules lookups                                              | one read per dependent document per request, charged again for listeners "each time the query results are updated"                        | pricing                                                       |
| Hosting transfer; Hosting storage                          | 360 MB per day; 10 GB                                                                                                                     | [pricing](https://firebase.google.com/pricing)                |
| Auth daily active users (Tier 1)                           | 3,000 per day                                                                                                                             | [auth limits](https://firebase.google.com/docs/auth/limits)   |
| Cloud Functions, Cloud Run                                 | Blaze only                                                                                                                                | firebase pricing                                              |

Not verified: whether a client's own local echo is billed, whether denied requests bill the
lookups, writes per second per document, any concurrent-listener cap (none is stated), which
sign-in methods Auth counts as Tier 1. The pricing page also lists an Enterprise edition on
Spark with different units (write, read and real-time update units): the deploy must create the
Standard edition. Quotas are project-wide: every page, user and attachment shares one day.

## Model (one page; N members with it open, W of them writers)

Per append the project pays the writer's Rules lookups (3: member projection or link enrollment,
page head, and `ext.getAfter` on the head for the create-only sequence) plus, for each of the
other N-1 listeners, the delivered document and their lookups again (2). The guard uses this
pessimistic figure; the optimistic column assumes dependents are cached.

| N   | Reads per append (guard) | Appends/day (guard) | Appends/day (optimistic) | Min flush interval at 70% of 2 active h |
| --- | ------------------------ | ------------------- | ------------------------ | --------------------------------------- |
| 1   | 3                        | 16,666              | 16,666                   | 0.6 s                                   |
| 2   | 6                        | 8,333               | 12,500                   | 1.2 s                                   |
| 5   | 15                       | 3,333               | 7,142                    | 3.1 s                                   |
| 10  | 30                       | 1,666               | 4,166                    | 6.2 s                                   |
| 25  | 75                       | 666                 | 1,851                    | 15.5 s                                  |

The model covers one page and its appends only; the pool is project-wide, so several active pages, attachment chunks (#2165) and deletes are not in the guard, and the provider's `resource-exhausted` answer is the backstop for their sum. A page the free plan cannot host (its share is too small to allow one append, about 8,333 members with one writer, or 91 when every member writes) is refused when the model is built. Reads bind long before the 20,000 writes. Colab's 10 appends per second per device is only an
upper bound: one device could spend the whole day in minutes, so the extension coalesces local
updates into one sealed update per flush interval. A reconnect after 30 minutes re-reads the checkpoint and the retained tail, at most 200 updates after the checkpoints that a write accepts (`colab-v1.md`, default limits). Presence is off in layer 1 (Colab, v1). Attachments are
chunk documents: a 12 MiB payload is 24 documents at about 512 KiB (384 at 32 KiB); the chunk size
is a #2165 parameter. Colab's page budget is 5,000,000 bytes gzipped and its write tail 4 MiB (`colab-v1.md`, default limits); storage per page follows from those, and attachments count fully.

## Storage without physical TTL

Expiry by Rules only denies reads of expired data; it frees nothing. Removal is a client delete:
compaction by a writer (folded updates), and the owner's clients or bridge deleting expired pages
and attachments, all inside the 20,000-delete pool. Remote owns the arithmetic and the refusal;
the extension owns the policy (when to compact, what to delete first). Data of a page whose owner
never returns stays until storage fills; Blaze TTL is optional and never required.

## Where the guard runs

Remote owns the table and arithmetic (Rust here, the same vectors in the browser SDK);
the extension's transport calls it before each append and persists the client's own count per
Pacific day. The Firestore emulator does not enforce quotas, so tests assert the guard against
the vectors and the Rules path, never Google's counter.

## Recorded readiness

Running `tmt remote status --layers` reads the private deployment record once without the
writer lock or a provider call. It reports recorded project/sign-in/Rules outcomes; a partial
Rules attempt withdraws the old usable binding. Missing/draft records have no evidence,
while damaged records report unknown and are never reset. Tier and quota remain unknown:
the machine cannot observe them, and layer-1 traffic goes browser to Firestore. A complete
recorded deployment therefore does not make sharing read enabled; a later readiness decision
must supply the tier treatment and quota source. The deployed-artifact emulator fixture
proves Rules admission only, not Firebase provisioning or free-plan capacity.
