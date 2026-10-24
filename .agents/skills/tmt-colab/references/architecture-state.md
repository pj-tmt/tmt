# Layout, state, transitions and admission

Source: `extensions/tmt-colab/rust/tmt-colab/src`. Exact DTOs, codes and limits are in
[colab-v1](../../../../extensions/tmt-colab/contracts/colab-v1.md) and `limits.rs`; do not
restate them.

## Layout

| Path (under `extensions/tmt-colab/`) | Owns                                                                                                                 |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| `rust/tmt-colab-model`               | Pure codecs, fixed crypto and the Rust side of the vectors. No I/O, core or Remote.                                  |
| `rust/tmt-colab`                     | The executable: CLI, owner-only socket, SQLite store, keyring, sync server, owner transitions, isolated Yjs decoder. |
| `typescript/colab-client`            | WebCrypto primitives mirroring the model, tested against the shared vectors.                                         |
| `typescript/app`                     | React/Vite app: trusted parent chrome, renderer, Worker fold, writer, Ask modules and the `acceptance/` suite.       |
| `contracts/`                         | colab-v1 and the frozen vectors with their independent Python oracles.                                               |

## Gotchas

Page links stay with trusted chrome: `page-link-request.ts` admits the current renderer's closed
request and classifies URLs against the trusted app base and space. `page-link-bar.tsx` owns the
frozen pending confirmation and the one in-place navigation per render. `renderer.ts` checks
frame/lifetime and reads the parent's browser activation at receipt; the bootstrap only intercepts
links and reports intent. Neither its render ID nor its messages authenticate a user gesture.
This is a bounded parent-owned exception: same-space navigation requires recent browser user
activation or trusted confirmation, and outside URLs always require confirmation with the full
frozen URL. No frame privilege or network policy is widened.
The [renderer contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#renderer-and-live-anchors)
owns direct internal activation versus confirmed external/fallback behavior. ReaderApp reuses the
confirmation owner without receiving same-space app-navigation authority.

Attachment descriptor/manifest/selector/publication syntax and asset verification
live in model `attachment.rs` and client `attachment.ts`, reusing existing crypto;
[attachment-v1](../../../../extensions/tmt-colab/contracts/attachment-v1.md) owns the bytes.
The app's `FilesSurface` (`files-panel.tsx`) owns the Files drawer and its sibling
preview/removal dialogs. `FilesUpload` retains the picked-byte draft while the
one-overlay view switches; `DocumentFiles` remains the typed add/remove owner.
`useAttachmentOpener` owns admitted read lifetimes. A preview captures its current
lifetime at trusted activation and drops its display after a disclosure change,
without replaying the read. Owner and reader hosts share that surface.

Runtime `attachments.rs`/`attachments.ts` join exact authenticated references to
positive-sequence own creation proof and recheck current authority after committed-object
I/O. Fold has separate writable and archive-capable read capture; frozen epochs remain
read-only. Native Session reads use registration/readers' actual context and eligible
addressed wraps or owner-signed public keys; browser history keys are opaque addressed-wrap
handles. `object_channel.rs` owns the raw handshake on the existing MountSocket,
fresh public Remote discovery and replacement/join of each neutral Bus generation.
The dormant sealed-document client shares that generation's one dispatcher and
ordered request counter with objects. `object_channel/sealed.rs` correlates sync
results, coalesces Changed hints and retains terminal interests until confirmed
cancellation or generation close. `object_channel/documents.rs` maps Colab fields to
canonical REST Values and page-bound IDs; checksums and routing metadata confer
no admission. The public opaque SealedClient has no standalone constructor.
`object_channel/cloud_sync.rs` is the serve's one cloud-sync worker, started by `socket.rs` beside the rekeyer
and stopped after the object channel closes. Per generation it asks Remote to activate (the
binary injects `supervisor::activate_sync`), waits for that generation's channel, binds, and
keeps the most recently active page published: `object_channel/cloud_sync/publisher.rs` creates the page's
admission rows (including a live link's `links` row), then the signed membership and chain
originals of the streams this machine writes and the chains of each live link's unrevoked
devices (`link_device_chains`), the epoch's signed baseline (from the second epoch, in
the `checkpoints` carrier under the hash its `epoch.advance` names), the wraps (including those addressed to
links), then each local original in sequence, and `object_channel/cloud_sync/local.rs` reads them from the
Store and records a position only after the exact original was created or read back identical
(`store/publication.rs` owns the positions). An ended link's cloud rows are removed by the publisher's
retirement step (`links` row, then its `holders`), driven by `link_publications`. This machine writes the
root-local writer its keyring names (every CLI write and browser Save, with no registration row) and the devices
it registered; `OwnerTransaction::is_local_device` is the one decision, used by both the publisher and the inbound
path, and a chain imported from the cloud stays foreign. `sync.rs` reports the page a peer opened or wrote
through `ActivePage`, and `socket.rs` also touches it after a management change (`Applied.touched`); the worker never retries a refusal Remote will not change by itself
(`object_channel/cloud_sync/activation.rs`) and shows its one line once. The same worker is the reader of that page:
`object_channel/cloud_sync/interests.rs` holds the three interests whose notices only wake it,
`reader.rs` discovers the signed authority, one foreign stream's log tail and, for a stream this
machine holds nothing of, the page's checkpoints, and
`inbound.rs` (`Puller`) compares the cloud's membership log with the owner log, then follows
each foreign stream from the Store's `stream_head`; the worker reads only when a read is due (a
newly watched page, a notice, an unfinished read), and chains are read again at most once per
`CHAIN_RECHECK` after a log notice. `inbound/store.rs` admits an original with
`Snapshot::open_stream_object` (the fold that reads stored objects), keeps the writer's verified
chain for later catch-up, and applies it through
`Server::ingest`, the split of the browser append path, so one apply path serves both and nothing
that arrived is queued for publication; a checkpoint pair (and a fold's anchor) goes through
`Server::ingest_bootstrap` and `Store::bootstrap` instead. A refused writer stops alone, per page and epoch; `stops.rs` keeps the
page's stopped state in the worker's memory and the lines announce each transition.
Adopting a cloud-only log, forks, attachments and a second machine's own
enrollment are not implemented.
Its `admission` module captures the actual peer, current native context, exact
request and reference/target; `peer` serializes bounded original-ID calls without
retries. Reader tickets take precedence over a paired cookie and never upload.
`history` reuses sync's retained ciphertext pager under current history entitlement,
without opening another socket or changing the live fold. Browser `Connection`
requests run outside its Worker executor; historical fetch uses a detached Worker.
Complete committed reads check every range and raw digest before Colab asset crypto.
The root-local read is the serve's owner-only `attachment-read` route (`attachments/ipc.rs`,
`socket.rs`): it requires an established channel, never activates storage or opens a backend, and
answers only the local CLI (export and `attachment read`). `attachments/media_type.rs` derives bounded raster, PDF and strict UTF-8 text types from bytes, matching the browser and the shared media vector; native sealing and CLI reads use it rather than filename or signed type labels. Native attach (#2291) is its write counterpart: `attachments/slots.rs` (serve-owned staging slots), `attachments/seal.rs`, `attachments/attach_ipc.rs`, `object_channel/attach.rs` (upload protocol over a `Transport` seam, then proof and list change through `Publish`) and `RootWriteAdmission`, all behind `socket.rs`'s `attachment-stage`/`attachment-attach` routes; Remote's hub already admits `LocalExtension` uploads. Rekey (#2293) reuses that pipeline: `rekeyer.rs` is the serve's one maintenance worker, woken after admitted management changes, membership-changing device revocations and new channel generations; it also runs a bounded rotating sweep of signed deletion tombstones through `object_channel/namespace.rs`, whose native-only callbacks recheck the committed deletion before namespace removal; `attachments/rekey.rs` reads what waits from the page, and `object_channel/rekey.rs` re-seals through slots that record the attachment they `replaces`. A second ordinary owner Save repairs complete image `src` attachment references from the latest verified source, with the complete revision and source digest fenced; pending old-to-new mappings remain in the same 16-slot pool until settled. Maintenance reconciles a committed swap before age cleanup and before another replacement. Production Remote declares Colab Local;
its routed lifecycle gate uses the three shipped binaries. Snapshot/retained-reference
persistence is #2299, not a new Store/schema here. Read failures are four reasons
(`denied`, `not-found`, `changed`, `unavailable`): `object_channel/peer.rs` `error_code` maps
native faults and `AttachmentReadError` carries them in the browser; colab-v1 owns the lifecycle matrix. Historical `chains`/`readAuthor`
ignore original-creator current expiry, matching native folds; issuedAt, issuer/signature,
membership and revocation cuts still apply. Fresh `author()` and the actual caller
`validateRead()` retain expiry checks.

- Run Rust gates in the worktree's `rust/target` (no external `CARGO_TARGET_DIR`), with `CARGO_BUILD_JOBS=2`. Add
  `--no-fail-fast` when judging `cargo test -p tmt-colab`: Cargo stops at the first failing
  test binary and hides the rest. Timing-sensitive decoder tests can fail under load; rerun
  them alone before treating one as a regression.
- Run the architecture test on every Rust push:
  `cargo test --offline --locked -p tmt-cli --test architecture`.
- Regenerate frozen vectors only with their Python oracle in a throwaway virtualenv with
  `cryptography`, never `--write` outside a reviewed regeneration.

## Page markup check

`rust/tmt-colab/src/page_check/` is the pure checker behind `page check` and the warnings of
`page create` and `page write`: `tree.rs` parses with `html5ever` into an arena `TreeSink` (a
`template` fragment context, scripting off, nesting capped at 512 because the tree builder scans
its open elements on every start tag), and `question.rs`, `group.rs`, `review.rs`, `reading.rs` and `image.rs` hold the rule
tables (`group.rs` also decides which groups are skipped and which IDs are held, review sections included). It does no I/O; `cli_page_check.rs` adapts it to the commands. The browser's `inspectQuestions`
(`src/questions.ts`) is the other implementation of the question, group, review-section and reading-anchor rules. Both run
`contracts/vectors/page-markup-v1.json`: `e2e/question-vectors.spec.ts` in headless Chromium and
`tests/page_markup_vectors.rs` natively. Change a rule in both implementations and in the
vectors in the same PR. `html5ever` is a `tmt-colab`-only dependency (exact pin, no default
features, listed in the architecture guard); `tmt-colab-model` stays dependency-free.

## App build entries

The app build emits its main entry plus two public standalone entries (`vp build`, then
`vp build --mode recovery` and `vp build --mode reader`; the `build` script runs all three):

- `assets/recovery.js` uses the tab's bounded SDK recovery for private guidance.
- The read-only reader (`public/reader.html` served at `/read`, `src/reader-main.tsx`) builds as fixed-name
  `assets/reader.js`, `reader.css` and `reader-fold.js` (the decoder worker), so the native allowlist
  `assets::anonymous_file` is exact. Add a file to the reader entry only together with that list,
  the contract's anonymous-asset sentence and `served.spec.ts`.

The native `/assets/chrome.css` token/header stylesheet also remains available without
an app build. Only these files and `renderer.html` are public; the rest of the app stays owner-gated. Verify the
guidance CSP and pairing failure/reload guard alongside the app lifecycle tests (`served.spec.ts`,
`session-recovery.test.ts`).

## Message editing boundary

The answer-block path uses parent `questions.ts` admission, `question-layer.tsx`
group selection and verified-thread projection. `components/question-group-card.tsx`
owns drafts and publication outcomes once per logical group, with recipients and
failures scoped to each physical Send block. `question-card.tsx` presents the
controlled answer/conversation; `question-line.tsx` is the cosmetic disclosure.
A structural wrapper sends its eligible answers together; standalone questions
sharing `data-group` keep separate placements and Sends. `inline-slots.ts` owns
one measurement batch for proposals and answer blocks. Each compact block uses
the existing inline placeholder for its placement anchor and the bounded after
spacer for its complete trusted height. Authored nodes stay in the canonical
quote text map. `open(id)` selects and opens a question without publishing.
The render-bound shapes and publication rules live in
[Answer blocks](../../../../extensions/tmt-colab/contracts/colab-v1.md#answer-blocks).

Review sections ride the same renderer handshake and slot batch: `review-layer.tsx` draws the
parent bands and `review-actions.ts` publishes the explicit actions through the existing thread,
Ask and status owners; see [Review sections](../../../../extensions/tmt-colab/contracts/colab-v1.md#review-sections).
Recommended-reading metadata uses the same inert parser and shared check vectors. `reading.ts`
projects person read state from ordinary verified thread records; `reading-layer.tsx` reuses the shared NeedsYouBar and exposes its ordered read projection and
cosmetic navigation; `reading-actions.ts` records
and resolves through the existing binding/coordinator, and answers `ready` for the same condition
the click needs, so the layer disables `Mark read` until it can act and a click never does nothing. Its named-agent restriction excludes
other prose mentions. `reading-link.ts` owns cosmetic owner/paired navigation hints (`read`, `anchor`
and `task`, at most one, each with its own shape; `hintSearch`/`hintFragment` are the one spelling of a
hint in a router search or a fragment); Ask capture accepts only that exact hint and removes it from the
canonical admitted URL. A `task=T-n` hint picks which task of the page's own work list its band
opens (`WorkBand` `focusTask`, once, and nothing for a task the page lacks); the child page's crumb
links to its parent page with it. Reader capabilities are unchanged.

`needs-you.ts` combines the admitted layers' pending navigation without a stored count;
`needs-you-header.tsx` owns only its header menu. Layer-owned jumps scroll/focus existing
targets and open question disclosures; they never publish an action or create a thread.

The main app uses one Colab-local `components/rich-input.tsx` editor lifetime and
editable surface. `components/message-composer.tsx` configures its mention nodes,
completion and message submission for Chat, annotations, agent follow-ups, plain
replies and comment edits. The shared input accepts a caller-owned document adapter
and keyboard policy; its optional `accessory` slot is presentation inside the input
below text, aligned to its left edge; adding or removing it retains the editor root,
focus, caret and selection. A caller without mentions uses
`plainRichInputDocument`; it receives no recipient, transport or send capability.
`rich-input-document.ts` also owns the token, caret and range helpers both callers use
(`$richInputTokens`, `$richInputCaret`, `$selectRichInputRange`, `$replaceRichInput` with
caller tokens), so the composer's mentions and the page list's `project:`/`by:` tokens share one
document model and no second editor exists. Exact 0.52.0
`lexical`, `@lexical/react`, `@lexical/plain-text` and `@lexical/history` are app-only
runtime dependencies; only plaintext/history extensions are imported. The source
editor is a separate editing mode. Reader and recovery entries have no composer.

`ComposerEdit` emits one atomic snapshot of exact plaintext and all bound
machine/agent UUIDs with UTF16 mention ranges. Paragraphs serialize with one LF,
retaining blank and trailing lines. Parent echoes preserve history/selection;
explicit reset keys end a draft's editing lifetime. Mention-node identity follows
edits and undo; removing/replacing an atomic decorator chip invalidates its binding.
`message-mention-chip.tsx` renders presence/removal from the composer's directory context;
only the node's original token contributes to plaintext and UTF16 offsets. `mentionQuery`
opens completion at `@` or full-width `＠` after non-address-like text, including CJK;
`email@host` never routes. No Lexical document is persisted or sent as authority.

`AnnotationInput` owns the draft, trusted action, current content/Ask admission and
immutable one-comment send capture. `message-send-refusal.ts` shares its button and
explicit-submit presentation gates and classifies closed failure references. The
composer keeps a visible, announced refusal below the field until a deliberate
text edit, retry or successful Send; saved S2/S13 states stay neutral, without an
error mark or a resend action.
Connection guidance and editor normalization do not clear it. This state is local
to the draft, never stored or sent. The discussion contract below owns the reference
codes. The page-owned `creator-default.tsx` preference
shares admitted defaults across composers; mention-node provenance is editing-only
and the unchanged encrypted v1 draft format restores mentions as manual. `message-recipient.ts` resolves exact typed names
and distinct UUID pairs for presentation only; LiveAsk re-admits each frozen Ask.
The [browser discussion contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#inline-annotation-conversations-1587)
owns default-creator lookup, typed/picked binding, eight-recipient fan-out, offline
handling and recipient failure rules. `conversationAsks` owns comment/Ask association
and captured conversation (a batch Ask is associated with every question thread it covers,
its reply with the canonical one); neither prior replies nor display names select recipients.
`answer-batch.ts` owns an answered group's delivery: the origin validator, the Ask text and
`sendAnswers`, which publishes the comments and asks each recipient once. A card calls it and
never publishes comments, composes Ask text or keys retries itself.
The composer consumes an explicit draft-local mention request through its existing
editor, retaining the last selection across action focus and prepending a manual
chip as one undoable edit. The editor has no storage, ledger, notification or Remote capability. `LiveAsk` reports
whether Send adopted an operation. Only an authoritative failure before adoption
starts can be shown locally without inventing a signed Ask row; missing read views
after publication failure remain uncertain. Adopted outcomes come from the admitted stream.
`ask-again.tsx` owns the shared trusted recipient action and in-tab activation fence;
`AskRecordStore` owns the pair-locked durable UUID marker, block-check and adoption. Original
comment association and fresh capture stay with `CommentExchange`/`LiveAsk`, not
with the editor. The discussion contract owns exact eligibility and reload rules.

`agent-directory.ts` (`useAgentDirectory`) gives each Ask binding one explicit
`loading`, `failed` or `ready` state (empty `ready` is a real answer). Replacement
and explicit retry read again, show loading in the same render, and ignore obsolete
results. Mention Sends stay fenced before preparation until ready; plain comments,
draft, caret and token UUIDs remain. Failed discovery shows Try again in place,
kept mounted/busy during retry with focus restored. The page owner projects connection
copy once for every composer: ended uses the same
`EndedBody` sentence/action as the page; reconnecting says "Reconnecting…" and holds Send
while retaining the draft. Both outrank recipient/discovery and pre-record failure copy.
Recorded or uncertain turns keep their own status; connected copy keeps genuine
recipient/policy and record failures. There is no per-keystroke/recipient read or retry timer.
A `failed` read that Remote refused or ended carries Remote's code as a muted `Code: …` reference
under the same generic line. `REMOTE_STATE_UNAVAILABLE` is any Remote storage fault, so it gets no
sentence of its own; one cause is a full per-device request journal (1000 owned entries per 24 h,
which reads such as `agents.list` also consume, #2170), cleared as entries expire and prevented
by the sync keepalive that stops idle tabs from reconnecting.

Candidate geometry is input-only in the shared Listbox: it uses viewport bounds and
the native popover layer, remaining inside the current modal dialog's ownership.
Management pickers retain their button policy. No native asset allowlist, embedding
mechanism or CSP changes are required; no inline-style/eval relaxation is permitted.

## Read-only reader

`src/reader-link.ts` parses the fragment (strict grammar in the contract), `src/reader.ts`
(`ReaderSession`) derives the link keys and the link's `door` device from the seed
(`link.deriveLink`, `link.deriveDevice(…, 'door')`, `link.certifyDevice`; byte-identical chain on every open), runs challenge, session and sync, and reconnects until access ends.
It reuses `Admission` through its `ReaderSeat` option (link-addressed wraps, owner-log
verification, nothing persisted) and `Connection` with the ticket subprotocol. `src/reader-main.tsx`
removes the fragment first; `src/reader-app.tsx` renders read-only with the sandboxed renderer, and for a link
whose `role` is `editor` also offers the shared Source drawer (`src/reader-source.tsx`, `source-editor.css`), which
sends an edit request through `ReaderSession.edit`: `link-edit.ts` builds the `edit-request` record and its size
refusals (`EditTooLarge`, the one-document bound), `PageAnswer` reads the outcome from the page itself (applied,
changed, or still waiting), and the session's `Writer` seals the record into the link device's own stream with
`Admission.linkAuthor` (via `ownAuthor`, never the member `author`), keeping its pending state in memory only. Every
holder of a link writes that one stream, so a lost race for the next sequence comes back as `OwnAppendRefused`
(serve `CONFLICT`, link sessions only): the `Writer` drops the never-stored entry and the next write seals at the
new head; `Store::append_shared` (live link sessions, `sync.rs`) refuses it without freezing, `append` still freezes. The
owner router and the owner's Ask and export modules are not part of this bundle.

## Restart recovery

The owner-socket module description separates static navigation from the registration's
catalog projection. A busy registration mutex still describes Colab's rail, Pages link and
script entry, without Pages count, `needsYou` or Home cards. The shell treats these omitted
fields as unknown and draws no count. Missing or poisoned registration and actual catalog
read/integrity or capacity failures still refuse. This keeps reconnect contention from
advertising a running Colab as absent, without changing owner-session or page admission.

Remote keeps sessions and door cookies in memory, so after a restart a paired browser's
reload receives Colab's private guidance page. That page loads the public `assets/recovery.js`
(`src/guidance.ts`), which lets Remote's SDK check its paired key and call
`reopenSession()` for that tab, then reloads. Inside the one TMT web app (`/colab/…`) the host
route does the same before it loads the module: when the composed shell is missing it reopens the
session from the pairing and reads the shell again (`shell-app.ts`). The public reader entry loads
the app in place after `reopenSession()`, and a refusal that is the verified
session limit (`sessionLimit()`, `session-limit-error.ts`) still loads the app, whose own open
shows the limit card, never the unpaired guidance. A session-storage marker
(`colab-recovery:<mount path>`, `src/session-recovery.ts`) spans that reload so a second
guidance response cannot loop; authenticated boot (`mounted.ts`) clears it. A failed or
refused reopen, or unavailable session storage, leaves neutral reopen guidance and never
retries an Ask. Native HTML cannot infer pairing loss from absent owner context. The recovery
script requests one typed SDK attempt with a timeout signal (no bounded retry); only its
`unpaired`, `revoked` or `expired` result reveals the pairing command. Mismatch, capacity and
transport/storage uncertainty preserve pairing and show their approved recovery copy. On an open page's socket close, `Live` makes one read-only probe of the exact old
Connection/Registration/Remote owner. A current pending same-session attempt keeps
that owner while diagnosis settles, even if readiness has already rejected.
Verified session end starts one existing mounted-owner replacement; verified eviction
blocks. A pending failed attempt also blocks on a successful read without session end
or an unverified read failure. An opaque socket disconnect, or a native fetch
`TypeError` during mounted-owner Session replacement, stops the binding with
`RecoveryRequiredError` (original cause retained). A fresh open that Remote refuses with a typed
`ReopenSessionError` reason (`registration.ts` turns it into `ReopenFailedError`; a session is
re-admitted with `reopenSession(previous, {retry:'bounded', signal})`) waits when the reason
may pass (`unreachable`, `unconfirmed`, `transient`) and ends the page otherwise.
An admitted update reaches the page as a fresh detached copy of the whole projection (`Live` clones
it for each listener). The page route passes it through `shareView` (`share-view.ts`) against the
view it holds, so every part equal by value keeps its object and a reply to one thread changes
only that thread's path. Sections keyed on a part, such as the document highlights and the question
layers, do not run again for a change elsewhere, and the document frame is sent nothing for an
update that does not touch it (`e2e/large-page-update.spec.ts` counts what the frame receives).
The view itself is still a new object for every publication: the binding (its Ask facade, its
files) can change without the data changing, and the page must render to read it.

One owner per open page decides what happens next: `useConnection` (`use-connection.ts`) wraps
`ConnectionController` (`connection-controller.ts`), whose state (`connection-model.ts`) is
`connected | reconnecting | ended{reason, action}`. The page reports every failure its binding
publishes (`controller.report`); `settle` maps it. `reconnecting` is silent: the header chip reads
`Private · Reconnecting…`, no card or button shows, the page, drafts and open thread stay, and
writes are held. A lost transport or session retries `binding.reconnect()` after 1, 2, 4 and 8 s,
then every 15 s with 20% jitter, paused while the tab is hidden (`ConnectionOwner`), until it
connects or the failure is no longer recoverable. A series that gives up, or a binding with no
`reconnect`, ends the connection with the failure it stopped on: nothing else drives
`reconnecting`. An epoch advance stops that loop and reopens the page (`PageReopener`, at most
three reopens a minute), then hands the new binding over. HTTP admission refusals retain their
bounded body and code (`AdmissionRefusedError`): named endings stay terminal; a bare 404,
502, 503 or 504 is the door's mount answering for an extension that is restarting or upgrading
(#2929), which says nothing about the device, so it is `retry` and `Live` waits in the silent
transport loop above, as it does for a network failure; other stale or unclassified refusals
use the finite reopen path (five attempts at 0/1/3/8/15 s), with the existing reconnect-failure
sentence if it is exhausted. A revoked or unpaired device still ends there, because every retry
opens a fresh Remote Session first and Remote refuses it. A recoverable 403 or 503 during
page opening refreshes the same-device Session once through the mounted owner before
rediscovery; its abort signal fences registration, and concurrent Live reconnects share that
owner's pending replacement. PageReopener still owns the finite attempts and timing. Other
admission refusals never extend the transport loop. The page route keeps no loader match after the
reader leaves (`gcTime: 0`): the binding it loaded closes with the page, so entering the page again
loads a fresh binding instead of showing the closed one as Live while it reloads. While the reader
stays, the match never goes stale (`staleTime: Infinity`): a thread selection rewrites only the
address fragment, and a reload for it would replace the binding and close its writer under a send
in flight. `ended` replaces
the page with one card: the failure sentence and the one action that fits (`Pair again` with the
command only for verified revoked, expired or missing pairing; a way back to the space for a
page that is gone; `Reload` for ended sessions, access refusals, identity mismatches and a device
at its Remote session limit, and generic reconnect failures; none for an oversized change). A modal drawer makes that card inert, so a live Chat
composer owns its ended sentence/action once; an archived Chat and every other drawer keep
the same sentence/action in the `notice` row under the header. An ended annotation keeps its
action scroll- and keyboard-reachable within the existing composer overflow. `Live` stays
the transport owner: it keeps its bounded resync
and publishes failures, and no longer decides terminal or recoverable for the page.
One table, `connection-state.ts`, classifies every token the live path can fail with (sync frame
codes, Remote refusal codes, HTTP admission refusals, a few short messages, each `ReopenFailedError` reason): what recovers
it (`resync` on the same session, replace the `session`, `reopen` the page for a new epoch,
`retry` a fresh open, or `none`) and which sentence it ends with. Verified pairing loss is a
separate cause from page access ending; a saved identity mismatch never proves pairing loss.
Generic reconnect failures offer Reload and Return to space, never a pairing command.
Only access-ending refusals (denied access, identity mismatch or verified missing/revoked/expired
pairing) use the red failed mark. Other stopped connections use the neutral circle and
`not connected` state, with the `Preview stopped` title. A failure before the page opens
keeps the separate open-failure presentation below. `live.ts`, `readmit.ts`,
`connection-model.ts` and `terminal-failure.ts` read it; none matches raw strings. A signed
`REMOTE_CLOSED`, `REMOTE_GRANT_CHANGED` or `REMOTE_SERVE_STOPPED` replaces the session once,
silently; the access card shows only when that fresh open is refused (#2619).
A terminal failure card uses a `strings.ts` sentence and readable code label
(`terminal-failure.ts`); revoked and expired pairing have distinct sentences. An unknown
non-admission token is not automatically recovered and gets the generic sentence. The raw
diagnostic remains bounded to 120 characters in the reference title rather than its visible label. A test requires a decision for every `SYNC_ERROR_CODES` and
`REMOTE_REFUSAL_CODES` entry, so a new code cannot ship as a raw headline or an accidental retry.
A page that cannot open (the root error on `/pages/…`) is decided once by `openFailure()` (`open-failure.ts`)
from the same classification, because the open has no deadline (#3975) and ends only by a closed connection,
a protocol violation or a refusal. A failure that reopening may cure (any recovery but `none`, and the
`session` and `generic` causes) is plain copy ("This page could not open."), a Retry that runs the page
loader again (`router.invalidate()`; the router then shows the same opening frame as a first open),
Return to space and the raw token as the muted `Code: …` reference, with no red state. A page over the
read limits (`large`) is the same card without Retry, with the way out (`tmt colab export`) and a
readable label in place of the token.
Only a refusal (access ended, not paired, page gone, session limit, or no verified page key)
keeps the red card, now with its sentence instead of the raw message. The recognized generic
reconnect sentence is retryable and uses that same plain open-failure card. A test decides every
`CONNECTION_CODES` entry. Other routes keep the plain root error.
Eviction and management-changed keep their own cards. A device at its Remote session limit
(`REMOTE_SESSION_LIMIT`: Remote never evicts an attached session, #2585) is refused with the
verified `limit` and `settingsUrl`; `sessionLimit()` (`session-limit-error.ts`) turns the boot open's raw
`RefusalError` and a bounded re-admission's `capacity` cause into one `SessionLimitError`, which ends
the page or the mounted tab (`MountedNotice`) with `SessionLimitBody` (`session-limit.tsx`, shared with the
eviction card): close another page, the limit, the one `tmt remote settings sessions-per-device <limit+1>`
command, the settings link and `Reload`, which opens the page once a session frees. A capacity reason
without its verified refusal stays a generic ended page.
Ready-page transport failures retain bounded same-session catchup. Superseded callbacks,
readiness, publications and diagnosis cannot alter the current attempt. Recovery
never re-sends an Ask or generates a new mutation. Pending own-stream envelopes
may be re-delivered through the Writer's existing staging path; exact accepted
envelopes deduplicate as `Replay`. The Writer's leader tab also owns the immutability of the
stream's keys: before sealing, the fold's read-only `own-conflict` command answers whether a
write sets a key the stream holds with a different value, and the leader refuses it with
`OwnRecordConflict` (a hint to the submitting tab, like the append refusal) without closing
anything; `AskController.recover` then records nothing and the next observation reads again. The
check is complete because the leader serializes writes and an append resolves only after the
connection has admitted its receipt, in the queue that runs the next write's check. If automatic recovery fails, the page's own bounded
reconnect loop and then explicit Reconnect first reuse the admitted mounted-owner replacement on the same Live binding. The
stopped write fence remains until the fresh Connection and projection are verified;
then a new Writer uses the identical stream key and the new Ask facade resumes
read-only observation. Recovery keeps the admitted Ask view and mounted composer
drafts; all sends and publications stay blocked until verification. An open annotation
stays mounted while its retired Ask facade is absent and rebinds after replacement.

A tab that outlives an app upgrade says so (`app-build.ts`). The build is the hashed entry
module the page references (`assets/index-<hash>.js`; every response is `no-store`, so a
reload always fetches the served build). On a failure path (mounted registration, a live
page ending) and on route resolution, at most once a minute, the tab reads its own root page
uncached and shows the non-blocking `Colab has been updated.` row with a Reload action only when
the same bundle name has a different hash. The row sits directly under the fixed header and
`--colab-update-height` (set by `data-colab-update` on the root) extends the header metric, so
the page moves down instead of being covered. Inside the TMT web app the document is the shell's:
the running build is the one the Colab module was made with: `vp build --mode module` (always after
`vp build`, in the same `dist`) writes that app build's hashed entry into `assets/colab-module.js`
as `__COLAB_MODULE_BUILD__`, so nothing read after the module loaded (an upgrade can land between
the import and `mount()`) can rename it, and the checks read the paired mount against it. The build
fails without that entry, and `scripts/verify-module-build.mjs` (in `build` and `build:bundles`)
fails when the shipped module carries anything but the entry `dist/index.html` loads; `build-stamp.ts`
holds the one rule that names a build, shared with `vite.config.ts`. The row joins the shell's sticky
bar under the header. The local preview, being unpaired, has no running build and shows no row, and
the dev server, which defines no stamp, shows none either. An offline read, an unreadable page, or a page of
another kind (pairing guidance, the reader) never counts. There is no polling and no
automatic reload: Reload is the reader's action, and unsent in-tab drafts follow the existing
rules (device-stored per page, so a reload restores them when storage works; see
`draft-store.ts`).
The same check reads `GET api/serve-release` (`serve_release.rs`, contract). An installed serve
follows verified `current` selections without an installer hook: receipt-bound executable,
read-only app/store preflight, idle drain up to 60 seconds, then ordinary store startup.
New writes stop during the drain; admitted saves and attachment maintenance finish first.
Pre-cutoff failure keeps the old serve and names manual recovery (`restartFailed:true` in
its release status). Post-cutoff failure names the release and manual recovery, exits
unsuccessfully and never claims rollback. `serve.rs` uses
normal stop: an owned Remote is reaped, an attached Remote stays up. The selected release
execs through ordinary startup, so Remote ownership/stop reporting stay truthful without a
guardian or handoff. An owned door briefly restarts on its saved port. Browser
reload stays explicit; old serves still show the named manual restart row.
A recovery press does not dismiss it or take focus from an active composer. Mobile
Chat closes its modal drawer to reach Reconnect; its DOM selection is retained only
for that recovery and restored when the same connected composer regains focus.
Concurrent clicks share one in-flight attempt. Network failure returns to the waiting
notice and editable draft without another automatic reopen. Eviction, authority,
admission and page faults remain terminal. Only a typed fresh `SessionEndedError`
permits the existing guarded `recoverSession`/reload fallback, whose successful
reload restores device-stored drafts only when storage worked. The card makes no
draft-persistence promise for that fallback. The guarded fallback marker still spans its reload: only its
explicitly started network failure clears the failed marker. Automatic guidance
keeps its marker until authenticated boot clears it.
The Remote restart and retained-draft cases in `acceptance/ask.spec.ts` verify
original-ID observation, no resend and fresh explicit Send after recovery.

`supervisor.rs` owns only a Remote group this serve started. Shutdown signals that
group before reaping its leader, then uses read-only absence checks. All reaps
and pipe/watcher joins share bounded cleanup deadlines; failure reports cleanup
unconfirmed without replacing the original startup error. An unexpected exit
is observed with rustix's safe `waitid(WNOWAIT)` while serving, so the leader
continues reserving its identity until group cleanup. Attached Remote is untouched.
Darwin's zombie-leader KILL EPERM is cleared only after successful bounded reap
and a read-only ESRCH, matching Core's process cleanup rule. The existing private
serve-error record carries a cleanup failure before the serve lock releases; stop
reports outcome unknown for that phase or an unreadable record. Missing/empty
records remain compatible with older serves. No success record or socket message
is added.

## Persistence layout

- The data root comes from one fixed `storage.root` API call through the absolute
  `$TMT_EXECUTABLE` (`core.rs`, the only core access). A missing or invalid root fails
  before any state is created; `spaces` creates nothing.
- State is `<dataRoot>/colab/` (owned 0700, no symlink) holding `owner.key`, `serve.lock`,
  `keyring.lock`, `space.db`, `settings.json`, `work-index.json` (a rebuildable page-work cache,
  under `work-index.lock`) and the socket `door.sock` (0600 files). The layout is the
  `tmt-extension-state` leaf behind `keyring::Layout`; Colab adds only its file names and
  error codes. A settings change is one `tmt_extension_state::Document` update (the shared lock
  in `<dataRoot>/document-locks/`, an atomic rename, a five-second wait whose refusal names the
  file and the exact `tmt colab settings` retry), so a reader takes no lock and a crash leaves the
  old file; a file past its 4,096-byte bound is refused, never replaced. `space.db` waits five
  seconds (`store::BUSY`) for another connection. `Keyring` publishes the owner seed create-only; an existing wrong-length key
  fails closed (`COLAB_KEY_INVALID`) and is never replaced. The seed never leaves
  `Keyring`; callers ask it to sign or seal.
- `space.db` schemas are append-only (`store/schema.rs`): 1 ciphertext (pages, streams,
  receipts, checkpoints), 2 owner authority (membership log, recipients, devices, epoch
  secrets, wraps, `owner_operations`), 3 `device_registrations`, 4 `baselines`,
  5 nullable checked server-observed content time on `pages`, 6 nullable original content
  publication scope on `owner_operations`, 7 page labels (replaced in 13), 8 bounded confirmed-publication metadata, 9 `link_admissions` (a live
  link's cloud admission token, dropped with the link), and 10 `link_publications` (links whose cloud
  row may exist, until their removal completes). 11 and 12 cache cloud rows and edit decisions; 13 `projects` and `page_projects`, which replace `page_labels`. Migration leaves legacy times null and legacy
  outcomes unscoped without backfill. Epoch
  secrets are local key material, not an encrypted-at-rest guarantee.
- `Store::open` creates and migrates, first copying an existing store (`schema::backup`:
  `space.db.pre-v<N>.bak`, 0600, partial file synced and renamed, the two newest schemas kept; a failed copy
  refuses to migrate with `COLAB_STORE_BACKUP_FAILED`) and, for a newer store, naming the newest copy this
  build can open in the `COLAB_STORE_NEWER` message (`schema::copies`; contract: Copy before a migration).
  `Store::read` and `Store::write_existing` open only
  existing 0600 state owned by the user, create nothing and never migrate;
  `write_existing` requires the serve lifecycle lock and exactly the current schema.
  Both existing-state paths check the version derived from `MIGRATIONS`; a future
  schema fails closed with `COLAB_STORE_NEWER` and the `tmt upgrade` instruction.
  Current-schema CLI inspection rejects an older migratable schema with
  `COLAB_STORE_OUTDATED` and "Run tmt colab stop, then tmt colab serve --background."
  Refusal never migrates; serve applies the existing migration history. Human
  errors omit schema numbers; JSON preserves both versions and a runnable `next`
  command array (`tmt colab stop`, then `tmt colab serve --background`, or `tmt upgrade`), including through
  operation-correlated errors. Human hint sentences stay separate.
- Receipts are create-only: an exact retry returns the stored receipt, a conflicting
  envelope freezes its stream. Checkpoint publication prunes a shared prefix only when
  every namespace with updates there has a checkpoint at the same sequence/hash; pinned
  checkpoints and receipt identities survive. `store/publication.rs` tracks
  a stable verified binding and own stream without copying payloads. Initial tracking
  starts from the current paired checkpoint and contiguous retained tail; its cut
  is pending, never presumed published. Exact acknowledgements/readbacks advance
  progress; protected update pruning stops there. The newest paired checkpoint
  remains available while historical pairs may be superseded. A wrong binding,
  uid, hash, sequence or epoch cannot advance progress, and missing history refuses.
  Metadata counts toward page capacity, is bounded by the namespace budget, and
  is deleted with page data. The metadata itself does no networking: `serve` attaches
  the native cloud-sync worker (`socket.rs` `with_cloud_sync`) only when its deploy
  declaration can be produced and the sync server is registered.
  Capacity returns `Fault::Capacity`; nothing is evicted.
- The shared create-only append samples the injected server clock only for new
  content, in the same transaction as its receipt. Page time is nondecreasing; exact
  replay, own/discussion/Ask updates, checkpoints and policy/epoch changes do not
  refresh it. A failed later operation receipt rolls back content and time together.
  `owner/bootstrap.rs` verifies the owner log and supplies one retention/expiry
  projection to inspection and mounted discovery. The expiry is the cloud's: the cloud-sync
  publisher reports each confirmed write of a page row (`Local::page_row_confirmed`) and
  `Store::confirm_page_row` keeps the latest expiry per page and epoch (`cloud_page_rows`,
  schema 11); nothing is derived from local activity, and unsigned dates remain display hints.
- The store never decides authority. Callers verify signatures, roles, sessions and epochs
  before an append (`Admission`, below).

## Owner-local transitions

- `fold.rs` verifies the retained owner hash chain and derives page and issuer policy from
  signed statements. A SQLite read snapshot supplies epoch keys, cuts, certificates,
  checkpoints and tails; objects are verified before they are decrypted, and content
  merges go through the isolated decoder. A baseline must match its signed descriptor.
- `transitions::Engine` owns one decoder per page and is the only signer of statements.
  `transitions/request.rs` is the dispatch seam for admitted `OwnerRequest` values;
  `membership.rs`, `links.rs`, `sharing.rs` and `epoch.rs` implement member, link,
  page-policy and epoch changes with the same atomic runner.
- Preparation (decoder work, baselines) happens outside the writer reservation; the commit
  rechecks head, page epoch, namespace cuts and device projections. A moving snapshot is
  retried three times, then returns `STALE_HEAD`.
- Signed statement, secrets, baseline, wraps, page epoch and the operation receipt commit in
  one transaction or not at all; `owner_operations` makes an exact retry return the saved
  outcome with its original head. Callers propagate mutation errors so everything rolls back.
- `page::commit_publication` reuses the immediate device transaction and a
  content savepoint for a verified sealed batch plus one scoped terminal outcome. Expected
  admitted rejection rolls back content/stream/receipt/device/time changes before retaining the rejection;
  unexpected failure rolls back the enclosing transaction. Original-key replay returns exact
  bytes after current writer admission, without effect fences, quota charge or time refresh.
  `publication_status` is a read-only original-key lookup with caller-supplied current authority;
  it never issues a chain or persists UNKNOWN. Shared capacity includes scoped outcomes in
  existing page budgets across epochs, without eviction. The contract owns wire details;
  CLI `page write`, its serving route and browser Save call it; Save reaches it through
  `Admission::commit_save`, not `LocalWrite` IPC (see
  [Page source and export](architecture-sync-decoder.md#page-source-and-export)).
- Link seeds are borrowed for key derivation and never persisted or returned
  (`transitions/links.rs`). While the seed is borrowed, the same transaction derives the link's
  devices, one per `link::Transport` (`door`, `hosted`: `link::device_chain`, byte-equal to the
  `authority-v1.json` `linkDevice` and `linkHostedDevice` oracles; `device_signer` and `device_id`
  are the same derivation), and keeps their chains as device rows naming the `link.add` statement
  (`put_link_device` in `membership.rs`); they are never registrations, so `local_streams` does not
  treat them as streams this machine writes. A reset or removal revokes the rows with the link;
  the cloud snapshot publishes every live one. Each transport has its own stream, so a door/hosted
  race cannot fork one stream (#3174).

## Admission and lock order

- `socket.rs` treats registration, session, pages, management, the reserved page-publish and
  page-compact and device-events routes and the reader challenge/session routes specially. The root-local
  management, page-publish and page-compact routes deny any request carrying a forwarded
  `tmt-device-context` or device-event header (`local_denied`), and Remote refuses to
  forward the reserved `/.tmt/` subtree from browsers.
- **Lock order: the sync lock before the `Registration` mutex** (`registration.rs`).
  Management serializes with sync first, so an owner change and the shutdown of matching
  live handles are one step.
- `registration::Registration` owns the store/keyring pair. It verifies both
  remote-owned extension-key certificates against the full forwarded owner context before
  signing. Remote owns pairing, cookies and grants.
- Pins (`page_pins` in schema 17, #4356) are display-only owner-store state beside projects: `page.pin`
  (`{pageId, pinned}`, empty `expectedRevision`) goes through `Engine::set_pinned` ->
  `Store::project_transaction`, no head change and no receipt. The list builder puts pinned, non-archived
  pages first in pin order with `pinned:true`; `drop_page_pin` runs in the archive transaction and
  `delete_page_data` removes the row, so unarchive restores nothing. `SPACE_PINS` (20) bounds them.
  The app keeps the owner's order for pinned pages (`pinnedFirst` in `projects.ts`, the quick switcher's
  `pin` rank when nothing is typed), `ManagementPort.setPinned` signs the request, and Home's
  `PagePinAction` and the page header's pin control (hidden on a phone, where the title needs the room) call it. `cli_pins.rs` is `pin`/`unpin`.
  Browser discovery preserves this display order, validates visible page IDs for uniqueness, and
  compares their membership with the sorted historical `pageIds` catalog independently of order.

- Projects (`projects.rs`, `projects` and `page_projects` in schema 13) are display-only owner-store
  state; a project exists on its own, so one may hold no page. Schema 13 turned every earlier page
  label into a project of the same name. The two writes, `page.projects` (whole-set replacements for
  up to 64 pages) and `project.manage` (create, rename, delete; the signed frame names the space scope
  page `SPACE_SCOPE_PAGE`), are management requests that bypass the engine's statement path
  (`Engine::set_projects` / `manage_project` -> `Store::project_transaction`, no head change, no
  receipt, last writer wins, all-or-none per request), and `page_list()` serves them as `projects`
  per entry and at the top level of `/api/pages` and the CLI catalog. They are never a log
  statement, so a project change never contends with sharing or member changes. `delete_page_data`
  removes a page's assignments (the project stays); an open page is one that still has its epoch
  secrets.
- Page summaries (`summary.rs`: comment count, pending-item count, the creator, the page's own
  work) are display-only values for the owner list that only the owner fold can read, so the serve keeps
  them in an in-memory `SummaryCache` keyed by each page's fingerprint (epoch, membership head,
  receipt and checkpoint counts, read in the same store transaction as `page_list()`). A list
  read replies at once with the fresh entries, omits `summary` elsewhere and schedules one
  single-flight background worker (`summary::drain`) over every stale page, most recently updated
  first (`recent_first`), reading that order again every `REFRESH_PAGES` pages; the worker folds
  through a second read-only `page::save::Source` (its own opener, `with_summary_source`), never
  the request path's store, keyring or decoder. A drain folds each page at most once (its `tried`
  set is keyed by page ID, so a page an agent keeps writing cannot hold it): a failed or timed-out
  page keeps no summary and waits behind pages not yet tried at its fingerprint. The cached `work` is the page's own
  (`work_index::page_work`, the same derivation the CLI index uses); the list read resolves each
  fresh page's row and parent across the fresh entries with `work_links::Graph`, so a followed
  page's stage is never cached. The same projection caches complete reading
  descriptors (`summary_readings.rs`) and the named pending passages with the latest comment
  excerpt (`summary_passages.rs`: `needs` ranked review, question, reading, proposal and capped at
  20, text made inert), never a second state owner. The cache retains at most
  `PAGES` entries, including failures; catalog reads prune deleted pages. The owner-only chapter-details
  route reads one current fingerprint without decoding on the request path; complete results are
  bounded by `SUMMARY_DETAILS_BYTES`, pending/failed entries remain unknown. The summary values are
  not persisted; the title is, by the next entry.
- Page titles (`page_titles.rs`, `page_titles` in schema 16) are the owner's local plaintext title
  projection of an active page. The two summary workers write it as a by-product of their fold
  (`summary::summarize_current`); CLI `inspection::ListTitles` reads current rows once and folds only
  missing/stale titles, also filling them through `TitleWriter`'s lazily opened `Store::write_existing`
  handle, which never creates or migrates state:
  the page's full fingerprint is captured before the fold and compared again inside the immediate
  `project_transaction`, so a concurrent edit, membership change, archive or delete installs
  nothing. A row keeps the head-less `page_progress` key, and `page_titles()` serves only a row whose key is current.
  A write error is ignored by design (the row is rebuildable); `page.archive` drops the row in its
  own transaction and `delete_page_data` drops it with the page. The serve calls
  `Registration::start_summaries` once at start (the same single-flight `summary::drain` a list read
  schedules), so a store upgraded to schema 16 fills without the list being opened; no backfill state.
  `Registration::pages` reads the current rows in the catalog's own transaction (`CatalogRead`) and
  `attach_to_catalog` adds them as `title` newest page first while the reply stays within
  `DISCOVERY_BYTES`; mounted `spaceHome` shows that title instead of the browser's hint.
- Revocation arrives as a revision-ordered device event: known devices use the owner
  transition, unknown IDs a local tombstone. Equal or older events and already-revoked
  devices write nothing and close no tunnel.
- `page/lines.rs` is the one definition of a page's numbered lines (LF records that keep their
  LF, a CR is content, an empty page is one empty line) and of a range's hash; the anchored patch
  and the ranged read both use it.
- `cli_read.rs` captures one existing `page::read` snapshot for whole source, exact line
  ranges and bounded literal grep/context blocks; revision and range hashes refer to that
  same snapshot. `--out` uses `export::publish_source` to publish exact bytes through the
  existing owned staging/exclusive-file owner, without overwriting a destination or
  changing full-fidelity export. Read projection carries no write or transport authority.
- `page patch` (`cli_patch.rs`, `page/patch.rs`) evaluates an anchored change (exact find,
  hash-fenced line range or unified diff) as a pure function of the source `page::read` folds
  to, then goes through `commit_source` (`main.rs`), the one whole-source path `page write`
  also uses, so limits, the markup check, the no-op and lost-reply replay are shared. The
  evaluated base is fenced by `PublishOptions::base_source_sha256`; a stale base or commit
  re-evaluates on the page as it is, at most 3 more times, unless `--expected-revision` fences.
  The browser Save's merge uses the same evaluator (`apply_hunks`, `conflicting_hunks`), so a
  hunk places by the same rules in both front doors.
- `PageUsage` (`page.rs`) is the page's usage: the tail count and bytes the write path refuses
  at, the checkpoints, and the whole state with its gzipped size from 256 KiB. `fold.rs` counts
  them once (`UsageCounts::add`, `MaterializationInput::usage`) for the refusal and for the
  report, so the two cannot disagree; `page read` returns it as `usage`.
- `page compact` (`cli_compact.rs`) asks the serve over `page/compact_ipc.rs` (route
  `page-compact`, `Server::compact_page` under the sync mutex) or, when stopped, runs
  `page::compact::request` under the lifecycle lock. `request` is the automatic combine with a
  trigger of one change plus the report, then `page::owner_fold::fold_others` (the owner fold,
  #3983): it combines other owner-member devices' streams as folds signed by the local writer and
  reports `folded` / `left` / `unfolded`; a stream a fold cannot cover stays and is named.
- `management.rs` holds strict DTOs and device-signature admission and adapts to the
  engine; it never writes authority tables or chooses baselines, cuts, wraps or epoch
  keys. The CLI (`cli_grammar.rs`, `cli_management.rs`, `inspection.rs`) is root-local:
  `ls`, `show`, `open`, `share mode/link/member/history`, `retention`, `archive`, `delete` and
  `project` (`cli_projects.rs`); `ls --project` filters in `cli_management.rs`.

## The page list (space home)

The home route `/` is the page list (`home.tsx`); there is no Today view. `/all`, the list's former
route, redirects to `/` with its search (`beforeLoad`), so an old link or history entry still lands
on it. The list reads the loader's `spaceHome()` pages through `useHomeSpace` (`home-space.ts`),
which fills a missing title by the bounded hint read; the ⌘K search is the header's control.
`summary_pending.rs` derives the shared pending target IDs (proposals first, readings, answer
groups, reviews) from the admitted owner fold and `page_check::summary`'s static-source quote
associations (200 ms projection bound), without executing authored code or trusting device labels.
The shared needs-you vectors pin native/browser parity. A missing summary is unknown, never a false
zero. Returning through the existing Home loader or reloading reads again; while a non-archived
page lacks a summary the visible list also reads every 2 s, at most 15 times (`home-fill.ts`;
`rereadInPlace` puts the window back, since the router scrolls to the top when a load renders).

`page-layers.ts` composes the existing question/review/reading owners for the page; the action
owners report lifetime separately from stale-outcome presentation fences.

`home.tsx` renders the list from the same pages;
`manage-page.tsx` holds the shared management dialog host. The filter lives in the route search
(`validateHomeSearch`: `q`, `project[]`, `by[]`, `sort`, `stage`, `archived`, `needs`, `scope`; the router merges its result
over the raw query, so it names every key and sets an unreadable one to `undefined`), never in
component state, so a reload, Back and the quick switcher share it; the editor (`page-list-search.tsx`, the shared
rich input with one `FilterTokenNode` for `project:` and `by:`) holds only a draft until Enter
applies it, and a row's project button or a token removal applies at once. `stage:` is a third suggestion kind that
is never a token (below). A row's `summary` (the contract's
Page summary section; `bootstrap.ts` validates it, including the work row and parent, and holds
`AUTHOR_BYTES` and the 2 MiB `DISCOVERY_BYTES`) feeds the separate creator, updated and comment-count metadata and the
`◆ Needs you` mark, and the `by:` filter and free text read the creator; a page without a
summary shows only what the list already knows, with no placeholder. The Needs you tab (a chip
in a list without tabs) sets `needs=true` in that search and keeps the
active pages whose summary is `waiting` (`needsYou > 0`; an unknown summary is never counted); it
combines with the project, creator and text filters, and another tab leaves it. `projects.ts` is pure: project grammar (the owner
store's), counts (`projectCounts` also lists the space's projects that no page is in), filtering and
sorting, and the whole-set `withProject` change. `applyProjectChanges` sends `PROJECT_REQUEST_PAGES`
(64) pages per request in ascending order, sequentially: each request is atomic on the owner,
the batch is not, and a failed batch reports "Updated N of M" so a retry converges. Creating,
renaming and deleting a project are one `project.manage` request each, whatever the number of
pages: the dialogs call `ManagementPort.manageProject`, so there is no batch to resume. `ManagementClient`
signs `page.projects` and `project.manage` requests with an empty `expectedRevision` (no view, head or
verification: projects are display-only and the reply is only checked for shape); a `project.manage`
frame names `SPACE_SCOPE_PAGE`. The space's project names reach the list as `SpaceHome.projects`
(the discovery reply's top-level `projects`), so an empty project shows in the page menu and the manager. Management controls (the Archived tab,
row actions, project dialogs) render only when the transport has `management`; a reader
keeps search, sort and filters because `projects` arrive in `/api/pages`. The project-scope pills and module menu share the route scope and its device-local preference; `project:` in the search narrows inside it. Search
suggestions count the pages the tab shows, while the manage dialog counts every page in the project
and says how many of them are archived.

`project-dialogs.tsx` presents the manager as separate project cards with active and archived
counts. Its New project action uses the same Home-owned creation state as the module menu.
The creation form names the project and optionally selects its initial pages, with archived
pages explicitly labeled. Rename and delete replace the manager with one centered shared dialog;
cancel returns to the named card action, and a successful write returns to the refreshed manager.
Only one native modal is active. Busy requests retain their existing dismissal fence, and merge,
archived impact and partial-write/refusal outcomes keep the same project contract and writer.

The list follows the concept website’s `/#/colab/pages` composition: a hero with New page, the owner's actual pinned pages in pin order,
then view tabs, project-scope pills, search and Sort, then the list. `page-list-items.tsx`
owns display-only pinned cards and rows; Home retains filtering, selection and all write lifetimes.
Pins wrap after three on a wide screen and stack on compact screens; no pins means no card block.
Project membership remains plural; `projectTone` maps exact names deterministically to existing role tints for header, pin and row tags; missing summaries never become a fabricated stage.
Wide rows have Title, Project, Stage, Updated, Author and Access columns, with compact task-count words under the title; compact rows stack metadata without sideways overflow. Unknown authors stay absent. The shared
flat tabs show separate numeric counts and wrap on compact screens, without a horizontal scroller. The `New page` action (`new-page-dialog.tsx`) still gives the
`tmt colab page create` command with `CopyCommand`; the browser creates no page.
The list fills the shell content pane using the shared `tmt-ui-page` insets. Home is its one vertical viewport: hero, pins, controls, rows and footer scroll together; `.page-list-scroll` adds no nested scroller. Successful action feedback stays in a visually hidden live region; refusals remain visible.
`useShownRows` (`page-list-rows.ts`) draws 50 rows and 50 more each time the count line
(`Showing N of M`) comes within a screen of Home's end. Home passes the count line explicitly as the observer sentinel; changing a filter/view/sort resets drawing and Home scroll to the top. Selection and its header checkbox
operate on the current matching pages, including rows not yet drawn.

`NewPageDialog` adds Close through the shared dialog footer; it offers the literal machine command,
not a browser-side create operation. `CopyCommand` owns the native copy button and its decorative
mark, trusted activation, disabled admission and clipboard feedback; denial keeps the command
selectable. The connection guide uses the same helper. `PageOperationDialog` likewise supplies
Cancel and its lifecycle action through the shared footer. It only confirms the operation and
names the first five titles for bulk actions; Home retains statement submission, busy dismissal
and verified, awaiting or refused outcomes. The shared dialog owns the divider and action layout.

The views are the tabs All, Needs you, Discussion, In progress, Done and Archived, in that order
(`BrowserTabs variant="flat"` with wrapping); `list-stage.ts` owns the stage vocabulary (`HOME_STAGES`,
`DEFAULT_VIEW`, `HomeView`, `pageStage`, `stageCounts`, `takeStage`). `stage` in the route search selects a stage or `all`, `archived`
selects Archived and wins over `needs`, which selects Needs you, and the absent key is `DEFAULT_VIEW`, All (a list without the
management port has no tabs and is every active page). A mounted address names the list as `/colab/` and carries none of this search, so `keepListSearch` (`list-history.ts`) stores it in the history entry's state and `parseLocation` reads it back: a reload, Back and Forward return to the list as it was left, and a fresh visit opens `DEFAULT_VIEW`. Each label counts what the scope holds: an archived page
counts only under Archived, and a page whose summary has not arrived (`pageStage` is `undefined`; a
fresh summary without `work` is in discussion) under no stage, so it is in All only. A stage is one
state with the search: a typed `stage:<name>` (`takeStage`, applied with Enter) or a suggestion pick
selects the tab and leaves the search text, and a tab click removes one still typed. A row shows its
progress and who works (`page-work.tsx`: the count as `done of total tasks done` and at most three
assignees from `summary.work`, the rest counted); an empty stage tab names its stage, an empty
In progress offers Discussion and All and never opens another tab by itself, and a stage tab says
`Reading page details…` while `useSummaryFill` is still reading (at most 15 re-reads) and a page in
the scope has not reported, so a cold start never claims a tab is empty. A page waiting on the viewer
shows a waiting-colour dot with `Needs you · N`: the badge retains its readable word/count, so
the state remains explicit without colour. The row's actions (open in a new tab, projects, archive, the ⋯
menu with `Manage page`, `Archive…` (or `Restore…` when archived) and `Delete…`) show on hover or focus where there is a pointer
and stay on touch; under 480px the new-tab and archive icons give way to the ⋯ menu. `Home` owns the open `ManagePage` dialog and its captured page ID/title above the rows: a list refresh that removes a row, including the last page after deletion, does not discard its pending request. Explicit Close or leaving the list ends that dialog. Archive and
delete confirm in `PageOperationDialog` and run `runPageOperation` (`page-operations.ts`): the four
steps `ShareDialog` runs (`read`, `prepare`, `send`, `verify`) for one `page.archive`, `page.unarchive` or
`page.delete` statement, ending `verified`, `awaiting` (acknowledged but its log not read back, said
so and never called done) or `failed` with the management error code. An archived row offers `Restore…` and `Delete…`. Restore requires an archived, non-deleted
page in the verified owner log. Its owner transaction preserves content, keys, epochs and access
policy while committing the statement, projection, monotonic `pages.last_activity_at_ms` reset and receipt together.
`pages.last_update_at_ms` remains the last content edit and is unchanged by restore.
Exact receipt replay does not refresh activity. Archive and restore end existing page sessions
with `RESYNC_REQUIRED`; replacement verifies current policy before writing and never submits
retained drafts automatically. The existing touched-page cloud activation republishes the open
state; confirmed cloud expiry remains the result of a confirmed publication, not a local guess.

Each row has a leading checkbox (`BrowserListRow`'s `leading` slot) when the transport has
`management`. The selection is a set of page IDs in `Home`; what counts is the selected pages the
list shows now, so a tab, scope or filter that hides one drops it. While one exists `BulkBar`
(`bulk-bar.tsx`) appears below the search/scope controls (the draft survives in `Home`): add to or remove from
a project (`withProject` per page and `applyProjectChanges` batches of `PROJECT_REQUEST_PAGES`; a
page already in eight projects is skipped and said so), `Archive…` (not in the Archived tab) and
`Delete…`. Under 480px (`useMedia`) the bar is one row: the count, an `Actions` menu with the same
four commands and `Clear`; the project choice then opens in `ProjectChoiceDialog`. A bulk archive or delete confirms with the count and the first titles, then
`runPageOperations` sends one signed statement per page in order, each through its own read, prepare,
send and verify; the first refusal stops the batch, `operationStatus` names the page and reports
`Updated N of M pages`, and the pages not reached stay selected. `usePageListKeys`
(`page-list-keys.ts`) maps `/` (search), `j`/`k` (move between rows; Enter and Ctrl/Cmd+Enter act on
the focused link as before), `x` (select), `e` (archive the selection, or the focused row) and `l`
(that row's projects menu, through `BrowserMenu`'s `openRequest`); Escape clears the selection, and
on an empty search it leaves the search. The keys fire only for a trusted unmodified press whose
target is the page or the list's own section: never in a text field, never above a menu or dialog,
never in a hidden list.

The TMT web shell's module menu (`colab-shell.tsx`, rendered by the leaf's `BrowserShell`) scopes
the whole list. `scope` in the route search is the base
filter: `inScope` (`project-scope.ts`) decides which pages feed the rows, the tab counts, the
`project:` suggestions and the free-text and creator filters, and a `project:` token narrows inside it
(intersection). The menu lists All pages and every project of the space (empty ones too, with the
pages shown and how many wait on the viewer), then `New project…` and `Manage projects…`, which
`Home` offers through `ProjectMenuActionsSlot` (`project-menu-actions.ts`) while it can manage
projects, so the dialogs keep the list's data. The menu reads the list through `spaceHome` when the
list opens. The default scope is device-local: `readScope`/`writeScope` keep it per space in
`localStorage` (a refused access means All pages), the home loader redirects an unscoped `/` to it
with `replace`, and choosing All pages removes it. A scope the space no longer has (a stale link or
a remembered project that was deleted) shows All pages and is neither redirected to nor rewritten.
The home hero names the current scope; `AppHeader` passes `titleVisible=false` through
`ColabHeader` to the shared header, retaining its accessible name and tools without another
visible heading. Other routes keep the default header title. A page
opened from a scoped list carries the scope in history state only, and its own header shows
`<project> / <title>` with the project linking back to `/?scope=<project>` (the shell bar draws no
module crumb); a copied or reloaded page link has no crumb. A heading narrower than 200px (a
container query on it) leaves the crumb and its divider out so the title keeps its room; the rail's
All pages (or a standalone header's brand link) still returns to the remembered scope.
`cli_management::selection` adapts public CLI inputs to strict existing DTOs;
its shared page resolver reuses the short-link helpers to admit unique UUID prefixes
from the verified complete catalog, including retained deleted IDs, before
read/write/export/management effects. Domain requests, JSON and confirmations keep
full IDs. Ambiguity lists authenticated titles and shortest candidate IDs;
missing/deleted prefixes refuse without changing state. Other ID operands stay strict.
`ls` keeps verified catalog metadata and membership-head consistency, but full content checks run
only for title misses. It omits layout fields; `show` retains full title/content/layout inspection.
Its work/parent display uses only current `work_index::cached` entries, without a second fold or
index write; work admission and explicit work reads retain their existing scan path.
Member removal/role changes capture complete verified assignments, and deletion requires `--yes`.
Explicit frozen delete retries bypass only the missing catalog view so the engine
can replay the retained receipt. Retention reads use verified policy without a decoder.
It uses the private socket IPC when `serve`
holds the lifecycle lock and the offline path otherwise; an uncertain IPC reply never
falls back to a second writer (`cli_management.rs`, `page/ipc.rs`). Management errors
are exact plain codes/statuses; malformed or mismatched replies remain uncertain.
A non-frozen CLI management intent may rebuild once after a verified pre-effect stale
head, revalidating the same target and confirmation scope; explicit IDs/revisions,
changed scope, a second stale head and unknown outcomes never rebuild (colab-v1 Local management CLI).

- `readers::Sessions` keeps at most 64 ephemeral challenges, tickets and active readers
  (`CAP`), with a one-minute challenge and ten-minute session. Readers are page- and
  epoch-scoped and never owner devices. A link session writes only through `sync::Access::Write` and
  `Append`: `Sessions::writer` reads the live link, so its role admits `content` and `own` (editor),
  `own` (commenter) or nothing (viewer), signed by the link device (`Admission::device`); the fold
  stays the authority on what a stored original means. No link has `Access::Save` or `Publish`: an
  editor link asks for a text change with an `edit-request` record (`Writer::edits` gates the
  serve applying it, below). Every other frame stays denied (`readers.rs`, `registration.rs`).
- `sync/edit_requests.rs` settles a link device's `edit-request` records (grammar:
  `edit_request.rs`): `Server::apply_edit_requests` reads the page's `own` projections from a fresh
  read-only view outside every lock, authorizes the writer through
  `Admission::authorize_link_save` (a live editor link, from the owner log; only `Denied` or
  `Expired` is the link's answer and refuses, any other error leaves the request undecided) and runs
  the request as a `SaveJob` with `for_request` through the same `run_save_job` and `finish_save` as
  a browser Save, answering no peer. Decisions live in the Store's `edit_decisions` (schema 12), at
  most `EDIT_DECISIONS` per epoch, checked before a request is saved. Two callers: a link session's
  connection after its own original is stored (the pages in `State::requests_due`, all settled by
  the next connection to poll), and the cloud pass (`Inbound::settle`) after it read new link
  originals or first read a page.

`quick-switcher-dialog.tsx` composes the website Search presentation over the existing
source and ranking owners. Its project menu edits the same `SearchEdit` as query chips;
there is no separate filter state. The result list keeps native links, plural memberships
and real author/waiting metadata, with a shared-dialog footer for keyboard hints.

The quick switcher's data (#2599) is React-free except one hook. `quick-switcher.ts` ranks the
list's own `PageSummary` rows for the list's own `PageFilter` (no second index or parser): match
strength of the free text, then a page waiting for you, then `max(opened, updated)`, ties by title
and ID, archived pages and an `exclude` page left out before the result `limit`. `recently-opened.ts`
is the device-local record behind "opened": one IndexedDB key per space (`recent-pages:<space>`),
the newest 100 page IDs with their open times, closed-shape validated, written by the page route
once per load under a Web Lock, never synced, exported or signed, and empty when storage is
unavailable. `quick-switcher-source.ts` reads `spaceHome()` and that record each time a switcher
opens, shows the last list of the same transport and space at once (the one a switcher read, or
the one the space home loader remembered with `rememberPages`) and refreshes it, and drops a
result that arrives after a close, a reopen or a change of transport or space.

## The dashboard

Code identifiers, files and stored values keep the older name `shell` (`ShellPage`, `shell-page.tsx`,
`meta.shell`); the UI, the CLI and the contract say dashboard.

A dashboard opens in the owner app as `ShellPage` (`shell-page.tsx`), mounted by the page route in place
of the HTML frame; the app owns every pixel and the document supplies only data. The look follows the
concept website's Team dashboard (#4784): the page's own head (its title as the h1, the greeting as
the subtitle), then one card ("Your pages, at a glance.") holding the maintainer agent's lines, three
count tiles and, below them, the Pages list beside the selected page. `shell-grid.ts` is the plain
per-item view: `shellCells` matches each item, in `order`, to its
source page in the space list (title, archived, last edit, Needs you count, author, first project); a
page the list does not show is `unavailable`, never linked, and a reference grants no access.
`shellByline` names the summary writer only by the Core-resolved display agent, never by identity,
and `pagesChangedSince` counts the distinct source pages edited after the summary.

`shell-model.ts` is the pure computation, given the cells, the owner chapter details and a clock:
the referenced pages are the distinct available, non-archived pages (a page named by several items
is one chapter); `waiting` is the sum of their `needsYou` (the number the page header shows); a page
not waiting and edited within seven days is in progress (`inProgress()`, the one place a page's work
stage can replace that rule); waiting chapters come first, then in progress, each by update time with
the dashboard's `order` as the tie-break; older pages fold into `earlier`. A chapter's passage is the
pending item an item names (a reading ID or an anchor equal to a pending ID), else the first pending
item by rank (review, question, reading, proposal), else the latest comment, else the item note;
`pending` only while a waiting page's details are unanswered. `shell-sections.tsx` draws the parts
(`ShellHead`, `ShellSummary`, `ShellCounts`, `ShellPages`, `ShellAside`). The Pages list numbers the
chapters, then the older pages under their own heading; each row is a button that selects its page
(`aria-current`; Enter or Space selects, the arrows only move focus), and the selected page beside the
list (the first one until a row is chosen, or the one a return from its page names) shows who and when,
its passage and the ways to open it: deciding first when something waits, else the page. The counts
select and focus the first row they count; where the two columns stack, choosing a row brings the
selected page into view.

A chapter's panel (`shell-panel.tsx`, `ChapterPanel` in the app's `BrowserDrawer`) opens from the
selected page's "Open to decide" button when the window is at least 900 px wide (`useWide` in
`shell-page.tsx`); narrower, that action is the same link to the page as before. It is the page
route's `open` search parameter (`openSearch` in `reading-link.ts`, a page ID, dropped otherwise), and
only a chapter of the dashboard opens one. Opening pushes a history entry; closing from a panel this
window opened steps back, and one that arrived by link only drops the parameter. The drawer returns
focus to the trigger (or the chapter's row after a reload). `withViewTransition` wraps open and close
in the browser's view transition when it exists and motion is not reduced. The panel shows the chapter
details' `needs` and `latest` as text and links to the page, the passage and the anchor, and holds no
page session of its own. Its first action, "Decide here", swaps that body for the page itself
(`EmbeddedPage` in `router.tsx` renders `OpenPage` with `embedded`, the same component the page route
renders): the compact bar of the page's header actions and status, the work band and the document. It
is panel state, not the address, so Back still closes the panel; "← Summary" goes back. The page
opens through the same transport call as the route and the panel owns that one live binding, closing it
when it leaves the page (summary, full page, Esc), so a page never holds two on the same own stream
and a second app instance is never needed. The embedded page's drawers and menus stay the app's
modal, viewport-level dialogs (top layer; Esc closes the topmost only); its `PageEnvironment` writes
no tab title or thread fragment and its `PageRouting` has no home link or crumb.

The page reads the space list once (`spaceHome()` through `useHomeSpace`, so a missing title is
filled by the same bounded hint read as the list); a failed read keeps every page as a row in the
older list with a notice. The selected page's actions are real `Link`s (`pageLinkKey` keeps the modified-open behaviour); a
passage that names a reading adds the page route's `read` hint, and an anchor item's element adds the
`anchor` search parameter (`pageSearch` in `reading-link.ts`). The page route then asks the renderer
to `reveal` it: the frame counts the elements with that exact id and, only when exactly one exists,
reports its top so the host scrolls there, finishing the scroll if the frame grows within two
seconds and the reader has not moved; anything else leaves the page open at the top with a short
notice. Before it leaves for a chapter the page records the page in its own history entry
(`shellFocus`), and the dashboard selects that page and restores focus to its row when the browser
comes back. The layout holds no per-user state; a dashboard whose value
was not admitted shows one state line instead of an empty page.

Passages come from the owner chapter details (`reading-details.ts`, the optional owner-only
transport port `readingDetails`) of the first `READING_DETAILS_PAGES` chapters, through
`ReadingDetailsQueue`: one read in flight, aborted when the page is left; a failed read is published
as `unavailable`, so a chapter never loads forever. The dashboard stores and writes none: opening a
chapter never marks a reading read (only the page's own `Mark read` does), and returning to the
dashboard reads again. Editing stays with the seven validated actions; the browser has no edit
controls, and the shared and hosted readers keep `ShellNotice`.

## Trusted browser management

- `management.ts` owns frozen device-signed selections, transient link seeds and
  projections from `Admission.statements()`. Its metadata reader reuses `Frames`
  for current baseline/statement chunk transport without opening content or a Worker.
  POST acknowledgments never change policy: verification requires the exact signed
  revision/hash and matching change, even when later owner commits exist.
- `mounted.ts` supplies one management facade through the mounted tab lifetime and
  current registration. Views and prepared requests retain their originating client;
  session replacement refuses old mutations, while acknowledgment verification is
  read-only under the new registration. It never opens another Remote session.
- `share-dialog.tsx` supplies the trusted modal for share/member/link/history and
  retention/archive/delete controls. Unexpected failures retain a bounded, redacted
  name/message cause and report it once without changing the displayed error code.
  `router.tsx` supplies active/archived home
  filtering and refresh after changes. Archive commits end only that page's sync peers
  with RESYNC_REQUIRED; paired tabs reopen with read catch-up, retaining drafts for Copy
  and removing publication controls. Other acknowledged or uncertain policy changes close
  the old Live binding, including its writer and Ask observer. Local samples have no management capability.
- Archive verification uses its same readable page. Delete uses another readable
  page's owner log plus discovery absence and the frozen initiating context.
  Without that evidence, last-page deletion remains acknowledged/awaiting
  verification. A dropped/DENIED target socket never erases the acknowledgment.
- `expiry.ts` formats browser relative retention time and local absolute dates;
  `retention-hint.tsx` presents the same hint inside home cards and the dialog.
  Warnings begin seven days ahead with a waiting mark and body text color; normal
  hints are dim. The CLI formats the same projection with one local clock per human
  result: relative last edits, short expiry values, and dim list hints under each link.
  Finite expiry within seven days has a waiting mark; its footer says "Expiry never
  deletes your local copy." JSON keeps exact milliseconds, and pages gone from the cloud remain
  available locally. Browser hints use the same lowercase wording (`expires in 6 days`,
  `not in the cloud`, `local only`); CLI values use UX's lowercase values: `kept forever`,
  `local only` and `not in the cloud`.
  Exact warning codes and forever semantics live in the contract.
  The link artifact is a transient ID/seed, not a new reader URL/import grammar.

## The work band

An ordinary page whose view carries `work` (`PageSnapshot.work`, then every live projection), or
whose list row names a parent, draws `WorkBand` (`work-band.tsx`) in the page route between the link bar and the canvas, so it scrolls
with the page and pushes it down when the list opens. The band itself is read-only: `derivedWork`
gives the stage, progress, workers and sign-off count; `work-list.tsx` lists the tasks and
`work-detail.tsx` shows one, with an `actions` slot, a `listActions` slot and an optional `parent`
breadcrumb for a page that follows a task (#4118), `Parent page › T-n · task title` with the title from the owner list row's `summary.parent.title` (absent from an older owner). Where this device may change the work
(`PageBinding.canWork`, the owner device's alone), `PageWorkBand` (`work-band-live.tsx`) fills the
slots from `work-controls.tsx`: state (with a reason for blocked), sign-off and send back, assign,
edit title and detail, convert to Colab (`task.convert`: the serve makes a page from the task and links it;
refused for a task that follows a page or has an external ID; a page made but not linked is named, with
"Link to that page" and "Open the page"), remove after a second click, and add a task by title. Each control sends one
closed action through `PageBinding.work`, which `Live` runs as a `work` frame (`Connection.work`,
answered by `workresult`; a lost reply is settled by one `savestatus`, never resent). Nothing is
applied locally: one action is in flight at a time, and the list changes when the page's own stream
brings the commit back. A refusal (`COLAB_WIP_LIMIT`, `COLAB_STALE_BASE`, …) is told in the serve's
words and changes nothing. A dashboard never draws it. The browser
takes a followed page's stage, and this page's parent, from the owner list's summaries
(`useLinkedPages`, `work-stages.ts`: the list the viewer last loaded, followed as other views refresh
it, plus one fresh read only when some task follows a page); a page the list does not name, or whose
summary has not arrived, leaves its task showing its own stored state, as the page list does. The
parent is the page's own row `summary.parent` and the parent row's title, drawn above the summary as
`<parent title> › T-n` linking to the parent page; a parent the list does not name draws nothing, and
nothing reads the parent page. A page without work draws the band only for that line. A band with a
summary and no task offers the owner the add-task field directly.
`work-people.tsx` is the one place an assignee is drawn (`AgentChip` for an agent).

## Short owner-page links

- `short_links.rs` and `short-links.ts` derive UUID prefixes from the existing
  complete verified catalog, including retained deleted IDs (ID/deleted flag only). Archive
  filtering and deletion never rebind a prefix; no alias state is stored.
- `socket.rs` admits the mounted alias and emits a same-mount relative redirect. Registration
  supplies metadata through the existing owner snapshot, without decoding titles. Anonymous
  requests retain only their requested prefix through root pairing/recovery guidance.
- `router.tsx` resolves the `/short/$prefix` chooser from its existing admitted transport;
  one live match canonicalizes to the full page route, deleted matches show disabled untitled
  rows or the deleted-page view, multiple matches show plain links and none
  use the unavailable-page view. The chooser and deleted view reuse `ColabHeader` through
  `AppHeader` and the shared `NoticeCard`; only their list rows own additional styling.
  `bootstrap.ts` preserves only the strict alias target on pinning.
- `Reach`/`Status` keep full JSON links and add `shortLink`; human output and auto-open use the
  Remote root alias. Human `ls` puts the page title first, with the same shortest unique catalog prefix as its link;
  JSON and `show` retain full IDs. Ask composition captures the catalog prefix in `Live`, preserving full
  signed scope, legacy source-link admission and unchanged reader links. See the contract for
  Remote's root-redirect dependency and exact URL/JSON shapes.
- Explicit `open [PAGE]` reuses this catalog/link boundary and the shared opener without
  starting services. JSON/no-open never launches; stopped Colab
  reports the serving next step. This does not replace browser admission.

## Browser title hints

- `title-cache.ts` owns optional encrypted title records scoped to space/device/page.
  `keyring.ts::titleKey` owns their local non-extractable AES-GCM key, without changing
  Remote-certified device keys. The existing IndexedDB record helper owns durable writes.
- `Live#publishViews` passes accepted folded titles and their exact admission
  registration to `mounted.ts`. Mounted tab ownership/current registration fences
  the best-effort cache write. Rejected folds and replaced sessions cannot supply hints.
- Home reads hints only after verified discovery/policy. `home-titles.tsx` admits visible
  uncached rows through a one-page queue (24 attempts and 16 MiB of incoming sync frames
  per Home generation). `mounted.ts::readTitle` uses the existing Admission/Connection/Fold
  path without Live, Ask, an editor or Subscribe. It serializes canceled generations until
  their read work joins; `TitleCache.fillMissing` cannot replace a foreground/cached hint.
- Paired Live and the shared reader explicitly Subscribe after catch-up. A valid
  Subscribe selects the cloud active page, as does every successful store; existing early
  write wakes are unchanged. A Home Hello/ACK does not select it.
- `router.tsx` and `share-dialog.tsx` use them as display labels, with UUIDs under Details and the
  explicit unopened fallback; the parent tab title follows the live page. A later
  foreground fold replaces stale hints; cached titles changed elsewhere remain stale until
  opened. Missing/corrupt cache data never authorizes or denies access.
- The cache adds no plaintext server field, native schema or Worker/renderer capability.
  Crypto/scope tests live in `test/title-cache.test.ts`; real IndexedDB reload and
  safe rendering are covered in `e2e/live.spec.ts`, and native page-create title
  propagation/profile isolation in `acceptance/titles.spec.ts`.
