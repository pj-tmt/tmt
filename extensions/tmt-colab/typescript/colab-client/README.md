# Colab browser primitives

Private `@tmt/colab-client` is a WebCrypto-only library. The
[colab-v1 contract](../../contracts/colab-v1.md) owns normative bytes and full L1
gates. This slice implements values/LP/strict JSON, strict signature admission,
immutable object envelopes, sign-in/management input and typed owner statements,
device certificates/chains, namespace cuts and native HPKE epoch-key opening.
Pairing/send/baseline builders remain later L1 work.
No app, persistence, transport or authority lookup is included. A valid signature
is insufficient without current log, device, role, epoch and stream admission.

`statement.Envelope.verifyNext` binds the URL space/root, exact payload digest and
next retained head; revision 1 pins the editor management member. Persist that
head before dependent state. `certificate.Chain.verify` requires a caller-resolved
live issuer statement/key and exact certificate context, including validity.
`wrap.Envelope.open` requires the current log's owner, recipient and epoch context
and a persisted non-extractable recipient handle. Its fixed RFC 9180 schedule
uses native X25519/HMAC/AES-GCM; no raw private or intermediate-secret API exists.
The model's current-state policy remains outside these byte/crypto ports.
`page.history` is owner-only statement syntax; callers default an absent statement
to `shared` and enforce the 64-most-recent-epochs forward-wrap cap and atomic
join lists of at most 512 sorted unique entries. Earlier-epoch wraps use the
existing epoch-key grammar at the current membership revision.

Subject-key admission deliberately differs: browser syntax checks canonical
encoding and torsion; native admission additionally decompresses the point.
The owner admits keys natively before signing, and enrolled devices prove
sign-in possession. A backend cannot inject an unusable subject without the
owner/issuer signature. Syntax grants nothing: every authority use still requires
a successful strict native signature. No custom curve or extra possession step
is introduced.

Object seal generates its ID internally. WebKit's native Ed25519 signer produces
valid randomized signatures. No Colab code may rely on re-signing to reproduce
envelope bytes or hashes: every retry must resend the stored frozen bytes. The
envelope hash covers the signature, so a re-seal is a new object. Mutable byte
inputs are copied before asynchronous crypto.
Signing and recipient private keys are non-extractable native handles; recipient
restoration verifies its public-key binding using native X25519. No seed import
or intermediate-secret API is provided. Capability probes require a secure
context and fail on unavailable Ed25519/X25519 without a fallback.

Object seal/open accepts either a 32-byte epoch root or a non-extractable
HKDF/deriveBits handle imported from a validated 32-byte root. Handle input length
is hidden by WebCrypto, so its importing caller owns that check. No handle is
exported; derivation labels and cipher inputs remain the contract's frozen values.

Unit tests use the workspace-pinned Vite+ runner and explicitly select
`vitest.config.ts`; lint, formatting and the three-engine harness retain their
separate tools.

See [Colab browser verification](../../../../.agents/skills/tmt-colab/references/development.md#app-and-browser-client)
for library checks, unit tests, engine installation, harness commands and local
binary/report options. The default `test:browser` requires all three engines;
append `--engines chromium` for a Chromium-only diagnostic (or a comma-separated
known set). Empty/unknown/duplicate sets reject, and every requested engine must
pass. Reports use `{engines, results}` and explicitly name the selected engines;
a scoped run is not full L1 evidence. Developers run the default full set before handoff.

The test-only harness checks WebCrypto snapshots, opaque keys, independent known
answers and fresh ciphertext interoperability both ways with the Rust model
through developer-only examples, plus exact authority answers and fresh native
wraps/statements in all engines. Root-handle checks require exact frozen
headers/ciphertexts, handle-open and strict signature verification in every engine,
then Rust verification of all fresh seals. Full frozen-envelope equality is checked
in Node, Chromium and Firefox. Its imported keys are public fixtures, never
product inputs. The independent frozen-vector generators remain documented in
[the vector provenance](../../contracts/vectors/README.md).

CI runs the library check and unit tests on every pull request. The separate
advisory three-engine workflow follows the scope and cache policy in
[the CI architecture](../../../../ARCHITECTURE.md#ci-selection-and-worker-model).
