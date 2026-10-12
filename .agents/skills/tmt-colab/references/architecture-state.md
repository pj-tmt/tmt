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

- Run Rust gates with your own `CARGO_TARGET_DIR` and `CARGO_BUILD_JOBS=2`. Add
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
its open elements on every start tag), and `question.rs`, `group.rs` and `image.rs` hold the rule
tables (`group.rs` also decides which groups are skipped and which IDs are held). It does no I/O; `cli_page_check.rs` adapts it to the commands. The browser's `inspectQuestions`
(`src/questions.ts`) is the other implementation of the question and group rules. Both run
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
group selection and verified-thread projection, and `components/question-card.tsx`
controls. It reuses the shared BrowserTabs leaf. `inline-slots.ts` owns the one
measurement batch for proposal and answer cards; renderer-managed visual collapse
keeps inactive authored questions in the canonical quote text map. The render-bound
group/slot shapes and publication rules live in [Answer blocks](../../../../extensions/tmt-colab/contracts/colab-v1.md#answer-blocks).

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
caller tokens), so the composer's mentions and the page list's `label:` tokens share one
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
immutable one-comment send capture. The page-owned `creator-default.tsx` preference
shares admitted defaults across composers; mention-node provenance is editing-only
and the unchanged encrypted v1 draft format restores mentions as manual. `message-recipient.ts` resolves exact typed names
and distinct UUID pairs for presentation only; LiveAsk re-admits each frozen Ask.
The [browser discussion contract](../../../../extensions/tmt-colab/contracts/colab-v1.md#inline-annotation-conversations-1587)
owns default-creator lookup, typed/picked binding, eight-recipient fan-out, offline
handling and recipient failure rules. `conversationAsks` owns comment/Ask association
and captured conversation; neither prior replies nor display names select recipients.
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
(`ReaderSession`) derives the link keys and the link's one reader device from the seed
(`link.deriveLink`, `link.deriveDevice`, `link.certifyDevice`; byte-identical chain on every open), runs challenge, session and sync, and reconnects until access ends.
It reuses `Admission` through its `ReaderSeat` option (link-addressed wraps, owner-log
verification, nothing persisted) and `Connection` with the ticket subprotocol. `src/reader-main.tsx`
removes the fragment first; `src/reader-app.tsx` renders read-only with the sandboxed renderer. The
owner router, writer, Ask and export modules are not part of this bundle.

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
three reopens a minute), then hands the new binding over. `ended` replaces
the page with one card: the failure sentence and the one action that fits (`Pair again` with the
command for access ended, revoked, expired, unpaired and mismatch; a way back to the space for a
page that is gone; `Reload` for a device at its Remote session limit; none otherwise). A modal drawer makes that card inert, so a live Chat
composer owns its ended sentence/action once; an archived Chat and every other drawer keep
the same sentence/action in the `notice` row under the header. An ended annotation keeps its
action scroll- and keyboard-reachable within the existing composer overflow. `Live` stays
the transport owner: it keeps its bounded resync
and publishes failures, and no longer decides terminal or recoverable for the page.
One table, `connection-state.ts`, classifies every token the live path can fail with (sync frame
codes, Remote refusal codes, a few short messages, each `ReopenFailedError` reason): what recovers
it (`resync` on the same session, replace the `session`, `reopen` the page for a new epoch,
`retry` a fresh open, or `none`) and which sentence it ends with. `live.ts`, `readmit.ts`,
`connection-model.ts` and `terminal-failure.ts` read it; none matches raw strings. A signed
`REMOTE_CLOSED`, `REMOTE_GRANT_CHANGED` or `REMOTE_SERVE_STOPPED` replaces the session once,
silently; the access card shows only when that fresh open is refused (#2619).
A terminal failure card shows one of seven `strings.ts` sentences (access ended, not paired, page
gone, session ended, too large, session limit, generic), never the raw token (`terminal-failure.ts`); an unknown token is not
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
envelopes deduplicate as `Replay`. If automatic recovery fails, the page's own bounded
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
The same check also reads `GET api/serve-release` (`serve_release.rs`, contract): when the serve is
older than the release installed beside it, the row says to restart `tmt colab serve` (naming both
versions, no Reload, `data-colab-update='serve'` for its two-line height) and the serve prints one
terminal warning. A reload cannot help, so nothing restarts or reloads itself.
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
  publication scope on `owner_operations`. Migration leaves legacy times null and legacy
  outcomes unscoped without backfill. Epoch
  secrets are local key material, not an encrypted-at-rest guarantee.
- `Store::open` creates and migrates. `Store::read` and `Store::write_existing` open only
  existing 0600 state owned by the user, create nothing and never migrate;
  `write_existing` requires the serve lifecycle lock and exactly the current schema.
  Both existing-state paths check the version derived from `MIGRATIONS`; a future
  schema fails closed with `COLAB_STORE_NEWER` and the `tmt upgrade` instruction.
  Current-schema CLI inspection rejects an older migratable schema with
  `COLAB_STORE_OUTDATED` and "Start or restart tmt colab serve to update it."
  Refusal never migrates; serve applies the existing migration history. Human
  errors omit schema numbers; JSON preserves both versions and a runnable `next`
  command array (`tmt colab serve` or `tmt upgrade`), including through
  operation-correlated errors. Human hint sentences stay separate.
- Receipts are create-only: an exact retry returns the stored receipt, a conflicting
  envelope freezes its stream. Checkpoint publication prunes a shared prefix only when
  every namespace with updates there has a checkpoint at the same sequence/hash; pinned
  checkpoints and receipts survive. Capacity returns `Fault::Capacity`; nothing is evicted.
- The shared create-only append samples the injected server clock only for new
  content, in the same transaction as its receipt. Page time is nondecreasing; exact
  replay, own/discussion/Ask updates, checkpoints and policy/epoch changes do not
  refresh it. A failed later operation receipt rolls back content and time together.
  `owner/bootstrap.rs` verifies the owner log and supplies one retention/expiry
  projection to inspection and mounted discovery; unsigned dates remain display hints.
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
  (`transitions/links.rs`).

## Admission and lock order

- `socket.rs` treats registration, session, pages, management, the reserved page-publish and
  device-events routes and the reader challenge/session routes specially. The root-local
  management and page-publish routes deny any request carrying a forwarded
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
- Revocation arrives as a revision-ordered device event: known devices use the owner
  transition, unknown IDs a local tombstone. Equal or older events and already-revoked
  devices write nothing and close no tunnel.
- `management.rs` holds strict DTOs and device-signature admission and adapts to the
  engine; it never writes authority tables or chooses baselines, cuts, wraps or epoch
  keys. The CLI (`cli_grammar.rs`, `cli_management.rs`, `inspection.rs`) is root-local:
  `ls`, `show`, `open`, `share mode/link/member/history`, `retention`, `archive`, `delete` and
  `label` (`cli_labels.rs`).

## Page list

`home.tsx` renders the space home from the loader's `spaceHome()` pages; `manage-page.tsx`
holds the shared management dialog host. The filter lives in the route search
(`validateHomeSearch`: `q`, `label[]`, `sort`, `archived`), never in component state, so a
reload, Back and the quick switcher share it; the editor (`page-list-search.tsx`, the shared
rich input with a `LabelFilterNode` token) holds only a draft until Enter applies it, and a
chip click or token removal applies at once. `labels.ts` is pure: label grammar (the owner
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
  epoch-scoped and never owner devices or writers (`readers.rs`).

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
  deletes your local copy." JSON keeps exact milliseconds, and expired local pages remain
  available. Browser hints use the same lowercase wording (`expires in 6 days`, `expired 2 days ago`,
  `expiry starts after the next edit`); CLI values use UX's lowercase values: `kept forever`, `starts after the next edit`,
  and `beyond the supported range`. A verified out-of-range warning masks the human retention count
  as `out of range`, without changing JSON or policy.
  Exact warning codes, checked arithmetic and forever semantics live in the contract.
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
- Home reads hints only after verified discovery/policy. `router.tsx` and
  `share-dialog.tsx` use them as display labels, with UUIDs under Details and the
  explicit unopened fallback; the parent tab title follows the live page. A later
  fold replaces stale hints. Missing/corrupt cache data never authorizes or denies access.
- The cache adds no plaintext server field, native schema or Worker/renderer capability.
  Crypto/scope tests live in `test/title-cache.test.ts`; real IndexedDB reload and
  safe rendering are covered in `e2e/live.spec.ts`, and native page-create title
  propagation/profile isolation in `acceptance/titles.spec.ts`.
