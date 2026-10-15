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
Colab's connection state owner. Only a verified fresh-open retained-grant proof ends admission
as revoked/expired; opaque or unverifiable admission stays non-terminal. Pin terminal no-retry,
nonce/request/run correlation, prior-bundle uncertainty and no publication after abort.
After admission, original-outcome observation must prove one core send total even when
its original acknowledgment was lost. Never make helper recovery silently resend.

## Hosted Firebase asset

The hosted-only adapter is built from exact `firebase@13.0.0` in the workspace lockfile.
`remote-client/dist/remote-firebase-v1.js` and `dist/THIRD-PARTY-NOTICES.txt` are generated,
not committed or embedded by Cargo. `firebase-build-pin.json` owns their relative paths,
SHA-256, byte lengths and Firebase version; the package tests compare built bytes with it.
Build twice and compare both hashes before changing the reviewed pin. Keep source maps,
timestamps and absolute paths out of output. Preserve the generated Firebase license text.
Colab copies them as `/sdk/remote-firebase-v1.js` and
`/sdk/THIRD-PARTY-NOTICES-remote.txt`, beside `/sdk/remote-v1.js`, and verifies the pin.
Colab and Infra own their build ordering and copy integration. Only the embedded SDK/page
assets retain the git-diff regeneration gate. The lazy adapter's bundle check rejects Node
imports, grpc-js and undici. Before loading Auth, `projectId`, `appId` and `authDomain` must
equal same-origin `/__/firebase/init.json`. A hosted page that calls `connect` must allow
identitytoolkit.googleapis.com and securetoken.googleapis.com in `connect-src`, plus
firestore.googleapis.com for sealed-document transport. Colab owns the hosted shell policy.
`firebaseRemote.originLink()` derives its public pair link from that same origin; `connect`
retains every verification check. The local path never imports Firebase implicitly.

## Sealed-document types

`sealed-sync.ts` and native `sealed_sync.rs` own the bound ports and pure validation.
`HostedAuth.sealedSync(space, guard)` implements the browser factory after anonymous sign-in. The [contract](../../../../contracts/remote-channel-v1.md#extension-scoped-sealed-document-client)
owns context, injected Auth, sequence catch-up, uncertainty and the cloud object
profile. The browser transport shares hosted App/Auth, uses REST for data and one memory-only
Web SDK listener per page interest for coalesced wake hints. The caller supplies admission
reservations before REST attempts/listener starts; validated neutral mutation identity lets the
caller refuse an unsupported commit shape before Auth/HTTP. Listener delivery and REST catch-up are
separate modeled read costs, including the writer's own listener. Optional `guard.received`
reports the full initial server tail and every later batch; its terminal budget refusal
closes the listener, and the caller joins its ledger jobs. Native transport and
credentials remain separate slices. Frozen fixtures are not installed provider evidence.

The explicitly selected native emulator acceptance uses the same Colab vectors; see
[the runner](../../../../extensions/tmt-remote/typescript/remote-client/test/emulator/README.md#native-acceptance-in-ci).
The [contract](../../../../contracts/remote-channel-v1.md#extension-scoped-sealed-document-client)
owns the proof boundary; normal crate tests do not start emulators.
