# Digest usage report contract v1

`tmt digest usage report [--since 24h|7d|30d] (--json | --csv usage|exchange)`
joins Core's public consumption, limit and exchange documents into one read for
people and spreadsheets. `--since` defaults to `24h`; exactly one output is chosen.
It is read-only: Digest keeps no usage store, collector or scheduler for it, and it
never changes settings or delivers messages. Every figure comes from Core, so the
report adds no evidence of its own.

## Reads

Digest calls Core only through public commands, under a 60-second command deadline
with Core's usual 15 seconds and 1 MiB per call:

1. `tmt identity ls --json`: the roster. This is the only required read.
2. `tmt x flow --since W --bucket W --json`: one row per sender, recipient and kind
   for the window ([flow contract](../../../contracts/request-response-v1.md#content-free-exchange-flow)).
3. `references.resolve` for flow participants that are not on the roster (names,
   retired).
4. `consumption.history` for the roster plus retired flow participants, ordered by
   name then UUID, in batches of 16, as two reads of `windowsMs:[W]` with
   `includeRetired:true`: `maxBuckets:1` for totals with attribution and
   `maxBuckets` 24 (`24h`, hourly) or 7 / 30 (`7d` / `30d`, daily) for the series.
   Windows above an hour read Core's [30-day rollup](../../../contracts/extension-api.md#thirty-day-rollup),
   so `window.toMs` is the last completed five-minute boundary and up to five minutes
   of the newest activity are not in it. Totals are Core's, never a sum of buckets.

## Document

`--json` prints one document, `{"version":1, ...}`:

- `observedAtMs` (Digest's clock), `since`, and `window` (`{fromMs,toMs}` Core
  reported; null when no identity could be read).
- `usage.seats[]` (identities with rollup evidence): `id`, `name` (null when Core
  could not resolve it), `retired`, `totals` `{input,output,cached}`, `byModel`
  (Core's attribution as `{modelId,input,output,cached}`, or null when Core could
  not attribute the whole total), `series[]` (`{fromMs,toMs,input,output,cached,complete}`)
  and `coverage` `{coveredMs,windowMs,complete,gap}`. Cached input is part of input;
  total is input plus output. An incomplete bucket is a lower bound, not an error.
- `usage.unavailable[]`: `{id,name,retired,reason}` for an identity with no
  evidence: `no_evidence` (found, no successful read in the rollup), `not_found`, or
  `read_failed` (its batch failed or contradicted itself; a warning says why).
  Unavailable is never reported as a seat with zero.
- `usage.lines[]`: totals grouped by model family, from the model id: `claude`
  (`claude-*`), `gpt` (`gpt-*`), `other`, and `unattributed` for totals Core could
  not attribute. `seats` counts identities contributing; `partialSeats` those whose
  coverage is not complete. The family is a presentation label, not a provider claim.
- `limits[]`: account windows, deduplicated by `(windowMinutes, resetsAtMs)` across
  seats and buckets; the newest observation wins and windows are never summed. Each
  has `usedPercent`, `observedAtMs`, `seats` (UUIDs), `stale` (`resetsAtMs` is not
  after `observedAtMs` of the report), `elapsedPercent` (share of the window length
  elapsed at the report's `observedAtMs`, clamped to 0–100, two decimals) and
  `paceDeltaPoints` (`usedPercent - elapsedPercent`; negative is under an even pace).
  A stale window has null `elapsedPercent` and `paceDeltaPoints`.
- `limitsUnavailable[]`: `{line,reason}` for each line whose seats reported no
  limits: `claude` is `no_source` (Core has no Claude limit source) and others are
  `not_observed`.
- `exchange`: `sinceMs`, `observedAtMs`, `pairs[]` (`sender`, `recipient`, `kind`,
  `count`, `bytes`), and `bySender[]` / `byRecipient[]` (`participant`, `count`,
  `bytes`; most messages first, then UUID). A participant is `{id,name,retired}`;
  a null `id` is an unknown sender or recipient. Counts and bytes are content-free
  metadata. Null when the flow could not be read.
- `warnings[]`: `{section,code,message}` for each degraded read.

Only the roster is fatal. A failed flow leaves `exchange` null; unresolved names are
null; a failed or inconsistent history batch makes its identities `read_failed`.
Retired identities have no listing, so one appears only as a flow participant.
Failure exits 1 and prints `{"version":1,"error":{code,message}}` on stdout with `--json`, or a one-line error on stderr with `--csv`.

## CSV

`--csv` prints one RFC 4180 table with a header and LF row ends. Its rows are the
JSON rows, so the two never disagree.

- `usage`: `id,name,retired,line,model,input,output,cached,coveredMs,windowMs,complete,unavailable`.
  One row per model of each seat (`line` from the model id; `unattributed` and an
  empty `model` when `byModel` is null), then one row per unavailable identity with
  only `id,name,retired,unavailable`.
- `exchange`: `senderId,sender,recipientId,recipient,kind,count,bytes`, one row per
  pair; an unknown participant has empty cells. It fails with `EXCHANGE_UNAVAILABLE`
  when the flow could not be read.

A text cell starting with `=`, `+`, `-`, `@`, tab or carriage return gets a leading
`'`, so a spreadsheet never reads a name as a formula. CSV carries no `limits`;
use `--json` for them.

## Vector

The frozen [usage report vector](vectors/usage-report-v1.json) pins the join: Core's
documents in, this document and both tables out, with arithmetic written by hand.
