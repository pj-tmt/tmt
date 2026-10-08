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
  prepares only that writer's own structs; `page::prepare_own_publication` freezes them as a
  `kind:"own"` publication that `main.rs::publish_write` sends through the offline lock or
  the root-local `page-publish` route, with the certificate, sequence, ciphertext commit
  and unknown-outcome rules of source writes. `cli_threads.rs` adapts these operations to `threads`, `resolve` and `reopen`.
  Actor labels and clocks remain display assertions.
- `components/conversation-turn.tsx` owns the shared turn markup, attribution and
  square styling. Its `thread` layout uses User/Bot avatars and an ink agent-body
  rail; its `chat` layout uses sided tinted turns and a bot mark in the agent meta
  line. `conversationAsks` owns admitted comment/Ask association for display, reply
  defaults and captured context; `CommentExchange` presents it on both surfaces. Replies have one agent/name/time byline; pending states
  and trusted delivery actions stay in the requester turn until a reply exists
  (including an empty reply), then the status disappears.
- `thread-panel.tsx` exports `ThreadWindow`/`ThreadWindowProps` for the shared
  conversation body, including the initial selection before a thread exists. The
  anchored layout grows toward the viewport bottom before its messages scroll;
  its header and parent composer stay stationary during message scrolling. First
  open, new comment IDs and changed associated replies move that area to the
  arrival. Unrelated cloned publications preserve a reader's history position.
  A parent composer slot preserves the same input instance through first commit;
  existing writer-owned controls await the discussion binding with busy/error
  display and no notification or storage capability.
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
  access. Resolve/Reopen is the parent's `onStatusChange` seam over
  `ThreadStatusCoordinator`, not a thread edit: the window receives the thread's
  `ThreadPresentation.status` and awaits one status change (shown only when
  the local device has owner-member provenance, `Admission.ownerDevice`, the
  browser form of native `status_writers`; the fold already counts only writers
  admitted that way), collapses the anchored layout only after a resolve that left
  nothing to tell, and lists mentions that could not become recipients as
  `Not notified`. Only the parent's trusted open paths call
  `markThreadStatusSeen`; rendering, panel opening, close and reload never do.
  A resolved thread leaves the renderer's anchor set, so its margin marker is
  hidden until Reopen, while Comments keeps its row (`Resolved`, or
  `Resolved by <agent>` with an unseen `New` mark) and the Comments toggle shows
  the open count.
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
consume `contracts/vectors/discussion-v1.json`, including causal and status-export byte
vectors. `test/thread-status.test.ts` and native `export::tests` pin the effective status
and provenance; native `tests/page.rs` checks agent status publication, shared content/own
sequence, status preservation and causal continuation after own-stream compaction,
idempotence, stale-base refusal and CLI no-dispatch behavior. `test/own-fold.test.ts` checks
atomic preparation and immutable rejection; `test/live-ask.test.ts` checks real
comment IDs in signed Ask framing. `e2e/renderer.spec.ts` tests bounded DOM quote
capture/resolution and containment. `acceptance/discussion.spec.ts` uses two real
paired browsers, foreign-stream replies, reload/restart, exact explicit Ask effect
counts and desktop/mobile theme screenshots. Use the existing acceptance harness
and [commands](development.md); no separate server or state store is introduced.
