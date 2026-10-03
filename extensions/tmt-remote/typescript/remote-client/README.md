# Remote device SDK

This private module implements the device side of
[remote-channel-v1](../../../../contracts/remote-channel-v1.md):

- `src/canonical-bytes.ts`: decoded-value envelope, device enrollment, possession
  and `tmt-ext-cert-v1` signing-byte builders, the `K_response` and `serverProof`
  HMAC inputs, pairing-code decoding, strict unpadded base64url and four-word
  fingerprint indexes. Inputs are already decoded; structural checks enforce
  framing, exact UTF-8, decimal bounds, fixed profile values and kind/origin pairs.
- `src/device.ts`: a non-extractable WebCrypto Ed25519 device key, the pairing link
  parser and pairing client (retrying a pending candidate and accepting the machine
  key only after `serverProof` verifies), the `session.open` client that verifies
  the machine-signed response, and extension key certification.

- `src/browser.ts`: the browser entry the door serves as `/sdk/remote-v1.js`. It
  runs the pairing page and gives mounted extension pages `reopenSession` and
  `certifyKey`, whose extension comes from the door's `/sdk/mount` answer.

`pnpm build` bundles the browser entry with Vite+ on the aliased Vite core (library mode, unminified) into
`../../rust/tmt-remote/assets/remote-v1.js`, which the door embeds; commit the
result. `pnpm test:browser` uses `vp exec playwright test` to run the Playwright Chromium pairing smoke against
`rust/target/debug/tmt-remote` (or `TMT_REMOTE_BINARY`).

Network access goes through an injected fetch. The caller persists the device
key's opaque `CryptoKey` (the browser page uses IndexedDB structured clone); the
private key is never exported. Byte construction and signatures establish no
authority: live grants, timestamps and replay are checked by remote.

Production uses TextEncoder and native WebCrypto; no Node imports or third-party
crypto. Payload bytes are copied before asynchronous hashing and are never parsed,
normalized or reserialized. Fingerprint indexes point into the pinned BIP-39
English list at `../../rust/tmt-remote/assets/bip39-english.txt`; callers map
indexes to words.

From the repository's `typescript` directory with Node 22.12.0 or later, pinned pnpm and Python 3:

```sh
pnpm --filter @tmt/remote-client install --frozen-lockfile --ignore-scripts
pnpm --filter @tmt/remote-client --fail-if-no-match check
pnpm --filter @tmt/remote-client --fail-if-no-match test
```

The test command checks the independent Python 3 oracle before running the
workspace-pinned Vite+ test runner with explicit `vitest.config.ts`. Oracle failures stop the command before the test runner;
assertion failures and missing tests also fail the command. Type checking remains
in the separate `check` command.
The existing unconditional Code quality CI job runs these commands using the
repository default Node 22.23.2, including for changes confined to this directory.
Run the same test command with Node 24 for the contract conformance target. No
release or distribution entry is added.

`test/reference.py` independently transcribes contract field order using Python's
standard-library `struct`, UTF-8 encoder, `hashlib` and `base64`, and refuses
to run if the pinned wordlist digest differs. Committed `vectors.json`
contains literal full bytes and SHA-256 values, first established with Python
3.14.7, rather than generated from the TypeScript implementation. The Unicode
fixture data deliberately includes astral and decomposed characters. Raw example
public keys and MACs prove framing only, not valid cryptographic proofs. Regenerate
with `python3 test/reference.py` from this directory, then format `vectors.json`
with the package's Prettier (`check` requires it); `--check` compares parsed values, verifying fixed
bytes without rewriting them. The TypeScript tests compare these literal artifacts
and mutate one condition at a time to demonstrate refusal and exact byte binding.

Certificate signature conformance also consumes the Rust-owned fixed WebCrypto
vectors over the Python oracle's exact certificate bytes. Tests reproduce the
signatures for `sign` and `enc` and reject changed domains, LP endianness,
extension names, purposes, keys, times and signatures. The Chromium smoke tests
silent certification for both purposes and retained signatures after revocation:
the grant loses live owner context and cannot reopen, even though its old
certificate signatures still verify. Extensions enforce certificate freshness
and learn revocation from Remote's device events; a certificate alone grants no
authority.
