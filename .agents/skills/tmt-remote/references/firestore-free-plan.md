# Firestore free plan: limits and budget derivation

Layer 1 (page sharing and collaboration) must run on the Firebase no-cost (Spark) plan. The
guard code is `src/firestore_budget.rs`; its vectors come from the independent script
`tests/fixtures/firestore_budget/reference.py`. The semantics (thresholds, outcomes, error
code) are in the [contract](../../../../contracts/remote-channel-v1.md#free-plan-budget-guard).
This page holds the dated numbers: recheck them before relying on them, they change.
`tmt remote status --budget` and the settings page show the Firestore rows of the table below from `firestore_limits.rs`; change both together when you recheck.

## Official limits (read 2026-10-09; pages last updated 2026-10-07 UTC)

| Limit (no cost)                                            | Value                                                                                                                                     | Source                                                        |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| Firestore reads, writes, deletes per day                   | 50,000 / 20,000 / 20,000                                                                                                                  | [quotas](https://firebase.google.com/docs/firestore/quotas)   |
| Stored data; egress                                        | 1 GiB; 10 GiB per month                                                                                                                   | quotas                                                        |
| Reset                                                      | around midnight Pacific                                                                                                                   | quotas                                                        |
| Free databases per project                                 | exactly one                                                                                                                               | quotas                                                        |
| Composite indexes; single-field index configs (no billing) | 200; 200 (1000; 1000 with billing, re-read 2026-10-14)                                                                                    | quotas                                                        |
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

The lookup count is derived from Colab's deployed [log-create Rules](../../../../extensions/tmt-colab/firestore/admission.rules), not provider billing measurements. A non-first append by a non-owner member reads four distinct dependent documents: page, member, space owner and predecessor (`ext.getAfter`). An owner skips the predecessor (three); a first append can skip continuity reads; a continuation after another uid of the same editor link also reads that writer's member row (five). The guard models every writer as the four-lookup member and does not count that fifth read; it is advisory and its listener term is pessimistic. Each other N-1 listener still pays the delivered document plus two Rules lookups. The optimistic column assumes listener dependents are cached.

| N   | Reads per append (guard) | Appends/day (guard) | Appends/day (optimistic) | Min flush interval at 70% of 2 active h                                                                                                                                                                                                                                                                              |
| --- | ------------------------ | ------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | 4                        | 12,500              | 12,500                   | 0.8 s                                                                                                                                                                                                                                                                                                                |
| 2   | 7                        | 7,142               | 10,000                   | 1.4 s                                                                                                                                                                                                                                                                                                                |
| 4   | 13                       | 3,846               | 7,142                    | 2.7 s; native production assumes N=W=4 when membership is unknown (`NATIVE_DEFAULT_MODEL`): writer share 961, warning 672, refusal 864. This advisory guard can exhaust project quota before warning with more than four people; provider `resource-exhausted` remains the backstop (writes unknown, reads refused). |
| 5   | 16                       | 3,125               | 6,250                    | 3.3 s                                                                                                                                                                                                                                                                                                                |
| 10  | 31                       | 1,612               | 3,846                    | 6.4 s                                                                                                                                                                                                                                                                                                                |
| 25  | 76                       | 657                 | 1,785                    | 15.7 s                                                                                                                                                                                                                                                                                                               |

The model covers one page and its appends only; the pool is project-wide, so several active pages, attachment chunks (#2165) and deletes are not in the guard, and the provider's `resource-exhausted` answer is the backstop for their sum. A page the free plan cannot host (its share is too small to allow one append, about 8,333 members with one writer, or 91 when every member writes) is refused when the model is built. Reads bind long before the 20,000 writes. Colab's 10 appends per second per device is only an
upper bound: one device could spend the whole day in minutes, so the extension coalesces local
updates into one sealed update per flush interval. A reconnect after 30 minutes re-reads the checkpoint and the retained tail, at most 200 updates after the checkpoints that a write accepts (`colab-v1.md`, default limits). Presence is off in layer 1 (Colab, v1). Attachments are
chunk documents with a [declared per-resource size](../../../../contracts/remote-channel-v1.md#proposed-cloud-object-profile); the payload-only examples below exclude project-wide traffic. Colab's page budget is 5,000,000 bytes gzipped and its write tail 4 MiB (`colab-v1.md`, default limits); storage per page follows from those, and attachments count fully.

## Storage without physical TTL

Expiry by Rules only denies reads of expired data; it frees nothing. Removal is a client delete:
compaction by an authorized writer (folded updates), or owner deletion of pages and
attachments, all inside the 20,000-delete pool. No Remote cleanup runner or physical TTL
setting ships on Spark; Rules expiry does not free storage. Remote owns the arithmetic and the refusal;
the extension owns the policy (when to compact, what to delete first). Data of a page whose owner
never returns stays until storage fills; Blaze TTL is optional and never required.

## Where the guard runs

Remote owns the table and arithmetic (Rust here, the same vectors in the browser SDK);
the extension's transport calls it before each append and persists the client's own count per
Pacific day. The Firestore emulator does not enforce quotas, so tests assert the guard against
the vectors and the Rules path, never Google's counter. The emulator suite also drives the
shipped SDK bundle's guard through real appends: it refuses below the modeled allowance, the
emulator stores exactly the allowed appends, and the refused update is sent unchanged after the
Pacific reset.

## Recorded readiness

The [status-layer contract](../../../../contracts/remote-channel-v1.md#local-cli-discovery) owns record projection,
index/legacy/partial states and requirement satisfaction by the shipped guard, not observed
billing or headroom. The [owner runbook](firestore-owner-runbook.md#4-check-the-recorded-result)
owns inspection commands; status never refreshes provider evidence. The deployed-artifact
emulator fixture proves Rules admission only, not provisioning or capacity.

## Provider deployment

The `deploy_firestore` adapter checks live plus missing planned single-field configurations and
composite indexes against their separate 200-entry free-plan ceilings, including unrelated indexes.
It preserves existing indexes and TTL policies; bounded inventory failure refuses allocation.
Declaration-derived stream composites, index merging for discovery/page interests, digest binding and
READY-only verification are owned by the [composition contract](../../../../contracts/remote-channel-v1.md#shipped-layer-1-composition).
A building index leaves deployment partial; rerun the same authorized command to observe it, not recreate it.
A lost mutation reply stays unknown. These checks neither measure daily traffic nor prove the project's tier.
Installed Colab acceptance and real-project proof still require separate owner authorization.

The settings page shows these published allowances under native Details, including the
source-read date and 70/90 guard thresholds. They are not a measured usage percentage or
quota-headroom source, so no threshold-crossing warning is inferred from this table. The
SDK validates the dated member against the same checked limits fixture; received values
and date, rather than a local fallback, are displayed.

## Proposed object payload cost

The [sealed-document contract](../../../../contracts/remote-channel-v1.md#proposed-cloud-object-profile)
owns the chunk profile. A 12 MiB ciphertext payload takes 48 chunk writes at 256 KiB
or 384 at 32 KiB, plus a final manifest write. Against 20,000 writes/day, payload-only
ideals are 416 and 52 such objects; manifest writes reduce those to 408 and 51.
These are not available headroom: Rules lookups, recovery, other pages, deletes,
listeners and storage also consume the shared project budget. No per-user/per-space
counter companions or preparation reads are part of a sealed-document append: it
is one write plus its resource's Rules lookups; the continuing member log append above uses four (five after another link-mate). The shipped modeled
page-share guard remains a guard, not a hint or aggregate project enforcement.
Native polling/read costs are in [door and discovery](door-and-discovery.md#native-firestore-sign-in-and-transport); this payload profile adds none.
