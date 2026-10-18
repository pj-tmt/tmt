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

Attachment descriptor/manifest/selector/publication syntax and asset verification
live in model `attachment.rs` and client `attachment.ts`, reusing existing crypto;
[attachment-v1](../../../../extensions/tmt-colab/contracts/attachment-v1.md) owns the bytes.
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
`reader.rs` discovers the signed authority and one foreign stream's log tail, and
`inbound.rs` (`Puller`) compares the cloud's membership log with the owner log, then follows
each foreign stream from the Store's `stream_head`; the worker reads only when a read is due (a
newly watched page, a notice, an unfinished read), and chains are read again at most once per
`CHAIN_RECHECK` after a log notice. `inbound/store.rs` admits an original with
`Snapshot::open_stream_object` (the fold that reads stored objects), keeps the writer's verified
chain for later catch-up, and applies it through
`Server::ingest`, the split of the browser append path, so one apply path serves both and nothing
that arrived is queued for publication. A refused writer stops alone, per page and epoch; `stops.rs` keeps the
page's stopped state in the worker's memory and the lines announce each transition.
Checkpoint import, adopting a cloud-only log, forks, attachments and a second machine's own
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
owns in-tab drafts, visible recipients and the shared footer; `question-card.tsx`
is controlled answer/conversation presentation. BrowserTabs and BrowserChips remain
presentation leaves. `inline-slots.ts` owns the one
measurement batch for proposal and answer cards; renderer-managed visual collapse
keeps inactive authored questions in the canonical quote text map. A chip-form group
(`data-form="chip"`) renders through `question-layer.tsx` as the parent-drawn
`components/question-chip.tsx` over a frame placeholder, with its panel kept mounted while
collapsed; `open(id)` is the one entry that opens it, for the chip and for later callers. The render-bound
group/slot shapes and publication rules live in [Answer blocks](../../../../extensions/tmt-colab/contracts/colab-v1.md#answer-blocks).

Review sections ride the same renderer handshake and slot batch: `review-layer.tsx` draws the
parent bands and `review-actions.ts` publishes the explicit actions through the existing thread,
Ask and status owners; see [Review sections](../../../../extensions/tmt-colab/contracts/colab-v1.md#review-sections).
Recommended-reading metadata uses the same inert parser and shared check vectors. `reading.ts`
projects person read state from ordinary verified thread records; `reading-layer.tsx` reuses the shared NeedsYouBar and exposes its ordered read projection and
cosmetic navigation; `reading-actions.ts` records
and resolves through the existing binding/coordinator. Its named-agent restriction excludes
other prose mentions. `reading-link.ts` owns cosmetic owner/paired navigation hints; Ask capture
accepts only that exact hint and removes it from the canonical admitted URL. Reader capabilities
are unchanged.

`needs-you.ts` combines the admitted layers' pending navigation without a stored count;
`needs-you-header.tsx` owns only its header menu. Layer-owned jumps scroll/focus existing
targets and open question chips; they never publish an action or create a thread.

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
caller tokens), so the composer's mentions and the page list's `label:`/`by:` tokens share one
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

Remote keeps sessions and door cookies in memory, so after a restart a paired browser's
reload receives Colab's private guidance page. That page loads the public `assets/recovery.js`
(`src/guidance.ts`), which lets Remote's SDK check its paired key and call
`reopenSession()` for that tab, then reloads. A session-storage marker
(`colab-recovery:<mount path>`, `src/session-recovery.ts`) spans that reload so a second
guidance response cannot loop; authenticated boot (`mounted.ts`) clears it. A failed or
refused reopen, or unavailable session storage, leaves plain pairing guidance and never
retries an Ask. On an open page's socket close, `Live` makes one read-only probe of the exact old
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
admission refusals never extend the transport loop. `ended` replaces
the page with one card: the failure sentence and the one action that fits (`Pair again` with the
command for access ended, revoked, expired, unpaired and mismatch; a way back to the space for a
page that is gone; `Reload` for a device at its Remote session limit; none otherwise). A modal drawer makes that card inert, so a live Chat
composer owns its ended sentence/action once; an archived Chat and every other drawer keep
the same sentence/action in the `notice` row under the header. An ended annotation keeps its
action scroll- and keyboard-reachable within the existing composer overflow. `Live` stays
the transport owner: it keeps its bounded resync
and publishes failures, and no longer decides terminal or recoverable for the page.
One table, `connection-state.ts`, classifies every token the live path can fail with (sync frame
codes, Remote refusal codes, HTTP admission refusals, a few short messages, each `ReopenFailedError` reason): what recovers
it (`resync` on the same session, replace the `session`, `reopen` the page for a new epoch,
`retry` a fresh open, or `none`) and which sentence it ends with. `live.ts`, `readmit.ts`,
`connection-model.ts` and `terminal-failure.ts` read it; none matches raw strings. A signed
`REMOTE_CLOSED`, `REMOTE_GRANT_CHANGED` or `REMOTE_SERVE_STOPPED` replaces the session once,
silently; the access card shows only when that fresh open is refused (#2619).
A terminal failure card shows one of seven `strings.ts` sentences (access ended, not paired, page
gone, session ended, too large, session limit, generic), never the raw token (`terminal-failure.ts`); an unknown non-admission token is not
recovered, gets the generic sentence and stays visible, bounded to 120 characters, as a muted
`Code: …` reference. A test requires a decision for every `SYNC_ERROR_CODES` and
`REMOTE_REFUSAL_CODES` entry, so a new code cannot ship as a raw headline or an accidental retry.
Eviction and management-changed keep their own cards. A device at its Remote session limit
(`REMOTE_SESSION_LIMIT`: Remote never evicts an attached session, #2585) is refused with the
verified `limit` and `settingsUrl`; `sessionLimit()` (`registration.ts`) turns the boot open's raw
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
the page moves down instead of being covered. An offline read, an unreadable page, or a page of
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
  `keyring.lock`, `space.db` and the socket `door.sock` (0600 files). The layout is the
  `tmt-extension-state` leaf behind `keyring::Layout`; Colab adds only its file names and
  error codes. `Keyring` publishes the owner seed create-only; an existing wrong-length key
  fails closed (`COLAB_KEY_INVALID`) and is never replaced. The seed never leaves
  `Keyring`; callers ask it to sign or seal.
- `space.db` schemas are append-only (`store/schema.rs`): 1 ciphertext (pages, streams,
  receipts, checkpoints), 2 owner authority (membership log, recipients, devices, epoch
  secrets, wraps, `owner_operations`), 3 `device_registrations`, 4 `baselines`,
  5 nullable checked server-observed content time on `pages`, 6 nullable original content
  publication scope on `owner_operations`, 7 page labels, 8 bounded confirmed-publication metadata, 9 `link_admissions` (a live
  link's cloud admission token, dropped with the link), and 10 `link_publications` (links whose cloud
  row may exist, until their removal completes). Migration leaves legacy times null and legacy
  outcomes unscoped without backfill. Epoch
  secrets are local key material, not an encrypted-at-rest guarantee.
- `Store::open` creates and migrates. `Store::read` and `Store::write_existing` open only
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
  checkpoints and receipt identities survive. Dormant `store/publication.rs` tracks
  a stable verified binding and own stream without copying payloads. Initial tracking
  starts from the current paired checkpoint and contiguous retained tail; its cut
  is pending, never presumed published. Exact acknowledgements/readbacks advance
  progress; protected update pruning stops there. The newest paired checkpoint
  remains available while historical pairs may be superseded. A wrong binding,
  uid, hash, sequence or epoch cannot advance progress, and missing history refuses.
  Metadata counts toward page capacity, is bounded by the namespace budget, and
  is deleted with page data. No serve call enables this tracking or cloud sync yet.
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
  CLI `page write` and its serving route call it; browser Save does not.
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
- Page labels (`labels.rs`, `page_labels` in schema 7) are display-only owner-store state: the
  one write `page.labels` is a management request that bypasses the engine's statement path
  (`Engine::set_labels` -> `Store::label_transaction`, no head change, no receipt, last writer
  wins, all-or-none per request) and `page_list()` serves them as `labels` in `/api/pages` and
  the CLI catalog. They are never a log statement, so a label change never contends with
  sharing or member changes. `delete_page_data` removes a page's rows; an open page is one
  that still has its epoch secrets.
- Page summaries (`summary.rs`: comment count, pending-item count, the creator) are
  display-only values for the owner list that only the owner fold can read, so the serve keeps
  them in an in-memory `SummaryCache` keyed by each page's fingerprint (epoch, membership head,
  receipt and checkpoint counts, read in the same store transaction as `page_list()`). A list
  read replies at once with the fresh entries, omits `summary` elsewhere and schedules one
  single-flight background worker over at most `REFRESH_PAGES` stale pages; the worker folds
  through a second read-only `page::save::Source` (its own opener, `with_summary_source`), never
  the request path's store, keyring or decoder. A failed or timed-out page keeps no summary and
  waits behind pages not yet tried at its fingerprint. The same projection caches complete reading
  descriptors (`summary_readings.rs`), never a second state owner. The cache retains at most
  `PAGES` entries, including failures; catalog reads prune deleted pages. The owner-only chapter-details
  route reads one current fingerprint without decoding on the request path; complete results are
  bounded by `SUMMARY_DETAILS_BYTES`, pending/failed entries remain unknown. Nothing is persisted.
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
  trigger of one change plus the report; it never touches another device's stream.
- `management.rs` holds strict DTOs and device-signature admission and adapts to the
  engine; it never writes authority tables or chooses baselines, cuts, wraps or epoch
  keys. The CLI (`cli_grammar.rs`, `cli_management.rs`, `inspection.rs`) is root-local:
  `ls`, `show`, `open`, `share mode/link/member/history`, `retention`, `archive`, `delete` and
  `label` (`cli_labels.rs`).

## The page list (space home)

The home route `/` is the page list (`home.tsx`); there is no Today view. `/all`, the list's former
route, redirects to `/` with its search (`beforeLoad`), so an old link or history entry still lands
on it. The list reads the loader's `spaceHome()` pages through `useHomeSpace` (`home-space.ts`),
which fills a missing title by the bounded hint read; the ⌘K search is the header's control.
`summary_pending.rs` derives the shared pending target IDs (proposals first, readings, answer
groups, reviews) from the admitted owner fold and `page_check::summary`'s static-source quote
associations (200 ms projection bound), without executing authored code or trusting device labels.
The shared needs-you vectors pin native/browser parity. A missing summary is unknown, never a false
zero. Returning through the existing Home loader or reloading reads again; no new polling loop.

`page-layers.ts` composes the existing question/review/reading owners for the page; the action
owners report lifetime separately from stale-outcome presentation fences.

`home.tsx` renders the list from the same pages;
`manage-page.tsx` holds the shared management dialog host. The filter lives in the route search
(`validateHomeSearch`: `q`, `label[]`, `by[]`, `sort`, `archived`, `needs`; the router merges its result
over the raw query, so it names every key and sets an unreadable one to `undefined`), never in
component state, so a reload, Back and the quick switcher share it; the editor (`page-list-search.tsx`, the shared
rich input with one `FilterTokenNode` for `label:` and `by:`) holds only a draft until Enter
applies it, and a chip click or token removal applies at once. A row's `summary` (the contract's
Page summary section; `bootstrap.ts` validates it and holds `AUTHOR_BYTES` and
`DISCOVERY_BYTES`) feeds the `by <creator> · updated · N comments` metadata and the
`◆ Waiting for you` mark, and the `by:` filter and free text read the creator; a page without a
summary shows only what the list already knows, with no placeholder. The `Needs you N` chip beside
the label chips sets `needs=true` in that search and keeps the pages whose summary is `waiting`
(`needsYou > 0`; an unknown summary is never counted); it combines with the label, creator and
text filters, and the list's clear action removes it. `labels.ts` is pure: label grammar (the owner
store's), counts, filtering and sorting, the whole-set `withLabel` change and the
`relabel` planner that rename and delete use over every page carrying the name, archived
included, whatever the list shows. `applyLabelChanges` sends `LABEL_REQUEST_PAGES`
(64) pages per request in ascending order, sequentially: each request is atomic on the owner,
the batch is not, and a failed batch reports "Updated N of M" so a retry (which re-reads the
space and plans only what remains) converges. `ManagementClient.setLabels` signs a
`page.labels` request with an empty `expectedRevision` (no view, head or verification: labels
are display-only and the reply is only checked for shape). Management controls (the page
menu, list options, label dialogs) render only when the transport has `management`; a reader
keeps search, sort and filters because `labels` arrive in `/api/pages`. The list-options gear is
icon-only, so the shown set names itself in the `page-list-status` line ("Archived pages");
the label chips count the pages shown, while the manage dialog counts every page carrying the
name and says how many of them are archived.
`cli_management::selection` adapts public CLI inputs to strict existing DTOs;
its shared page resolver reuses the short-link helpers to admit unique UUID prefixes
from the verified complete catalog, including retained deleted IDs, before
read/write/export/management effects. Domain requests, JSON and confirmations keep
full IDs. Ambiguity lists authenticated titles and shortest candidate IDs;
missing/deleted prefixes refuse without changing state. Other ID operands stay strict.
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

## The shell page

A shell opens in the owner app as `ShellPage` (`shell-page.tsx`), mounted by the page route in place
of the HTML frame; the app owns every pixel and the document supplies only data. `shell-grid.ts`
is the plain per-item view: `shellCells` matches each item, in `order`, to its source page in the
space list (title, archived, last edit, Needs you count); a page the list does not show is
`unavailable`, listed but never linked, and a reference grants no access. `shellByline` names the
summary writer only by the Core-resolved display agent, never by identity, and `pagesChangedSince`
counts the distinct source pages edited after the summary (unknown times never count).

The page reads the space list once (`spaceHome()` through `useHomeSpace`, so a missing title is
filled by the same bounded hint read as the list); a failed read keeps every item as a link with
an "unavailable" notice. Cells are real `Link`s (`pageLinkKey` keeps the modified-open
behaviour); a reading item adds the page route's `read` hint. Before it leaves for an item the page
records the item in its own history entry (`shellFocus`), and the shell restores focus to it when
the browser comes back. The layout is borderless and wide (grid columns, no card or shadow) and
holds no per-user state. Editing stays with the seven validated actions; the browser has no edit
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
