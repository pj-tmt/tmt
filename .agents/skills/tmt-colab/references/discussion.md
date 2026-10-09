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
  owner action remains effective after revocation; new publication is denied. The browser
  admits the same authors (`Admission.readAuthor`: owner-member devices and `bridge.add`
  bridges, shared vector `bridge-own-v1.json`) and derives status authority from owner-device
  provenance (`Objects.statusWriter`), so both readers project the same authenticated status.
  `discussion.rs` reuses the authenticated export projection for native reads and
  prepares agent status actions with no recipients or dispatch. The isolated decoder
  prepares only that writer's own structs; `page::prepare_own_publication` freezes them as a
  `kind:"own"` publication that `main.rs::publish_write` sends through the offline lock or
  the root-local `page-publish` route, with the certificate, sequence, ciphertext commit
  and unknown-outcome rules of source writes. `cli_threads.rs` adapts these operations to `threads`, `resolve` and `reopen`.
  Actor labels and clocks remain display assertions.
- Attachments (#1854): `attachment-file.ts` owns bounds, filename shortening and the
  header-only PNG/JPEG/WebP parse; `attachment-service.ts` (`AttachmentService`, held by
  `ThreadStore.attachments`) owns seal, begin/part/commit, status resume, discard, the
  publication records and the admitted read; `attachment-draft.ts` is one composer's chip
  state machine (no effect before Send, no automatic retry); `attachment-tray.tsx` and
  `message-attachments.tsx` are the trusted-click composer and message surfaces.
  `ThreadStore.create/createChat/reply` take the preallocated message ID and committed
  originals and write the proofs with the message in one batch; edit refuses a message that
  has references. A message attachment is fenced by `messageFence` (membership head, epoch,
  author; `attachments.ts` `currentBase`), mirrored by native `message_fence`, so a foreign
  write mid-upload never stales it. Checks: `test/attachment-{file,service,draft}.test.ts`,
  `colab-client/test/attachment-fence.test.ts`, the
  attachment cases in `test/thread-records.test.ts`, `e2e/attachments.spec.ts` and the
  real-binary `acceptance/attachments.spec.ts`.
- Page files (#1855): `document-files.ts` (`LiveDocumentFiles`, `PageBinding.files`) proves
  with `AttachmentService.publication(stored, source)` then saves the typed change through
  `Live.edit`; `files-panel.tsx` holds the writer panel and the shared `FilesList` rows (also
  the read-only reader list via `ReaderSession.attachments`); `attachment-opener.ts` is the
  shared trusted-click open hook. Checks: `test/document-files.test.ts`,
  `e2e/files-panel.spec.ts` and the Files case of `acceptance/attachments.spec.ts`.
- `components/conversation-window.tsx` owns one header, full-width history and bottom
  composer placement for Chat, anchored threads and new annotations. Window geometry
  remains caller-owned; this component owns scrolling for both surfaces.
  Its shared composer does not shrink; history takes the remaining height, down to
  zero, so the field, status and Send stay inside the window with a long history.
  It opens at the history bottom and follows new record identities while the reader
  is within 24px of the bottom. Above that threshold, arrivals preserve the reading
  position and show a politely announced "New messages" text action at the history's
  bottom edge. Activating it or manually returning to the bottom clears it.
  New comments from this device's writer always follow latest, before Remote delivery;
  agent replies have separate identities and do not count as an own Send.
  Cloned publications, edits and size changes are not arrivals. Size changes retain
  following; reaching the bottom after resizing also clears pending state, including
  when every message fits. The shared owner disconnects size observation on
  unmount and retains composer focus, draft and caret. Hiding a focused jump action
  returns focus to the named history region. `components/conversation-turn.tsx` owns their
  shared flat message markup: neutral 1px row rules, muted author/time and an agent
  3px ink rail. Status and trusted text actions share the meta line's right side;
  they wrap together at narrow widths. `components/message-text.tsx` and Lexical
  mention nodes share a cosmetic grey token style. Sent text marks only supplied
  bound recipient names; other `@text` remains plain. Display labels grant no routing
  authority. `conversationAsks` owns admitted comment/Ask association, reply defaults
  and captured context; `CommentExchange` presents it on both surfaces. A reply,
  including an empty one, removes pending status and delivery actions.
- `thread-panel.tsx` exports `ThreadWindow`/`ThreadWindowProps` for the shared
  conversation body, including the initial selection before a thread exists. The
  anchored layout fits its content and grows away from its selection edge up to
  the available viewport height before its messages scroll;
  its header and parent composer stay stationary during message scrolling.
  It supplies comment and admitted reply identities to the shared scroll owner;
  a delayed reply is an arrival, while editing an existing reply is not.
  A parent composer slot preserves the same input instance through first commit;
  existing writer-owned controls await the discussion binding with busy/error
  display and no notification or storage capability.
  It owns muted author/time labels with device-ID tooltips and the parent
  Resolve/Reopen/Close actions. Their icons, hover/focus tooltips and disabled/busy
  presentation use `BrowserIconAction`; remaining thread/edit submits use `BrowserAction`,
  with primary reserved for the form's default submit and other actions text. The
  host header gives its status title flexible space before fixed right-aligned
  actions, including the muted attachment label on the same row. It retains plain-text
  parent controls, one all-annotations list, expanded conversation and explicit
  reattach confirmation. Row action menus choose below/above placement when it
  fits, otherwise clamp inside their scroll container so the stationary window
  header cannot cover Edit/Delete. `annotation-input.tsx` owns parent draft/mention/send
  policy around the shared Lexical plaintext message composer: Enter submits the
  single Send, Shift+Enter adds a line, Escape closes candidates before cancellation. It
  opens at the selection in a cosmetic parent window and continues there after
  Enter; no drawer opens automatically. Known margin markers reopen that exact
  thread using the current renderer's admitted cosmetic position. The page owner
  keeps drafts and bound mention UUIDs by thread across collapse and explicit Comments
  access. `draft-store.ts` persists them per device behind that map (encrypted record per
  space/device/page, `DraftSession` coalescing, page restore on mount); `saved-drafts.tsx`
  lists selection drafts and replies to vanished threads under Comments. Restore only fills
  composers; the Ask ledger is untouched. Failed storage shows `Drafts are not saved on this
device.` once per failure and keeps the in-tab draft. Resolve/Reopen is the parent's `onStatusChange` seam over
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
  accepted-turn observation deadline and Clock + `No reply yet from <agent>` copy between annotation
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
