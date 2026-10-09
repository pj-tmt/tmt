# Firestore free plan: limits and budget derivation

Layer 1 (page sharing and collaboration) must run on the Firebase no-cost (Spark) plan. The
guard code is `src/firestore_budget.rs`; its vectors come from the independent script
`tests/fixtures/firestore_budget/reference.py`. The semantics (thresholds, outcomes, error
code) are in the [contract](../../../../contracts/remote-channel-v1.md#proposal-free-plan-budget-guard).
This page holds the dated numbers: recheck them before relying on them, they change.

## Official limits (read 2026-10-09; pages last updated 2026-10-07 UTC)

| Limit (no cost)                                            | Value                                                                                                                                     | Source                                                        |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Firestore reads, writes, deletes per day                   | 50,000 / 20,000 / 20,000                                                                                                                  | [quotas](https://firebase.google.com/docs/firestore/quotas)   |
| Stored data; egress                                        | 1 GiB; 10 GiB per month                                                                                                                   | quotas                                                        |
| Reset                                                      | around midnight Pacific                                                                                                                   | quotas                                                        |
| Free databases per project                                 | exactly one                                                                                                                               | quotas                                                        |
| Composite indexes; single-field index configs (no billing) | 200; 200                                                                                                                                  | quotas                                                        |
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

Reads bind long before the 20,000 writes. Colab's 10 appends per second per device is only an
upper bound: one device could spend the whole day in minutes, so the extension coalesces local
updates into one sealed update per flush interval. A reconnect after 30 minutes re-reads the
checkpoint and tail (about 211 documents). Presence is off in layer 1 (Colab, v1). Attachments are
chunk documents: a 12 MiB payload is 24 documents at about 512 KiB (384 at 32 KiB); the chunk size
is a #2165 parameter. Per page, steady state is about 10 MB at the checkpoint and tail budgets.

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
