---
name: tmt-remote
description: Develop and verify the tmt-remote extension and device SDK.
---

# Remote development

## Remote SDK operations

Follow [tmt-dev](../tmt-dev/SKILL.md), the
[Remote architecture](../../../ARCHITECTURE.md#remote-extension-pilot) and
[channel contract](../../../contracts/remote-channel-v1.md). The
[SDK README](../../../extensions/tmt-remote/typescript/remote-client/README.md)
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

Commit the regenerated `extensions/tmt-remote/rust/tmt-remote/assets/remote-v1.js`.
Repeat the build and compare exact artifact bytes; after staging, check the asset's
`git diff --exit-code` and `git status --porcelain` for regeneration drift.
Rebuild the worktree's debug door from `rust/` with
`CARGO_BUILD_JOBS=2 cargo build --offline --locked -p tmt-remote`, then run
`pnpm --filter @tmt/remote-client --fail-if-no-match test:browser` from `typescript/`.
Install Chromium through the package's Playwright command if absent.
The smoke uses real Remote/Chromium and a deterministic public-core fixture;
assert direct acceptance, read-only observation, the retained final and one core
dispatch. Preserve pairing/certification/revocation and joined fixture cleanup;
this does not prove real-core agent delivery. Use [tmt-layout](../tmt-layout/SKILL.md)
for added modules/fixtures. Keep run evidence in the issue/PR.
