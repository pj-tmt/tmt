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

- `assets/recovery.js` reuses tab coordination for private guidance.
- The read-only reader (`public/reader.html` served at `/read`, `src/reader-main.tsx`) builds as fixed-name
  `assets/reader.js`, `reader.css` and `reader-fold.js` (the decoder worker), so the native allowlist
  `assets::anonymous_file` is exact. Add a file to the reader entry only together with that list,
  the contract's anonymous-asset sentence and `served.spec.ts`.

Only these files and `renderer.html` are public; the rest of the app stays owner-gated. Verify the
guidance CSP and pairing failure/reload guard alongside the app lifecycle tests (`served.spec.ts`,
`session-recovery.test.ts`).

## Read-only reader

`src/reader-link.ts` parses the fragment (strict grammar in the contract), `src/reader.ts`
(`ReaderSession`) derives the link keys with `link.deriveLink`, certifies a fresh in-memory device
(`link.certifyDevice`), runs challenge, session and sync, and reconnects until access ends.
It reuses `Admission` through its `ReaderSeat` option (link-addressed wraps, owner-log
verification, nothing persisted) and `Connection` with the ticket subprotocol. `src/reader-main.tsx`
removes the fragment first; `src/reader-app.tsx` renders read-only with the sandboxed renderer. The
owner router, writer, Ask and export modules are not part of this bundle.

## Restart recovery

Remote keeps sessions and door cookies in memory, so after a restart a paired browser's
reload receives Colab's private guidance page. That page loads the public `assets/recovery.js`
(`src/guidance.ts`), which takes over the tab claim (`ActiveTab`, Web Lock) before Remote's
SDK checks its paired key and calls `reopenSession()`, then reloads. A session-storage marker
(`colab-recovery:<mount path>`, `src/session-recovery.ts`) spans that reload so a second
guidance response cannot loop; authenticated boot (`mounted.ts`) clears it. A failed or
refused reopen, or unavailable session storage, leaves plain pairing guidance and never
retries an Ask. An open page whose sync drops shows "Sync disconnected" with the explicit
Reconnect button, which uses the same `recoverSession` helper through `Live.reconnect` (it
closes the page socket, Ask and observer first). Acceptance drives this through `reconnect(page)`
in `acceptance/ask.spec.ts`.

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
  secrets, wraps, `owner_operations`), 3 `device_registrations`, 4 `baselines`. Epoch
  secrets are local key material, not an encrypted-at-rest guarantee.
- `Store::open` creates and migrates. `Store::read` and `Store::write_existing` open only
  existing 0600 state owned by the user, create nothing and never migrate;
  `write_existing` requires the serve lifecycle lock and exactly the current schema.
  **Adding a migration means updating the hard-coded current version in
  `Store::existing`** as well as `MIGRATIONS`.
- Receipts are create-only: an exact retry returns the stored receipt, a conflicting
  envelope freezes its stream. Checkpoint publication prunes a shared prefix only when
  every namespace with updates there has a checkpoint at the same sequence/hash; pinned
  checkpoints and receipts survive. Capacity returns `Fault::Capacity`; nothing is evicted.
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
  `ls`, `show`, `share mode` and `share link`. It uses the private socket IPC when `serve`
  holds the lifecycle lock and the offline path otherwise; an uncertain IPC reply never
  falls back to a second writer (`cli_management.rs`, `page/ipc.rs`).
- `readers::Sessions` keeps at most 64 ephemeral challenges, tickets and active readers
  (`CAP`), with a one-minute challenge and ten-minute session. Readers are page- and
  epoch-scoped and never owner devices or writers (`readers.rs`).
