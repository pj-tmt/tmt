# Discussion modules

The [colab-v1 contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#own-stream-discussion-records-1427)
owns record fields, limits, revision semantics and trust boundaries.

- `thread-records.ts` admits strict inert thread/comment JSON and projects
  consecutive writer-owned revisions with verified page/epoch scope. It uses
  historical envelope keys for display, never current publication authority.
- `thread-store.ts` owns current connection admission and serialized parent
  actions, setting a display-only publisher timestamp at each publication. Thread
  creation batches the opening comment with its thread through
  `Writer.submitOwnRecords`; foreign replies remain in the replying stream.
  `commentForAsk` rechecks origin IDs/revisions before freezing an Ask.
- `fold.worker.ts` prepares immutable bounded own batches without committing;
  `writer.ts` keeps the existing lifetime lock, shared sequence and durable exact
  ciphertext retry. `Live` publishes discussion from its committed own view.
- Native `threads.rs` validates typed record grammar after isolated Yjs decoding.
  It has no DOM, signatures, publication or dispatch responsibility.
- `thread-panel.tsx` owns muted author/time labels with device-ID tooltips, plain-text
  parent controls, draft retention and explicit reattach confirmation. `AskControl`
  captures comment context on opening and retains a prepared excerpt through
  subsequent edits or tombstones.
- `public/renderer.html` installs bounded selection capture and cosmetic quote
  resolution before author HTML; `renderer.ts` binds narrow requests/results to
  the current render and request. CSS Highlights and pointer-inert overlays grant
  no source truth or application capability. Ports/observers clear on teardown.

Verification: `test/thread-records.test.ts` and native `tests/discussion.rs`
consume `contracts/vectors/discussion-v1.json`. `test/own-fold.test.ts` checks
atomic preparation and immutable rejection; `test/live-ask.test.ts` checks real
comment IDs in signed Ask framing. `e2e/renderer.spec.ts` tests bounded DOM quote
capture/resolution and containment. `acceptance/discussion.spec.ts` uses two real
paired browsers, foreign-stream replies, reload/restart, exact explicit Ask effect
counts and desktop/mobile theme screenshots. Use the existing acceptance harness
and [commands](development.md); no separate server or state store is introduced.
