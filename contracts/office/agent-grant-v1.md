# Agent resource grant v1

Implemented Rules boundary with a locally verified pairing/renewal issuer,
not a public distribution or assignment UI. Automated evidence uses isolated Auth/Firestore
emulators. No production activation or human credential sharing is implied.

## Principal and document

Each selected installation/local identity/world binding has a distinct Firebase
custom-auth principal. Its trusted token contains `tmtOfficeAgent: true`,
`tmtInstallationId` and `tmtIdentityId`. The latter two must match the live grant.
The issuer must never reuse a retired principal for a different binding. A
device-wide credential must not implicitly authorize every agent on that device.
Display names, folders and pane IDs are neither credentials nor lookup keys.

An active `worlds/{worldId}/agentGrants/{principalUid}` contains exactly:

| Field            | Value                                                                            |
| ---------------- | -------------------------------------------------------------------------------- |
| `version`        | Integer `1`                                                                      |
| `ownerUid`       | String matching the immutable world's human owner                                |
| `installationId` | Lowercase hyphenated UUID of the originating installation                        |
| `identityId`     | Existing local identity UUID, lowercase and hyphenated                           |
| `blockId`        | Independent lowercase hyphenated UUID, never `home`                              |
| `capabilities`   | Exactly `['layout.read']` or `['layout.read', 'layout.write']` in that order     |
| `enabled`        | Boolean; only `true` grants access                                               |
| `createdAt`      | Current lease start as a Firestore timestamp, not later than server request time |
| `expiresAt`      | Firestore timestamp, after creation and at most 24 hours later                   |

A disabled grant may additionally contain `replacedByPairingId`, a lowercase
64-hex pairing ID written only by the issuer's
[reassignment transaction](pairing-v1.md#retained-block-reassignment).
This receipt marks a reserved or transferred source, not an unassigned block.
It never grants access, and is invalid on an enabled grant. Client writes cannot
add or change the receipt; revocation preserves it.

Access requires `createdAt <= request.time < expiresAt`, valid schema, current
owner tester admission and matching authenticated principal/installation/identity.
An ID token or locally cached approval alone never grants access. Unknown fields,
versions, capabilities, malformed identifiers and timestamps fail closed.

Grants are issued only by the [trusted pairing service](pairing-v1.md), currently
verified locally with emulators rather than deployed. All client creates and
deletes are denied. Owner-only lists require an explicit limit from 1 to 20;
agents and foreign owners cannot enumerate. Only the admitted world owner can read a known
grant or change `enabled: true` to `false`, without changing any other field.
Owners cannot enlarge, renew or reactivate it through Firestore client writes.
They may also disable a malformed record as fail-closed recovery. Agents cannot
read grant metadata, revoke another identity or grant themselves authority.

## Resource behavior

The block uses the existing [block v1](block-v1.md) document and revision rules
at `worlds/{worldId}/blocks/{blockId}`. There is no second layout codec or stored
owner field. An active reader can fetch only that block (including its absence).
A writer can create revision 1 or advance by exactly one. Enumeration, deletion,
other blocks and the owner's `home` remain denied to agents. The admitted owner
can read/edit `home` and UUID blocks, including retained unassigned blocks.
Neither owner nor agent can delete a block; clearing uses an empty layout.

Layout authority confers no world-root, notebook, profile, board, message, work
dispatch or general Firestore access. Those capabilities need separately reviewed
contracts. Presentation cannot grant permission. The browser owner can select
a grant's UUID block through the same editor as `home`. A revoked grant is not
proof that a block is unassigned; retained content remains owner-managed.

## Owner inventory

The browser reads one server-confirmed page of 20 grant documents ordered by
document ID, with an exclusive last-ID cursor. Refresh restarts at the first
page; it does not accumulate an unbounded cache. A full page may have an empty
next page. Concurrent insertions before a cursor appear only after refresh;
pages are not a point-in-time snapshot of the collection. No grant polling or
per-entry subscriptions are installed. Invalid records make the page unavailable.

Labels reflect the last fetch and the browser clock, not online presence or
fresh server authorization. Expired enabled leases may renew; revoked records
remain visible, including explicit transfer receipts. The inventory itself does
not change grants, recover credentials or enumerate blocks without surviving
grant references. Explicit reassignment uses the owner approval flow.

Revocation denies subsequent server operations even with the same cached token.
It neither erases retained blocks nor recalls already disclosed content. Expiry
is checked by Rules, not eventual TTL deletion. Previously authorized writes can
have committed before revocation; clients must not assume cancellation.

## Pairing and credential lifecycle requirements

These requirements constrain subsequent #209 slices; they are not implemented
by these Rules:

- Follow the [explicit approval/proof flow](../../docs/office/design.md#invitations-and-pairing).
  The owner approves the actual deployment, installation, selected identity,
  resource and capabilities. Only a trusted service issues the principal/custom
  token; no anonymous-provider enablement or browser-minted credentials.
- Use a 24-hour default and maximum resource lease. Renewal must recheck current
  owner admission, the exact approved binding and its non-revoked state. Token
  refresh alone cannot extend a lease or reactivate a revoked grant. Native
  renewal is defined by the [native pairing contract](native-pairing.md#invocation-owned-renewal):
  scoped access renews near expiry without a daily human prompt.
  Re-pairing after revocation creates a new principal.
  The issuer must enforce these transitions itself: Admin writes bypass Rules.
- Store agent refresh credentials through a native protected-store adapter,
  separate from config, source, argv, output and logs. Verify platform protection
  and any fallback before use. Never transfer the human's refresh token or a
  service-account key. ID tokens are short-lived transport credentials; Firebase
  refresh does not replace live grant enforcement.
- Conclusive temporary-identity retirement must disable local use and revoke
  remotely, retaining resources for the owner. Offline retirement cannot revoke
  a server record immediately: retain pending revocation, retry explicitly on
  reconnect and rely on the finite lease as the upper bound. Do not claim prompt
  or filesystem permissions isolate mutually hostile agents under one OS user.
- Preserve identity/resource ownership across saved reconnect and block moves.
  A replacement same-name UUID inherits neither credentials nor retained notes.
  Pairing supplies authority, not memory orchestration or model-driven behavior.

## Verification ownership

`extensions/tmt-office/typescript/apps/office/e2e/agent-grant-rules.spec.ts` uses real custom-auth emulator tokens,
direct SDK requests and independent operator fixtures. It proves scoped writes,
full layout bounds, cross-principal/world/capability denial, claim mismatch,
malformed/expired grants, one-way revocation and retained content. Existing block
vectors remain in their original suite. These tests do not prove secure issuance,
protected native storage, native retirement, renewal or usable end-to-end pairing.

Sources: [custom tokens](https://firebase.google.com/docs/auth/admin/create-custom-tokens),
[session lifetime](https://firebase.google.com/docs/auth/admin/manage-sessions),
[Rules and privileged SDK boundaries](https://firebase.google.com/docs/firestore/security/rules-conditions).
