# Colab browser primitives

Private `@tmt/colab-client` is a WebCrypto-only library. The
[colab-v1 contract](../../contracts/colab-v1.md) owns normative bytes and full L1
gates. This slice implements values/LP/strict JSON, strict signature admission,
immutable object envelopes, sign-in and management input. Typed owner statements,
certificates/wraps and pairing/send/baseline builders remain later L1 work.
No app, persistence, transport or authority lookup is included. A valid signature
is insufficient without current log, device, role, epoch and stream admission.

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
interoperability both ways with the Rust model through a developer-only example.
Browser keys in that harness are public fixture imports, never product inputs.
Python is not required at test time; the independent frozen-vector generators
remain documented in [the vector provenance](../../contracts/vectors/README.md).

`COLAB_RUST_TOOLCHAIN` selects an installed Rust toolchain (default `+1.97.0`).
Optional `COLAB_CHROMIUM_EXECUTABLE`, `COLAB_FIREFOX_EXECUTABLE` and
`COLAB_WEBKIT_EXECUTABLE` select explicit local binaries. Launch failures never
skip a required engine. Reports default to ignored `differential-results.json`;
set `COLAB_REPORT` to retain evidence elsewhere. Browser/server cleanup runs even
on a failed engine. CI integration requires the core lead's decision.
