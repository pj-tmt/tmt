# Browser sealed-sync qualification

This explicit lane uses the product Auth/REST/push implementation with a private
emulator-only entry. It does not ship that entry, use real Firebase credentials,
or substitute Remote-authored fixtures for Colab release data.

The wrapper reads the exact accepted Colab Git head's declaration, composed Rules,
indexes and pinned emulator Dockerfile. It builds one named image and starts one
Auth+Firestore container, with host ports bound to loopback. The headless browser
uses a demo project and ephemeral anonymous users; emulator Admin seeds admission
only in the fixture. No production client gains Admin authority. The member
listener has equality on space/page/epoch, with neither stream nor sequence.

Before requesting the Infra slot, build the normal SDK/assets and the private entry:

```sh
pnpm --filter @tmt/remote-client build
pnpm --filter @tmt/remote-client exec vp build --config test/emulator/vite.config.ts
```

After Infra grants the exact committed head, run from `remote-client`:

```sh
REMOTE_SYNC_INFRA_GRANTED=1 REMOTE_SYNC_HEAD="$(git rev-parse HEAD)" \
  node test/emulator/qualify.mjs
```

The wrapper fails rather than skipping without its grant/exact-head/clean-tree
inputs or the 30 GiB disk floor. Readiness comes from the emulator process's
ready signal, not sleeps or network polling. It stops on the first red, removes
its container/tag, and records image inventories, artifact hashes and logs under
its private evidence directory. Report newly pulled base images to Infra; never
prune shared images or restart Docker. A changed accepted Colab head requires
updating `colab-source.json` to that head with status `accepted` and requalifying. Trace
recording is off so emulator tokens are not written into traces.

The Firebase dependency remains exactly 13.0.0. Pin equality/size/license checks
cover the actual hosted asset separately; this test bundle is never copied into
Remote pages or Colab Hosting. Emulator evidence is not live-project acceptance.

## Native acceptance in CI

`native-qualify.mjs` selects the one ignored native Auth/REST scenario and runs it
inside the existing Native Rust workspace tests job. Host Temurin 21 and
firebase-tools 15.29.0 start Auth+Firestore on loopback; no Docker or account is
used. Declaration, Rules and indexes come directly from the checkout's Colab
vectors, with their SHA-256 hashes printed. The scenario uses `private-world.mjs`,
requires exactly one executed test and reports wall time; missing inputs fail.
Ordinary `cargo test` ignores this explicitly emulator-only scenario. The
[contract acceptance boundary](../../../../../../contracts/remote-channel-v1.md#extension-scoped-sealed-document-client)
owns what this proof establishes; composite-index enforcement and live-project
acceptance are separate.
