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
  line. `conversationAsks` owns admitted comment/Ask association for display, reply
  defaults and captured context; `CommentExchange` presents it on both surfaces. Replies have one agent/name/time byline; pending states
  and trusted delivery actions stay in the requester turn until a reply exists
  (including an empty reply), then the status disappears.
- `thread-panel.tsx` exports `ThreadWindow`/`ThreadWindowProps` for the shared
  conversation body, including the initial selection before a thread exists. The
  anchored layout keeps its header and parent composer stationary while only its
  messages scroll; new turns and associated replies move that area to the arrival.
  A parent composer slot preserves the same input instance through first commit;
  an optional awaited status callback delegates one trusted action to the parent,
  with busy/error display and no notification or storage capability. Without that
  callback, existing writer-owned controls use the discussion binding.
  It owns muted author/time labels with device-ID tooltips, Resolve/Reopen/Close
  Lucide controls (visible labels in Comments, hover/focus captions in the anchored
  window), plain-text
  parent controls, one all-annotations list, expanded conversation and explicit
  reattach confirmation. Row action menus choose below/above placement when it
  fits, otherwise clamp inside their scroll container so the stationary window
  header cannot cover Edit/Delete. `annotation-input.tsx` owns parent draft/recipient/send
  policy around the shared Lexical plaintext message composer: Enter submits the
  current parent action, Shift+Enter adds a line, Escape closes candidates before cancellation. It
  opens at the selection in a cosmetic parent window and continues there after
  Enter; no drawer opens automatically. Known margin markers reopen that exact
  thread using the current renderer's admitted cosmetic position. The page owner
  keeps drafts and recipients by thread across collapse and explicit Comments
  access. Resolve uses the existing writer-owned binding and collapses only after
  success.
  It extends the same `components/listbox.tsx` used by Manage and the agent list;
  input options portal into its dialog ancestor (otherwise the body) and use the
  browser popover layer with viewport bounds, so mobile modal sheets retain visible,
  clickable autocomplete across close/reopen. Plain replies and edits reuse the field
  with explicit buttons/newline policy. Recipient identity stays independent of text;
  a plain comment does not require working agent discovery.
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
