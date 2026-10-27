-- A receipt and its capture commit together. Keep receipts while queue replays
-- remain possible; they contain no final text and survive input-batch pruning.
CREATE TABLE hook_capture_receipts (
 event_id TEXT PRIMARY KEY NOT NULL,
 identity_id TEXT,
 batch_id TEXT,
 outcome TEXT NOT NULL CHECK(outcome IN ('committed','duplicate','not_applicable','no_admitted_start','retired_identity','missing_batch')),
 committed_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX hook_capture_receipts_retention ON hook_capture_receipts(committed_ms, event_id);
