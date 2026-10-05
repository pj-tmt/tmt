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
  `createChat` reserves the writer device UUID for one null-anchor page/epoch thread,
  refusing duplicate creation under the same writer lock. `commentForAsk` rechecks origin IDs/revisions before freezing an Ask.
  `captureConversation` captures prior comment/reply references on explicit Send;
  `conversationForAsk` revalidates that frozen set against admitted records, excluding
  later arrivals and refusing edited/deleted context.
- `fold.worker.ts` prepares immutable bounded own batches without committing;
  `writer.ts` keeps the existing lifetime lock, shared sequence and durable exact
  ciphertext retry. `Live` publishes discussion from its committed own view.
- `thread-status.ts` folds immutable status actions by causal depth and writer/action-ID
  ties after record scope and historical writer-key checks. Legacy `resolved` supplies the
  initial state; missing/cross-thread parents and cycles remain inert. The contract
  linked above owns the grammar and ordering rule. Status actions do not transfer
  creation, anchor, deletion or comment-edit ownership. `ThreadBinding.setStatus` uses a
  captured previous-action reference; publication failures preserve the effective state.
- `thread-status-coordinator.ts` freezes unique mentions and prior signed recipient UUIDs,
  publishes status/operation IDs before sequential notification attempts, and coalesces
  the explicit parent action. `LiveAsk.notifyStatus` reuses FrozenAsk, AskController and
  the existing own ledger; it never runs during loading or result re-checking.
  `thread-status-notification.ts` re-admits the committed action/recipient and captured
  discussion before Ask freezing, then projects held/refused/failure/uncertain outcomes
  without granting dispatch. `thread-status-view.ts` owns the shared status metadata,
  non-Chat open count and browser-local attention cleared only by an explicit parent open.
- Native `threads.rs` validates typed grammar and `threads/status.rs` owns the same causal
  fold. `fold.rs` derives owner-device status provenance from verified historical
  certificate issuers separately from signing keys; non-owner and bridge records
  keep their existing admission but their status actions are inert. A cut-admitted
  owner action remains effective after revocation; new publication is denied.
  `discussion.rs` reuses the authenticated export projection for native reads and
  prepares agent status actions with no recipients or dispatch. The isolated decoder
  prepares only that writer's own structs; `page::publish` shares the existing offline lock
  or root-local socket path, certificate, sequence and ciphertext commit fences with source
  writes. `cli_threads.rs` adapts these operations to `threads`, `resolve` and `reopen`.
  Actor labels and clocks remain display assertions.
- `components/conversation-turn.tsx` owns the shared turn markup, attribution and
  square styling. Its `thread` layout uses User/Bot avatars and an ink agent-body
  rail; its `chat` layout uses sided tinted turns and a bot mark in the agent meta
  line. `CommentExchange` associates admitted asks with their originating comment
  once for both surfaces. Replies have one agent/name/time byline; pending states
  and trusted delivery actions stay in the requester turn until a reply exists
  (including an empty reply), then the status disappears.
- `thread-panel.tsx` owns muted author/time labels with device-ID tooltips, visible Resolve/Reopen/Close thread labels alongside Lucide icons, plain-text
  parent controls, one all-annotations list, expanded conversation and explicit
  reattach confirmation. `annotation-input.tsx` owns one plain @ input with Enter
  Send, Shift+Enter newline, Escape cancellation. It
  opens at the selection in a cosmetic parent popover; saved threads open in Comments.
  It extends the same `components/listbox.tsx` used by Manage and the agent list;
  input options portal into its dialog ancestor (otherwise the body), so mobile
  modal sheets retain visible, clickable autocomplete across close/reopen.
  UI capture/default labels grant no routing authority. `ask-panel.tsx` shares the
  accepted-turn observation deadline and Clock + `no reply yet` copy between annotation
  threads and Chat; it retains read-only recheck without changing ledger state.
- `chat-panel.tsx` reads designated device threads from the same admitted projection,
  shows their page-visible history and inline outcomes, and continues only its device's thread with the shared input. Chat threads are excluded from Comments. No new store
  or record fields are introduced; two-hour reply timeout is display-only.
- `public/renderer.html` installs bounded selection capture and cosmetic quote
  resolution, highlights and count-bearing margin markers before author HTML; `renderer.ts` binds narrow requests/results to
  the current render and request. CSS Highlights and pointer-inert overlays grant
  no source truth or application capability. Known-thread marker messages open
  only parent views. Highlight messages contain only IDs and quote selectors, never
  comment bodies or display labels; marker tooltips use quoted text and comment
  first-line labels stay in parent chrome. Selection rectangles and anchor offsets are bounded cosmetic
  claims; normal anchor navigation scrolls the window. Ports/observers clear on teardown.

Verification: `test/thread-records.test.ts` and native `tests/discussion.rs`
consume `contracts/vectors/discussion-v1.json`, including causal and status-export byte
vectors. `test/thread-status.test.ts` and native `export::tests` pin the effective status
and provenance; native `tests/page.rs` checks agent status publication, shared content/own
sequence, idempotence, stale-base refusal and CLI no-dispatch behavior. `test/own-fold.test.ts` checks
atomic preparation and immutable rejection; `test/live-ask.test.ts` checks real
comment IDs in signed Ask framing. `e2e/renderer.spec.ts` tests bounded DOM quote
capture/resolution and containment. `acceptance/discussion.spec.ts` uses two real
paired browsers, foreign-stream replies, reload/restart, exact explicit Ask effect
counts and desktop/mobile theme screenshots. Use the existing acceptance harness
and [commands](development.md); no separate server or state store is introduced.
