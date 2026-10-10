# SDK operations and the embedded client

Follow [tmt-dev](../../tmt-dev/SKILL.md), the
[Remote architecture](../../../../ARCHITECTURE.md#remote-extension-pilot) and
[channel contract](../../../../contracts/remote-channel-v1.md). The
[SDK README](../../../../extensions/tmt-remote/typescript/remote-client/README.md)
owns caller-facing usage. Inspect the SDK and Rust operation shapes together.

From `typescript/`, run the package gates with pinned pnpm:

```sh
pnpm --filter @tmt/remote-client --fail-if-no-match check
pnpm --filter @tmt/remote-client --fail-if-no-match test
pnpm --filter @tmt/remote-client --fail-if-no-match build
```

`test` checks the independent Python oracle before unit tests; repeat on Node 24
for WebCrypto conformance. Cover signed states/refusals, response correlation,
sequence serialization and both consumed/unconsumed lost-request recovery branches
on the existing session through `operation.show`, with the original ID and no
recovery dispatch or reopen. Verify the two-guess limit and typed refusal/unknown
outcome codes.

## Embedded client and crypto fixtures

The door embeds `extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js`, built
from `remote-client`. After changing `remote-client/src`, rebuild and commit it
(Code quality rebuilds it and fails on a difference), then run the Chromium pairing
smoke against a debug door:

```bash
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match build)
(cd rust && CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote)
(cd typescript && pnpm --filter @tmt/remote-client exec playwright install chromium)
(cd typescript && pnpm --filter @tmt/remote-client --fail-if-no-match test:browser)
```

Repeat the build and compare exact artifact bytes; after staging, check the asset's
`git diff --exit-code` and `git status --porcelain` for regeneration drift. The
smoke uses real Remote/Chromium and a deterministic public-core fixture; assert
direct acceptance, read-only observation, the retained final and one core dispatch.
Preserve pairing/certification/revocation and joined fixture cleanup; this does not
prove real-core agent delivery. Use [tmt-layout](../../tmt-layout/SKILL.md) for
added modules/fixtures.

Byte and crypto conformance runs with the Rust tests. The shared vectors come from
`extensions/tmt-remote/typescript/remote-client/test/reference.py` (which also checks the
pinned BIP-39 list digest). Check the Rust-owned fixtures with:

```bash
python3 extensions/tmt-remote/rust/tmt-remote/tests/fixtures/mac-reference.py --check
node extensions/tmt-remote/rust/tmt-remote/tests/fixtures/webcrypto.mjs
```

Use the repository Node 22 and repeat the WebCrypto command on Node 24; `--write`
regenerates the public-test-key fixture. The Python oracle imports no product code.
None of this proves real Chrome key persistence across MV3 worker restarts.

## Session continuity

The SDK README owns `reopenSession`'s opt-in bounded policy and typed outcomes. Test the
same-origin/key fence, deadline/cancellation and single-flight series independently of
Colab's connection state owner. Keep an opaque admission refusal non-terminal; until the
verified authority-proof seam ships it cannot distinguish revoked/expired from transient.
After admission, original-outcome observation must prove one core send total even when
its original acknowledgment was lost. Never make helper recovery silently resend.
