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

Subject-key admission deliberately differs: browser syntax checks canonical
encoding and torsion; native admission additionally decompresses the point.
The owner admits keys natively before signing, and enrolled devices prove
sign-in possession. A backend cannot inject an unusable subject without the
owner/issuer signature. Syntax grants nothing: every authority use still requires
a successful strict native signature. No custom curve or extra possession step
is introduced.

Object seal generates its ID internally. Retrying means retaining the same
immutable envelope. Mutable byte inputs are copied before asynchronous crypto.
Signing and recipient private keys are non-extractable native handles; recipient
restoration verifies its public-key binding using native X25519. No seed import
or intermediate-secret API is provided. Capability probes require a secure
context and fail on unavailable Ed25519/X25519 without a fallback.

From the repository root:

```sh
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client install --frozen-lockfile --ignore-scripts
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client check
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client test
corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client exec playwright install chromium firefox webkit
COLAB_REPORT=/tmp/colab-browser-results.json corepack pnpm@10.33.0 --dir typescript --filter @tmt/colab-client test:browser
```

The harness requires all three engines, all 148 strict signature vectors and
nine accepted controls. Missing/skipped/error engines, missing controls or wrong
row counts fail nonzero; unit tests deliberately exercise that process failure.
It records engine versions and raw-verifier bypass results. It also checks
WebCrypto snapshots, opaque keys, independent known answers and fresh ciphertext
interoperability both ways with the Rust model through developer-only examples,
plus exact authority answers and fresh native wraps/statements in all engines.
Browser keys in that harness are public fixture imports, never product inputs.
Python is not required at test time; the independent frozen-vector generators
remain documented in [the vector provenance](../../contracts/vectors/README.md).

`COLAB_RUST_TOOLCHAIN` selects an installed Rust toolchain (default `+1.97.0`).
Optional `COLAB_CHROMIUM_EXECUTABLE`, `COLAB_FIREFOX_EXECUTABLE` and
`COLAB_WEBKIT_EXECUTABLE` select explicit local binaries. Launch failures never
skip a required engine. Reports default to ignored `differential-results.json`;
set `COLAB_REPORT` to retain evidence elsewhere. Browser/server cleanup runs even
on a failed engine. CI runs the library check and unit tests on every pull request;
the non-required, path-scoped three-engine job remains a coordinated follow-up.
