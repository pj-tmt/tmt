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

- Run Rust gates with your own `CARGO_TARGET_DIR` and `CARGO_BUILD_JOBS=2`. Add
  `--no-fail-fast` when judging `cargo test -p tmt-colab`: Cargo stops at the first failing
  test binary and hides the rest. Timing-sensitive decoder tests can fail under load; rerun
  them alone before treating one as a regression.
- Run the architecture test on every Rust push:
  `cargo test --offline --locked -p tmt-cli --test architecture`.
- Regenerate frozen vectors only with their Python oracle in a throwaway virtualenv with
  `cryptography`, never `--write` outside a reviewed regeneration.

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

The main app uses one Colab-local `components/message-composer.tsx` for Chat,
annotations, agent follow-ups, plain replies and comment edits. Exact 0.52.0
`lexical`, `@lexical/react`, `@lexical/plain-text` and `@lexical/history` are app-only
runtime dependencies; only plaintext/history extensions are imported. The source
editor is a separate editing mode. Reader and recovery entries have no composer.

`ComposerEdit` emits one atomic snapshot of exact plaintext, optional parent-selected
machine/agent identity and cosmetic mention-token range. Paragraphs serialize with
one LF, retaining blank and trailing lines. Parent echoes preserve the editing
history/selection; explicit reset keys end a draft's editing lifetime. Mention-node
identity follows edits and undo; editing/removing the token invalidates its cosmetic
metadata without changing recipient authority. No Lexical document is persisted or
sent as a routing instruction.

`AnnotationInput` owns the plaintext draft, selected recipient, trusted action,
current write/Ask admission and immutable send capture. `messageRecipient` is a pure
parent presentation policy: plain comments resolve before agent discovery, while an
explicit Ask uses a current stable destination. Publisher display-name metadata can
supply a default only through one unique current admitted candidate. Prior replies
use UUID/machine keys, never display labels. Ambiguous/stale choices do not silently
retarget. `conversationAsks` owns comment/Ask association for display, reply defaults
and captured conversation; the editor has no storage, ledger, notification or Remote
capability. Content-write and Ask failures retain the existing draft/recorded-turn
and uncertainty rules; recipient selection performs no preparation or dispatch.

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
retries an Ask. On an open page's socket close, `Live` makes one read-only old-session
probe: signed session end silently reopens that tab; signed eviction stops it with the
limit notice. Transport failures remain distinct and get bounded sync catchup. If recovery
fails, the explicit Reconnect button uses `recoverSession` through `Live.reconnect`, closing
the page socket, Ask and observer first. The Remote restart cases drive that explicit path
through `reconnect(page)` in `acceptance/ask.spec.ts`.

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
  5 nullable checked server-observed content time on `pages`. Migration leaves legacy
  times null without backfill. Epoch
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
- Link seeds are borrowed for key derivation and never persisted or returned
  (`transitions/links.rs`).

## Admission and lock order

- `socket.rs` treats registration, session, pages, management, the reserved page-write and
  device-events routes and the reader challenge/session routes specially. The root-local
  management and page-write routes deny any request carrying a forwarded
  `tmt-device-context` or device-event header (`local_denied`), and Remote refuses to
  forward the reserved `/.tmt/` subtree from browsers.
- **Lock order: the sync lock before the `Registration` mutex** (`registration.rs`).
  Management serializes with sync first, so an owner change and the shutdown of matching
  live handles are one step.
- `registration::Registration` owns the store/keyring pair. It verifies both
  remote-owned extension-key certificates against the full forwarded owner context before
  signing. Remote owns pairing, cookies and grants.
- Revocation arrives as a revision-ordered device event: known devices use the owner
  transition, unknown IDs a local tombstone. Equal or older events and already-revoked
  devices write nothing and close no tunnel.
- `management.rs` holds strict DTOs and device-signature admission and adapts to the
  engine; it never writes authority tables or chooses baselines, cuts, wraps or epoch
  keys. The CLI (`cli_grammar.rs`, `cli_management.rs`, `inspection.rs`) is root-local:
  `ls`, `show`, `open`, `share mode/link/member/history`, `retention`, `archive` and `delete`.
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
  retention/archive/delete controls. `router.tsx` supplies active/archived home
  filtering and refresh after changes; acknowledged or uncertain policy changes close the old
  Live binding, including its writer and Ask preview/observer. Local samples have
  no management capability.
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
