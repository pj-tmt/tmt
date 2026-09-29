# Office service boundary

Local Firebase emulators and private-world Rules. The SPA offers explicit
emulator/cloud Google sign-in and direct client
creation/read of owner-only worlds. Rules require operator-managed tester
admission. Cloud deployment, invitations, guest memberships and presence are
not implied by this implementation. Unspecified paths remain denied.

Rules also enforce [scoped agent grants](../../../contracts/agent-grant-v1.md)
for UUID blocks with live expiry, capability and owner-admission checks. The
trusted issuer is implemented under `functions/` for emulator verification;
the browser can explicitly approve/revoke, and the native companion pairs and
reads/edits its assigned block through authenticated client requests.
Do not manually enable anonymous authentication or create production agent grants
to simulate pairing.
The owner can edit the home block and inspect/edit assigned or retained agent
blocks through the existing space inventory.

[Pairing v1](../../../contracts/pairing-v1.md) defines approval, proof claim,
retry and revocation. Non-emulator activation defaults off. No Functions are
deployed by building or testing; production IAM, endpoint abuse controls,
retention and costs require separate review and authorization.

See [the architecture](../../docs/office/architecture.md). Add functions only
when a trusted operation cannot be safely implemented with reviewed rules and
client contracts. Never place local TMT database or process access here.

## Start locally

Docker is the only required emulator dependency. Java 21, Node 22 and Firebase
CLI 15.29.0 run in the image, without host login credentials or application data.
From the repository root:

```sh
docker compose -f services/office/compose.yaml up --build
```

Auth is available at `127.0.0.1:9099`, Firestore at `127.0.0.1:8080`. The container
binds its internal interfaces; Compose publishes only host loopback. Do not run
this Firebase config directly on the host or expose its ports publicly. Emulator
UI is disabled. Stop with Ctrl-C, then remove the stopped development container:

```sh
docker compose -f services/office/compose.yaml down
```

State is ephemeral: there are no mounted data volumes or automatic imports.

For the optional local sign-in UI and the opt-in `browser-tests` Docker target,
follow [Local browser sign-in](../../DEVELOPMENT.md#local-browser-sign-in).
The default image/Compose service does not install Chromium or start the app.
It also does not start Functions. The integrated `browser-tests` target builds
the service and starts its loopback-only Functions emulator on port 5001.
Its browser scenarios exercise actual owner approval, scoped credential use and
revocation, including actual native pairing and revision-safe decoration commands.
Do not mount credentials, host config or repository roots into this service.
The first image build downloads tools and emulator binaries; later runs use the
cached image. Rebuild deliberately to update pinned tools, not on every test.

## Verify without cloud access

```sh
docker build -f services/office/Dockerfile -t "tmt-office-emulators:$worktree" .
docker run --rm --init --network none "tmt-office-emulators:$worktree" \
  firebase emulators:exec --only auth,firestore --project demo-tmt-office \
  --config firebase.json --non-interactive 'node verify-office-emulators.mjs'
docker image rm "tmt-office-emulators:$worktree"
```

`$worktree` is the per-worktree image tag defined in
[DEVELOPMENT.md](../../../../../DEVELOPMENT.md#keep-local-development-from-filling-the-disk),
so a rerun replaces the image instead of adding one.

The Firebase runner waits for readiness and shuts emulators down after the
script, propagating failure. The proof uses actual Auth creation/lookup/deletion,
Firestore privileged fixture write/read/deletion and denied authenticated and
unauthenticated client requests. It is not Office membership/invitation coverage
or a browser E2E test. The privileged `owner` fixture token is emulator-only.
The verifier refuses non-loopback endpoints and any non-demo project.

## Owner-local project settings

### Limited cloud pilot

The operator must explicitly approve production setup before changing services,
Rules or tester grants. In Firebase Console, register a Web app, enable Google
in Authentication, and authorize the exact testing/hosting domain. If Firestore
has not been created, choose its location deliberately and start locked; never
use test-mode open rules. Billing changes are not part of this workflow.

Copy `apps/office/.env.example` to `apps/office/.env.cloud.local`, then copy the
four public web app values from Console. Run `pnpm office:dev --mode cloud`.
For a build use `pnpm --filter @tmt/office build --mode cloud`; configure the
SPA host to rewrite `/worlds/*` to `index.html`. Default builds stay disconnected.
This pilot currently uses the project's standard `PROJECT.firebaseapp.com`
Auth domain; custom auth domains need a separately reviewed configuration change.

Browser pairing additionally requires an explicitly reviewed/deployed trusted
issuer and its HTTPS base URL in `VITE_OFFICE_PAIRING_URL`. Leaving it blank
disables pairing without disabling ordinary world access. Do not activate or
deploy the issuer merely to fill this setting; its production review gates remain
in the pairing contract. The browser accepts only the versioned public link
defined there, never a secret, token or endpoint supplied by a link. The owner
must recognize the request and approve explicitly. A lost response keeps the same
request for retry; **Revoke request** disables its access and retains content.
Approval is not proof that the native agent has connected.

Deploy the reviewed `firestore.rules` only after explicit authorization, using
the exact intended project rather than a default alias. A user can then sign
in, see their UID and wait for access. Under Firestore Data in Console, create
collection `testers`, document ID equal to that UID, and one Boolean field
`enabled` set to `true`. Use Boolean, not a string. Set it to `false` to revoke.
The UI observes changes and also provides **Check access again** for retry.
No client can write this collection or list testers. Approval grants only
access to the pilot; it does not grant access to another user's private world.

Inside a world, the owner can arrange its single home block with curated
furniture. Select an item, click a destination tile or use the coordinate inputs,
rotate/remove it, then choose **Save layout**. Unsaved previews stay local.
Concurrent edits conflict rather than silently overwrite; **Load latest layout**
explicitly discards the draft in favor of the last server-confirmed view. These
controls do not publish agents or grant any local execution authority.

Rules-dependent document lookups can incur reads, including denied requests.
The app has one tester listener after login, at most one selected-world
listener, and one home-block listener while the admitted world editor is mounted.
A world create transaction reads one world and writes it once; a block save
reads its revision, writes once and reads the canonical result. Retries and
authorization checks may add reads. No global directory, per-frame writes or polling is used. Logout
and access loss detach private listeners and clear the view. Memory-only caches
do not recall information already disclosed. No custom claims or Admin server
are needed to manage the pilot allowlist.

The shared configuration never selects a real project. If needed, copy
`.firebaserc.example` to `.firebaserc` in this directory and replace the owner
alias locally. There is deliberately no `default` real-project alias. Cloud CLI
operations must select an explicit project and require separate authorization.
The development container neither copies nor mounts this file.

`.firebaserc`, `.env`/`.env.*` (except `.env.example`), `.firebase/`,
`.firebase-local/`, `.secrets/` and conventional service-account/private-key files
are excluded from Git and Docker contexts. Store any deliberate local emulator
export under `.firebase-local/`; never export production data for these tests.
These patterns are defense in depth, not a secret detector. Review staged content
before committing; never force-add credentials. Safe templates must contain only
placeholders. Never put a private key, refresh token or Admin credential in a
`VITE_*` variable: frontend values become part of the downloadable bundle.

Firebase CLI login stays in its own host credential store. Do not copy or inspect
those credentials to configure this repo. A project ID is an identifier, not a
credential; successful project listing does not authorize deployments, service
enablement, billing changes or database-location selection.

References: [Firebase CLI](https://firebase.google.com/docs/cli),
[emulator setup](https://firebase.google.com/docs/emulator-suite/install_and_configure),
[demo project isolation](https://firebase.google.com/docs/emulator-suite/connect_firestore).
