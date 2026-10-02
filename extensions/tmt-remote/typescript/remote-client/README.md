# Canonical byte groundwork

This private module implements decoded-value envelope, device enrollment and
possession signing-byte builders, the `K_response` and `serverProof` HMAC inputs,
pairing-code decoding, strict unpadded base64url and four-word fingerprint
indexes defined by
[remote-channel-v1](../../../../contracts/remote-channel-v1.md).
It is not a usable SDK. Inputs have already been decoded; wire JSON, duplicate
members, base64url/hex admission and payload operation schemas belong to a future
wire decoder. Structural checks here enforce framing, exact UTF-8, decimal bounds,
fixed profile values and kind/origin pairs. They do not establish key validity,
live grants, timestamp freshness, replay protection or remote authority.

Production uses TextEncoder and native WebCrypto SHA-256; no Node imports or
third-party crypto. Payload bytes are copied before asynchronous hashing and are
never parsed, normalized or reserialized. No signing, MAC computation, key
persistence, pairing ceremony, transport or browser-shell integration is included.
Fingerprint indexes point into the pinned BIP-39 English list at
`../../rust/tmt-remote/assets/bip39-english.txt`; callers map indexes to words.

From the repository's `typescript` directory with Node 22.12.0 or later, pinned pnpm and Python 3:

```sh
pnpm --filter @tmt/remote-client install --frozen-lockfile --ignore-scripts
pnpm --filter @tmt/remote-client --fail-if-no-match check
pnpm --filter @tmt/remote-client --fail-if-no-match test
```

The test command checks the independent Python 3 oracle before running the
workspace-pinned Vitest suite. Oracle failures stop the command before Vitest;
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
with `python3 test/reference.py` from this directory; `--check` verifies fixed
bytes without rewriting them. The TypeScript tests compare these literal artifacts
and mutate one condition at a time to demonstrate refusal and exact byte binding.
