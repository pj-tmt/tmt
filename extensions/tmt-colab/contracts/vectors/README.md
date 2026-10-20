# Colab model vectors

The [colab-v1 contract](../colab-v1.md) owns byte definitions and full L1 gates.
All seeds and secrets here are public test fixtures, never runtime inputs.

`shell-v1.json` contains hand-authored closed shell values and detached actions,
shared by the native shell validator and browser fold validator. Expected results
cover item order, complete replacement, summary attribution and reading references;
they establish no target access or signature authority.

`attachment-reference.py` independently frames descriptors/manifests, freezes
strict selectors/creation-publication records and seals asset envelopes with
Python cryptography over public RFC8032 fixture keys. Namespace and page-revision
known answers include empty, own-only and paired-checkpoint/tail positions; these
positions prove byte parity, not membership or reference authority.
`attachment-v1.json` is shared by Rust model/isolated decoder and browser
client/Worker tests, with exact canonical bytes, independent raw/envelope hashes,
source/context/signature substitutions and descriptor/list bounds. The client
differential gate executes the same codec/crypto cases in all three engines.
Run the oracle without `--write` to verify frozen bytes; deliberate regeneration
uses a throwaway virtualenv. Reference/history authority and live object callbacks
are subsequent #1853 slices, not established by these byte fixtures.

`attachment-fence-reference.py` (stdlib only) independently frames the message-attachment
fence; `attachment-fence-v1.json` is consumed by Rust `message_fence` and browser
`messageFence`. Run it without `--write` to verify the frozen bytes.

`ed25519-829.jsonl` retains all 148 cases from #829's
`evidence/ed25519-differential-vectors.json`; each line is an unchanged case value,
including nine accepted positives and mixed-order controls.
`encoding-829.jsonl` retains the ten signed `cases` and five pairing `macCases`
from #829's `evidence/encoding-vectors.json`. Their historical header/cut/chain
schemas do not cover colab-v1 namespace changes and must not be used as current
wire admission. Rust verifies their exact signatures/MACs and primitive framing.
Source evidence is read-only under `spikes/829-colab-crypto/` in the #829 worktree;
[the accepted report](https://github.com/wkh237/tmt/issues/829#issuecomment-5933403999)
records provenance and bounded conclusions.

`model-reference.py` independently implements RFC5869 and LP framing with Python
stdlib; Python cryptography supplies AES-GCM and Ed25519. It imports no Rust or
browser implementation. `model-v1.json` freezes the new namespace header and cut,
sign-in proof/possession, management digest, ciphertext and envelope hash. Python
is needed only for deliberate regeneration/checking; ordinary Rust tests need no
Python dependency. A different Python/OpenSSL crypto implementation provides
independent known answers, not proof of universal curve/renderer behavior.
Browser interoperability and the three-engine
missing-engine failure gate remain later L1 work.

`keys.jsonl` retains #829 device/owner/link/RFC9180 recipient public keys and
[RFC8032 test 1](https://www.rfc-editor.org/rfc/rfc8032#section-7.1) /
[RFC7748 section 6.1](https://www.rfc-editor.org/rfc/rfc7748#section-6.1) key KATs.

`authority-reference.py` independently computes RFC9180 Base AES256GCM wraps,
owner signatures, chain digests and purpose-separated link keys. Its frozen
`authority-v1.json` uses #829 public recipient/ephemeral fixtures with the actual
owner signer bound into the header. Ephemeral fixture secrets are generator-only;
the runtime seal API always generates fresh ephemeral keys. The owner statement
bytes also cover the strict owner-statement codec and hash-chain admission;
applying verified transitions remains a runtime caller responsibility.

The same independent generator freezes `owner-member-v1.json`: valid owner-signed
removal and role-change statements targeting the pinned owner are rejected, with
accepted peer-target controls isolating the owner-binding rule from signature and
payload admission. Rust and browser verifiers consume these exact envelopes.

The authority oracle also freezes strict `page.history` payload/statement cases,
obsolete `page.scripts` rejection and a wrong-owner signature control. An absent
history statement defaults to `shared` in caller policy, not by accepting a
missing payload mode. `forwardWrap` covers epoch 63 at current epoch 64 and
membership revision 2 with the existing epoch-key wrap grammar. `historyJoin`
carries an owner-signed member addition and 576 existing wrap envelopes, serialized
as compact JSON strings in sorted unique 512/64-entry lists for nine pages at
the 64-most-recent-epochs cap. Atomic join delivery, global list admission, the
history default and epoch cap remain caller policy; the model only parses and
authenticates existing values. No aggregate join wire grammar is introduced.

`authority-v1.json` `sealedHistoryCases` contains independently owner-signed epoch
seals/revocations and encrypted receipts. Native fold and browser admission consume
the same sealed positive, unsealed refusal, beyond-cut, tail-hash and checkpoint
controls. The vectors use the actual derived management principal; they establish
read-bound parity, not a live routed backend or current caller entitlement.

`authority-v1.json` `linkDevice` and `linkHostedDevice` freeze a link's seed-derived device for
each transport (the door, and the hosted cloud log): its ID, signing public key, and the exact
link-issuer chain (fixed 0 to 2^53-1 validity window) with its digest. The two share no ID or key.
The TypeScript client reproduces these bytes and the Rust model verifies them.

`authority-v1.json` `linkRulesToken` freezes the link's cloud admission token
(`HKDF(linkSeed, LP("tmt-colab-link-rules-token-v1", space, linkId))`, unpadded base64url); the
Rust model and the TypeScript client both reproduce it.

`bridge-own-v1.json` freezes owner-signed membership logs (`bridge.add`, `device.revoke`) and
sealed own/content envelopes of bridge streams, each case marked `admit` or `reject`. Native
`tests/bridge_vectors.rs` replays every case in a real store, so a native `Invalid` and a browser
`Objects.admit` rejection are asserted for the same bytes; `test/bridge-own.test.ts` replays
them in the browser. It has no Python reference: the native fixture is the oracle, and the
frozen envelopes use random AEAD nonces, so regeneration (`cargo test --test bridge_vectors
-- --ignored regenerate`) is a deliberate contract change that rewrites every byte.

`fold-v1.json` freezes the owner fold (colab-v1, "Owner fold"): an owner-signed
log (`member.add`, `bridge.add` and, per case, a `device.revoke` of the signer or of the covered
device), the signer's and the covered device's owner-member chains, and catch-up frames (an anchor
frame, then a fold frame), each case marked `admit` or `reject`. The browser test
`test/fold-own.test.ts` replays every frame through `Admission` and `Objects`. Native
`src/fold_vectors.rs` (a unit test, since signing a certificate is crate-private) replays the part
the model decides: each fold's header and covered device, the anchor check, whether the signer's
`device.revoke` lists the fold, and that a reader of today's thirteen-element header refuses a fold
by its field count (`oldReader`). Native `tests/fold_vectors.rs` replays the cases through a real
store: the chains are stored, each fold is written with `Store::fold` and read back through the
owner snapshot, and the case's outcome must match; the two `advanceThenRevoke` cases are browser-only.
Like the bridge vector it has no Python reference and its envelopes carry random AEAD nonces, so
regeneration (`cargo test -p tmt-colab --lib fold_vectors::regenerate -- --ignored`) rewrites every
byte.

`link-author-v1.json` freezes owner-signed membership logs (`page.share`, `link.add`,
`link.remove`), link-issued device chains (valid, forged, citing another statement, below the
`link.add` revision, and the member/link issuer-kind swaps) and sealed own/content envelopes of
those devices, each case marked `admit` or `reject`. Native `tests/link_author_vectors.rs`
replays every case in a real store; `test/link-author.test.ts` replays the same bytes through the
browser `Admission` and `Objects`. Like the bridge vector it has no Python reference and its
envelopes carry random AEAD nonces, so regeneration (`cargo test --test link_author_vectors --
--ignored regenerate`) rewrites every byte.

`management-key-v1.json` is generated by the same authority oracle. It freezes
space-bound, purpose-separated local management member ID and Ed25519/X25519
public derivation, plus exact remote-owned `tmt-ext-cert-v1` bytes/signature for
colab registration. All seeds in this fixture are public test inputs; production
Keyring exposes only the derived public binding and admitted signing operations.

`own-v1.json` freezes Yjs 13.6.32 update-v1 bytes at public test client ID 1264. Its
`stateEncoding` is what a browser device publishes as its own checkpoint (the state encoding
of the document holding its updates); native and browser decode it to the same projection.
Both engines consume the same four-map projections, checkpoint/dependent tail,
deletion/tombstone-shaped data, JSON normalization and UTF-8 message-boundary cases.
The 500-thread update is reused under two authenticated writer identities to prove
the page-wide 1,000 limit; its dependent extra-thread update proves one-over denial.
Negative bodies isolate roots, list/shared-type values, body types/sizes, malformed
bytes and unresolved dependencies. Raw record examples are not an accepted typed
Send/reply grammar. No Python or new runtime dependency is needed at test time.

`checkpoint-v1.json` freezes raw update-v1 bytes produced by Yjs 13.6.32
with public test client ID 1280. The checkpoint merges only that writer's original
updates, retaining dependencies and delete sets; the retained tail requires that
prefix. Browser Worker and isolated Rust decoder tests consume the identical
positive and malformed-body bytes. Signed browser fixtures separately bind
interleaved namespace headers, prefix sequence/hash and revoked-device cuts;
these fixtures do not establish native store pruning or mounted sync delivery.

`export-reference.py` independently implements the conversation projection, the JSON and Markdown
renderings and the manifest from the written colab-v1 rules (stdlib plus `cryptography` Ed25519 over
public RFC 8032 seeds). It regenerates `export-v1.json`, which native and browser tests consume without
Python: `python3 export-reference.py` checks it, `--write` regenerates after review.

`publication-reference.py` independently frames and signs the content-publication contract
with Python stdlib and `cryptography` using public RFC 8032 test seeds.
`publication-content-v1.json` freezes browser/native content and native own-namespace one- and
two-entry jobs (the first four fixtures predate the own kind and are byte-identical), exact original
envelope bytes, nested signature inputs and digests, all outcome variants and proposed
local write/status DTOs. Rust consumes the frozen vectors without Python; running the
oracle checks them, and `--write` regenerates after review. These pure codec vectors do
not establish transaction, authority, route, Save/CLI or recovery admission.

`deploy-declaration-reference.py` (stdlib only) independently rebuilds the
`tmt colab deploy-declaration --json` line from `../../firestore/declaration.json` and
`admission.rules`: it pins `admission.digest` and compares `deploy-declaration-v1.json`, which the
native command must print byte for byte. `deploy-declaration-v1.{firestore.rules,firestore.indexes.json,plan.json}`
are the composed goldens Remote's `compose_firestore` example writes from it; Remote's mirror test
and the Rules emulator suite read them. Run the oracle without `--write` to verify, and with
`--write` after a reviewed source edit (then regenerate the goldens, see the colab skill's
Firestore Rules emulator section).

The members `holders` discovery declares an integer `seq` the Rules pin to `1` (a declaration-grammar
constant, not a sequence), and the composed Rules admit a link wrap to a member whose row names the link.

`deploy-declaration-v1.hosting.json` is the same reply for an executable that embeds a hosted
inventory: the declaration gains a last top-level `hosting` key holding the canonical manifest
verbatim, and the declaration digest covers it. `deploy-declaration-v1.hosting.manifest.json` is
that manifest (path-sorted `{path, sha256, length, contentType, csp}`, compact, five fixture
files including Remote's three SDK routes). The oracle builds both from a fixed fixture, so no
digest is typed by hand; the Rust unit test equals the vector and Remote's mirror reads it.

`page-markup-v1.json` holds the shared cases of the page markup check (colab-v1, "Page markup
check"). A case is a `source` (lines joined with a newline), the `admitted` questions,
groups and review sections the page shows (`reviews` is omitted when a case has none), and the
`findings` with reason codes. `scope:"parser"` cases run in the
browser client (`inspectQuestions`, headless Chromium, the oracle) and in the Rust checker;
`scope:"checker"` cases (attachment references, nesting depth) run in the checker only. Keep
cases small: a payload of tens of KiB belongs in a unit test that builds it, since every tool
that reads the repository reads this file. Findings are compared as a set by `(element, ordinal, code)`; `line` is asserted by Rust
alone, as the DOM has none. The `limits` block must equal each side's named constants. Edit the
cases by hand and run both runners: the file has no independent generator, because the browser
is the oracle.

`project-name-v1.json` holds the project name grammar (colab-v1, "Projects"): `valid` and
`invalid` names, written with `\u` escapes where the character is invisible. The owner store
(`projects::valid`) and the browser (`projectNameError`) each read it, so the two cannot drift: the
grammar is Unicode `White_Space` (not JavaScript `\s`, which also takes U+FEFF) and Unicode
control characters. Edit the cases by hand and run both tests.
