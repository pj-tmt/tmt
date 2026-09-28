# Pairing approval and claim v1

Locally implemented issuer and browser approval protocol. The
[native pairing contract](native-pairing.md) defines its source-build consumer.
Production activation and local credential recovery remain separate gates; no
public Office distribution is promised by this contract.

## Proof and consent

The originating native installation generates 32 cryptographically random bytes.
The claim secret is their canonical unpadded base64url encoding (43 characters).
`pairingId` is the lowercase hexadecimal SHA-256 digest of those original bytes.
Only the public pairing ID and bounded requested binding appear in the browser
link fragment. Never put secrets or tokens in a URL, ordinary output, log or
approval request body. The claim secret and refresh token never enter browser
approval requests. The current owner's ID token is sent only in the
`Authorization: Bearer` header to the trusted configured HTTPS service (loopback
HTTP is allowed only for the demo emulator).

There is no unauthenticated durable "begin" write. The admitted human owner must
approve or explicitly cancel the exact public request before the service creates
a pairing record; cancellation creates only a disabled tombstone. Display labels are untrusted
presentation, not evidence of local identity or authority. The native client must
also compare the claimed binding with its own expected values before storing
credentials. A comparison code or copied link alone does not prove possession.

## HTTP operations

### Browser approval link

The owner approval route is `/worlds/{worldId}/pair` with fragment
`#tmt-pair={payload}`. Payload is canonical unpadded base64url of a UTF-8 JSON
approval input (including `version`); decoded input is at most 2048 bytes.
The route world must match the requested world. Unknown fields, secret/token
material, invalid Unicode, noncanonical encoding and malformed input reject.
No endpoint or actor identity comes from this fragment. Display labels are text,
never markup. A URL visit, login or admission change performs no approval write.

The browser uses its operator-configured pairing service and the current human
session; only explicit approval/revocation actions send requests. Uncertain
approval retries preserve the same immutable request and challenge. Session,
world and route changes dispose the view and fence late action completions,
without pretending that a submitted remote write was cancelled.

Browser decoding is input feedback and response integrity, not authentication.
The service remains authoritative. Cross-consumer conformance fixtures pin this
wire contract instead of adding a shared Firebase/runtime package to the SPA.
`pairing-examples.json` contains independent literal acceptance/rejection vectors
used by browser, service and native decoders. Unicode labels and escaped invalid
surrogates are technical fixture data for cross-language scalar/whitespace
boundaries; invalid raw JSON stays in `invalidJson`, not imported JSON values.
The browser accepts only an exact
approval response echo with a valid principal/block and representable expiry;
unexpected token fields never become view state.

### Service operations

The service accepts JSON POST operations under its explicitly configured base
URL. Bodies are bounded to 4 KiB, reject unknown fields and use `version: 1`.
Application responses are JSON with `Cache-Control: no-store`; errors contain a stable code,
not raw Firebase errors, secrets or paths. Requests never accept an actor UID as
authentication. Browser cross-origin access requires an exact operator-approved
origin; it never authorizes a caller. Non-browser requests may omit `Origin`;
the operation's bearer or original-proof authority still applies. Malformed HTTP/JSON
rejected by the Functions platform before the handler follows platform responses.

| Operation  | Input                                                                                                        | Authority and result                                                                                                            |
| ---------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `/approve` | `pairingId`, `worldId`, `installationId`, `identityId`, `installationLabel`, `identityLabel`, `capabilities` | Verified Google bearer ID token, current tester admission and world ownership; returns approved binding and five-minute expiry  |
| `/claim`   | `secret`                                                                                                     | Original proof; returns the same logical principal, resource grant and a Firebase custom token while the approval remains valid |
| `/revoke`  | `pairingId`, or `secret` (never both)                                                                        | Verified admitted owner, exact bound agent bearer, or original proof; disables only that approval/grant, retaining resources    |

UUIDs use the grant v1 lowercase hyphenated format; world IDs are 20
alphanumeric characters. Labels are 1..80 Unicode scalar values, nonblank,
without control characters. Capabilities are exactly the ordered layout lists
in [agent grant v1](agent-grant-v1.md). No arbitrary paths or additional resource
permissions are accepted. `/approve` alone also accepts the optional owner-selected
`replacesPrincipalUid` described under retained-block reassignment.

Claim responses retain the exact approved binding and approval `expiresAt`, and
add `customToken` plus `grantExpiresAt` from the validated resource grant. Approval
expiry, grant expiry and Firebase token expiry are separate clocks. A retry reads
the existing grant expiry; it never computes a new lease or extends one. Approval
responses do not contain `grantExpiresAt` because no grant has been issued yet.

Missing/invalid human authentication is `401 UNAUTHENTICATED`; invalid input is
`400 INVALID_ARGUMENT`; unauthorized approval/revocation is `403 PERMISSION_DENIED`.
Claims with wrong/unknown proof, expired/disabled approval or lost owner/grant
authority return `404 PAIRING_UNAVAILABLE`, without disclosing world details.
Approval retries return the same unavailable error for expired/disabled records
or an invalid issued grant, after human admission/ownership is checked. Changed input for an
existing authorized pairing is `409 PAIRING_CONFLICT`. Known-proof claims are
limited to one reservation per five seconds (`429 RETRY_LATER`). Failed external
credential minting is `503 UNAVAILABLE`, not a success or permission to allocate
another identity. Unsupported method is `405 METHOD_NOT_ALLOWED`; oversize input
is `413 INPUT_TOO_LARGE`.

## Durable ownership and recovery

`officePairings/{pairingId}` is service-only; Rules deny all client access. Its
versioned record owns approved immutable input, human owner, generated principal
UID and independent block UUID, creation/expiry timestamps, `enabled`, `claimed`
and `nextClaimAt`. A retained-block approval additionally records immutable
`replacesPrincipalUid`. Clients cannot select the generated principal; a block
may only be selected indirectly through the validated source grant below.

Approval retries return the existing record only for the same owner and exact
input before expiry. They never extend the deadline or re-enable a record.
Revocation is deliberately idempotent for an admitted owner, even after approval
expiry: the longer-lived resource grant must remain revocable. Unknown/malformed
records, another owner and lost admission all return permission denied.
Bound-agent and original-proof retirement cancellation use the reduction-only
authority defined below; they do not require continuing human admission.
The first claim transaction creates exactly one [agent grant](agent-grant-v1.md)
and marks the approval claimed. Later authorized retries reuse that grant and
its original expiry; missing or disabled grants are never reconstructed.
"One-time" means one logical approval/assignment, not an unrecoverable one-shot
HTTP response. A holder of the original secret can retry during the same short
claim window after a lost response. No automatic retry after that window.

All ownership, admission, expiry and grant checks participate in the Firestore
transaction. IDs are selected once outside transaction retries; time is sampled
within each attempt. Custom-token signing is outside the transaction. After
signing, recheck current approval/grant authority before responding. Revocation
can still race the final response, but live Rules deny use of a revoked grant;
do not claim to recall issued bytes or cancel prior writes.

An issuer error may leave a reserved grant with no delivered credential. Retry
uses the same principal and resource rather than manufacturing a second grant.
The service keeps expired/disabled records as tombstones in this slice: no TTL
or cleanup silently permits reuse of a pairing ID. Bounded retention and public
endpoint abuse controls must be reviewed before production activation.

## Retained-block reassignment

The owner may add `replacesPrincipalUid` (`office-agent:` followed by a lowercase
UUID) to `/approve`. It is not accepted in the native public fragment. Without
it, approval allocates a fresh block as before. The browser lists bounded grant
pages and requires explicit recognition and selection; after any attempt the
choice is fixed for that mounted request. It verifies that the response names
the selected block, while native claim continues to verify its own identity,
installation, world and capabilities.

The source must be a complete disabled grant in the same owned world. In the
approval transaction, reserve it with `replacedByPairingId` and create a new
pairing/principal that names the same block. No resource document is written,
copied or deleted; notebooks and profiles do not transfer. Same request and
source retry returns the same binding; a changed choice conflicts. Two approvals
cannot consume the same source concurrently.

A marked source can be reclaimed only when its referenced predecessor is
unclaimed and expired or disabled, with matching owner, world, source and block.
Disable that predecessor and replace the receipt in the same transaction.
Missing, malformed, mismatched or claimed predecessor evidence conflicts.
Once claimed, a later transfer must use the newly issued grant after revocation,
never an ancestor. The old principal stays disabled and old cached credentials
remain denied. Expired enabled grants are not eligible: expiry is not revocation.

The original grant records are the sole authority and transfer receipts. Browser
inventory labels do not promise that a reservation is available; the issuer
checks current state when approving. Local expired-pending and lost/corrupt
credential recovery are separate work, not implied by successful reassignment.

## Lease renewal

Owner approval permits the selected agent to renew the same resource lease until
revoked; the browser discloses this before consent. Each lease is at most 24 hours.
The consumer is defined in [native pairing](native-pairing.md#invocation-owned-renewal).
No background process or fresh daily human approval is required.

`POST /renew` accepts exactly `{version:1, pairingId, grantExpiresAt}`. The expiry
is the positive representable integer timestamp last observed by the caller,
not a requested extension. Authentication is a verified, non-revoked Firebase
agent ID token in the bearer header, never the original claim secret, an
unverified JWT payload or a human token. Its UID, agent flag, installation and
identity claims must match the stored pairing and grant.

The transaction rechecks current owner admission, enabled/claimed pairing,
original approval shape, matching resource scope and a complete enabled grant.
The five-minute approval window remains closed: renewal does not call claim,
mint another principal, reconstruct a missing grant or reactivate a disabled one.
An expired resource lease can renew; expiry is not revocation.

When the expected expiry matches the current lease and it has at most five
minutes remaining, replace only the lease timestamps with server `now` and
`now + 24h`. A still-long-lived lease is returned unchanged. A lower expected
expiry returns the existing newer lease unchanged, even if it too has expired;
a higher expected expiry conflicts. This comparison makes lost-response retries
and concurrent requests converge without another extension. Transactions
serialize renewal against revocation; a committed renewal never undoes a later
revocation. Rules remain authoritative for subsequent resource access.

Success is exactly `{version:1,pairingId,worldId,installationId,identityId,
principalUid,blockId,capabilities,grantExpiresAt}` with server-authoritative
expiry and no credentials. Existing authentication, invalid-input, permission,
unavailable-pairing, conflict and unavailable-service errors retain their
HTTP mappings. No browser receives agent tokens or direct grant-read authority.

## Retirement cancellation

Owner cancellation also accepts exactly `{version:1,publicApproval:<approval input>}`.
It requires an admitted human world owner, never an agent bearer. Existing records
must match that owner and exact request. For an unknown request, the same revoke
transaction creates a disabled, unclaimed pairing with ordinary generated IDs
and timestamps, but no grant, token or resource. Those IDs are inert placeholders,
not issued assignments. Repeated cancellation is idempotent; late approval/claim
cannot revive the record. Existing source-transfer receipts and resources remain
unchanged. Viewing the link or logging in never cancels it automatically.

`POST /revoke` accepts exactly `{version:1,pairingId}` with a verified,
non-revoked bearer token, or `{version:1,secret}` without an Authorization header.
Mixed forms reject with `400 INVALID_ARGUMENT`. The owner form retains its
existing live ownership/admission requirement. An agent token must have the
agent flag and exactly match the claimed pairing's UID, installation and identity.
It cannot revoke another assignment. Original proof derives the pairing ID using
the same canonical decoder as claim; it also permits cleanup of a reserved grant
whose credential response was lost.

For agent/proof cancellation, expired approvals or leases and lost owner
admission do not prevent reducing authority. Unknown/malformed pairings or an
agent scope mismatch return `404 PAIRING_UNAVAILABLE`. An unknown proof never
creates a tombstone: without an approved record the service has no authenticated
scope to cancel, and the caller must not mark remote cleanup complete.

The existing revocation transaction is the sole writer for all three authorities.
It sets only approval and existing grant `enabled` fields to false, does not
reconstruct a missing grant, and retains block contents and assignment IDs.
Repeated authorized revocation succeeds, including after expiry. Success is
exactly `{version:1,pairingId,revoked:true}`. Renewal/claim transactions and live
Rules cannot turn this response into new authority or revive disabled grants.

## Deployment and verification boundary

This slice is locally verified against Auth/Firestore emulators. Functions are
disabled for non-emulator use unless the operator explicitly enables the service;
enabling production, billing, IAM, ingress limits and deployment are not authorized
by an emulator test. Validate emulator configuration against the demo project and
loopback endpoints, never accept unsigned emulator credentials in a real project.

The [local-first M1 acceptance contract](../../../DEVELOPMENT.md#personal-office-milestone-acceptance)
owns verification/cost requirements. Issuer tests must exercise actual emulator
tokens, transactions, concurrent claims, injected signer failure and downstream
Rules. Browser scenarios additionally verify explicit owner consent, lost-response
retry and logout fencing through the real service. These are not evidence of
protected native credential storage, native retirement or the full CLI-to-browser
milestone flow.
