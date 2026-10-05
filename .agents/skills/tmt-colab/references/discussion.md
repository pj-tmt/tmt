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
- Native `threads.rs` validates typed record grammar after isolated Yjs decoding.
  It has no DOM, signatures, publication or dispatch responsibility.
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
consume `contracts/vectors/discussion-v1.json`. `test/own-fold.test.ts` checks
atomic preparation and immutable rejection; `test/live-ask.test.ts` checks real
comment IDs in signed Ask framing. `e2e/renderer.spec.ts` tests bounded DOM quote
capture/resolution and containment. `acceptance/discussion.spec.ts` uses two real
paired browsers, foreign-stream replies, reload/restart, exact explicit Ask effect
counts and desktop/mobile theme screenshots. Use the existing acceptance harness
and [commands](development.md); no separate server or state store is introduced.
