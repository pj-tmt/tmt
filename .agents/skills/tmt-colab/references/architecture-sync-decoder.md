# Sync, decoder, page source and export

Source: `extensions/tmt-colab/rust/tmt-colab/src` and `typescript/app/src`. Grammar, budgets
and failure codes are in [colab-v1](../../../../extensions/tmt-colab/contracts/colab-v1.md)
and `limits.rs`; do not restate them.

## Stream sync

- `sync::Server` is the opaque transport: append admission and bounded live subscriber
  queues behind an already-upgraded duplex stream. `Connection::poll` is driven externally
  over nonblocking `Read + Write`; the socket worker owns readiness, timers and shutdown,
  so the server creates no listener, runtime or thread.
- The caller implements `Admission` from its verified owner log and device chains (live
  page, role, namespace, revision, expiry, epoch). The server checks exact model
  envelope, header, hash and signature bindings, then calls the create-only `Store`. It
  never decrypts and never calls the decoder. An exact retry returns the same receipt
  without a rebroadcast; awareness is ephemeral.
- Overflow, an unknown or pruned cursor and any abnormal close end in `RESYNC_REQUIRED`:
  clients reconstruct from a fresh verified catchup. Bytes already written cannot be
  recalled, so a stalled write drops the stream instead of flushing ciphertext.
- Catchup pins the retained owner head (`Store::owner_head`), pages membership statements,
  then scoped baseline, wraps and the latest paired checkpoints before the merged
  namespace tails. The final page and the live subscription commit under the server lock
  so appends during paging are not missed. Large objects use the lazy chunk transfer
  (`sync/wire.rs`), one object per page; partial transfers publish nothing.
- Browser side: `admission.ts` admits statements and envelopes, `fold.worker.ts` folds, and
  `writer.ts` persists exact ciphertext before sending and retries those stored bytes,
  never resealing. One lifetime Web Lock owns each device stream; other tabs relay updates.
  Own publication completes only after the submitting tab's connection has admitted and
  folded its records. A relay acknowledgement alone cannot authorize dependent discussion
  or Ask reads; a bounded catchup failure ends the connection without another send.

## Isolated decoder

- `decoder::Decoder` is one caller-owned child runner per page and the only Yjs consumer.
  `decoder/child.rs` is the sole production `yrs` importer (pinned `=0.28.0` in the
  workspace; the architecture test enforces the import rule). The parent never parses Yjs
  bytes. The hidden `__decoder` entry runs before CLI and data-root routing, and the child
  is launched through `tmt-invoke` with an empty environment allowlist.
- The child checks namespace roots, materialized types and projection bounds, and merges
  only the supplied author updates. It also builds and verifies baselines from exact source
  and title. The parent checks input/output binding and hashes, then strict typed Ask and
  discussion record grammar (`ask.rs`, `threads.rs`).
- Limits are enforced before decoding and before returning output. Linux sets
  `RLIMIT_AS` in the child before reading input (a failed `setrlimit` rejects the job);
  other platforms, macOS included, report `memory limit unavailable`. This is crash and
  resource containment with the user's filesystem authority, not a sandbox.
- A runner is reusable only after `Cleanup::NotStarted` or `Cleanup::Confirmed`; any state
  where a child may survive blocks it. Invalid output, panic or timeout returns no
  application result. `decoder::Config` carries the program and deadline; only tests inject
  a larger deadline (`tests/support`), and no option tunes the production deadline.
  Invocation failures also emit a best-effort ASCII stderr record of at most 512 bytes,
  containing only the private call phase, input size, remaining runner-entry budget,
  invocation elapsed time, including cleanup, failure kind and cleanup category; original errors are unchanged.
  Decode-request failures also include parent wire-construction, JSON-serialization and input-hash
  wall intervals; JSON serialization includes incremental hashing, and the hash interval includes
  the existing stream guard plus digest finalization and encoding. Other commands omit these
  fields. The samples consume the original deadline.
  The record bounds bytes, not write latency, and does not measure CPU, scheduling or child phases.

- `Decoder::prepare_content_batch` uses the private `__decoder prepare-content` entry to return
  an explicit no-op or ordered causal update batch. The child replays the batch from the supplied
  admitted base and verifies the expected content projection; the parent checks input/output
  correlation, strict shape, bounds and expected metadata without parsing Yjs. The batch is
  preparation only; `tmt colab page write` publishes it through `page::prepare_publication`.

## Page source and export

- `page.rs` reads and writes admitted source locally: it prepares through the fold and
  decoder, and the opaque token binds the owner head, page epoch and every namespace
  position, because content appends do not advance the membership log.
- `page::prepare_publication` uses one admitted `fold::Snapshot` and its extracted
  materialization input owner for exact base/full metadata/own projections and causal decoder inputs.
  It returns Noop before ID/sequence/seal/certificate work, or a frozen signed content packet and
  chain through the existing local Keyring writer. Both single-edit and batch gzip admission include
  all own bytes in the checked raw fastpath. `page write` and the browser Save (`page/save.rs`) are its callers.
- `page/compact.rs` combines the local device's own stream after a write (best effort, repeatable):
  it opens only that stream's objects through `Snapshot::open_object`, merges them in the decoder
  child (`Decoder::merge`, no projection, since one device's stream can depend on another's structs),
  compares the full page's HTML, metadata and each own projection before and after substitution,
  then seals a checkpoint per namespace at the stream head and publishes through `Store::checkpoint`,
  which prunes the covered prefix once the namespaces are paired. Write limits count only the updates
  after checkpoints; the page budget (gzipped, `fold::gzip_over_budget`) applies to larger states.
- CLI create/write obtains an optional caller display label from the fixed public
  `identity show --json` command in `core.rs`; failures leave no label. The decoder's
  `ContentEdit` replaces/clears `meta.publisherAgent` atomically with source. Browser
  edits preserve it, and owner epoch baselines carry it in their committed update.
  No label selects an identity or grant. Creation reuses that single bounded identity
  snapshot and one optional same-root public machine-status observation to freeze
  `decoder::CreationRecipient`; the browser projection uses the canonical
  `CreationRecipient` in `fold-protocol.ts`. Strict metadata admission, source-edit
  before/after checks, fresh/chunked baselines, epoch, checkpoint/history and frozen
  export preserve the pair. Source edits cannot set it, and absence never backfills.
  There is no second acquisition, descriptor cache, service startup, private Remote
  state, HTTP or Session lookup. The [Colab contract](../../../../extensions/tmt-colab/contracts/colab-v1.md)
  owns the complete-pair grammar and strict-reader compatibility rules; the future
  #1817 UI default is not integrated by this native/non-UI boundary.
- `page write` freezes one batch (`prepare_publication`). Offline it holds the serve lifecycle
  lock, uses `Store::write_existing` and `commit_publication`; when `serve` holds the lock,
  `page/ipc.rs` posts one `LocalWrite` v2 to `/.tmt/colab/local/page-publish` and never
  retries, re-signs or falls back. `ipc::send` and `ipc::receive` are split so a failure
  before the body is fully written is a plain refusal, while any later doubt is resolved
  by one read-only `publication_status` in `main.rs::publish_write`, else
  `COLAB_OUTCOME_UNKNOWN` with the original operation ID. `Server::publish` prepares one
  broadcast per entry before the transaction and fans out only for a new committed
  outcome, then combines the own tail (`Trigger::until` = request read time plus `PUBLISH_COMBINE`; a later combine publishes
  nothing, so the page does not move after the reply the client waits for, and the reply
  carries the revision read under the sync lock). The write signs with a purpose-separated local device certified by the
  management member; it is not a Remote registration.
- Browser Save is a sync-socket request (`save`/`savestatus`, colab-v1 Browser Save), owned by
  `sync.rs`: `process` assembles and authorizes the request under the sync lock and hands back a
  `SaveJob`; `page::save::prepare` then opens its own read-only store, keyring and decoder through the
  `SourceOpener` that `serve` supplies (`Registration::with_save_source`) and prepares with the sync
  lock released (`prepare_publication_with_clock`, shared with `page write`, reads the clock once after
  its page snapshot is taken, so a certificate another writer issued before that snapshot is never in
  the preparation's future), so a large save never stalls other peers; `finish_save` re-takes the lock only for
  `commit_publication` (through `Admission::commit_save` and the same combine as a CLI write), the one
  `saveresult` and the shared fan-out. The browser's `Connection.save` paces the chunks and holds one
  request in flight; `Live.edit` settles a lost reply with one `savestatus` and never resends.
- `export.rs` snapshots exact source and title through `fold::Snapshot` and the decoder and
  writes `page.html`, `conversations.json`, `conversations.md` and `manifest.json` (format and
  disclosure: colab-v1). The fold now keeps each writer's decoded `own` projection and
  historical signing key in `View`; `export/conversations.rs` projects threads, comments and
  verified Asks from them and renders both conversation files. It adds no HTTP route, signing
  capability or core dependency. The browser equivalent is `export.ts` and `conversations.ts`
  plus the trusted-parent `export-panel.tsx`, built from a committed projection (`own` and
  `ownSigningKey`) and verified head. All serializers are pinned by
  `contracts/vectors/export-v1.json`, generated by the independent
  `contracts/vectors/export-reference.py`.

- `publication.rs` owns the pure `Manifest`/`SignedJob` (`kind` `content` or `own`, one
  namespace per job), original-ID `Outcome` and `LocalWrite`/`LocalStatus` codecs, with exact
  packet/hash/signature vectors for both kinds in `publication-content-v1.json`. It accepts a
  caller-admitted public key, never authority state, and preserves original envelope bytes.
  See the content-publication section of colab-v1 for its callers: CLI `page write` and
  `threads resolve|reopen`; Browser Save does not use it.

## Browser containment

- The fold runs in a dedicated Worker, with bounds on state and projection; the parent checks every projection and terminates the Worker on failure or
  deadline. No key or transport capability enters it. Treat it as resource containment.
- Parent chrome policy and renderer policy are two constants in `assets.rs` (`POLICY`,
  `RENDERER_POLICY`); change them only with the colab-v1 renderer section and the
  `renderer.spec.ts` browser checks.
- The browser Worker produces no content updates: Save is native (Page source and export). `Fold.prepareContent`
  (the private `prepare-content` Worker command) remains for tests with no production caller. It returns an explicit
  no-op or bounded ordered content deltas with the expected projection. Preparation leaves
  committed Worker state unchanged, and the parent validates the typed result against its
  admitted base. This interface has no signing or transport capability.
