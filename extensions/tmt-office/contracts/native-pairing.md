# Native pairing v1

Implementation contract for native pairing, renewal and retirement hooks. The
public Office alpha and source builds implement deployment discovery, pairing
and protected credential use; no hosted Office service or production Firebase
deployment is provided.
[Approval and claim](pairing-v1.md) owns remote proof/consent semantics.

## Deployment discovery

The caller selects a canonical world URL:
`https://office.example/worlds/abcdefghijklmnopqrst`. Discovery reads only that
origin's `/.well-known/tmt-office.json`, without redirects or credentials.
The descriptor is JSON, at most 4096 UTF-8 bytes, with exactly these fields:

```json
{
  "version": 1,
  "mode": "cloud",
  "projectId": "example-office",
  "apiKey": "public-web-api-key",
  "pairingUrl": "https://issuer.example/officePairing"
}
```

The SPA derives this allowlisted projection from its existing validated Firebase
and pairing configuration, not a second settings file. Preview and cloud without
an issuer publish no usable descriptor. Invalid configured deployments fail the
build. Publishing metadata does not activate the issuer or grant world authority.
No private environment values, credentials or identity data are included.

Project IDs use the existing Firebase format and exclude `demo-` in cloud mode.
API keys are 1..256 ASCII alphanumeric, hyphen or underscore characters. URLs are
canonical absolute HTTPS, at most 2048 bytes, with no user information, query or
fragment. The browser's configured URL may have one trailing slash; its descriptor
and operation base omit that slash. The literal corpus includes an origin-only
issuer to pin this projection. Native URL interpretation uses
the URL Standard, matching the browser; normalization cannot change a target.
Auth and Firestore endpoints come from the fixed Firebase API contract, never
arbitrary descriptor fields. Unknown and duplicate JSON fields reject.

Explicit emulator selection accepts only a canonical `http://127.0.0.1:<port>`
world origin, `demo-tmt-office` project/key, and the fixed issuer
`http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing`. Auth and Firestore
remain fixed loopback demo services. Cloud and emulator descriptors cannot be
interchanged. This is not a general insecure-HTTP switch.

The website origin and complete descriptor become part of the credential scope.
Local status and credential reuse do not rediscover endpoints. A changed
deployment cannot retarget an existing proof or credential. Discovery has a
10-second budget within the command deadline; reject redirects, non-JSON responses
and oversized headers/bodies before use.

## Command and credential contract

```sh
tmt office pair --world <url> [--identity <name>] [--read-only] [--timeout <seconds>]
tmt office status --world <url> [--identity <name>]
tmt office inspect --world <url> [--identity <name>]
tmt office unpair --world <url> [--identity <name>]
```

These retain existing Office prefix/output conventions. Names resolve through the
existing active identity UUID owner; omission requires a verified bound pane.
No folder guessing, same-name inheritance or temporary-identity promotion.
Explicit `--emulator` selects the fixed local environment on each operation.
Pair requests layout read/write unless `--read-only`. Timeout is integer seconds
1..300, default 300; retries retain the original proof/deadline and five-second
poll floor. Human mode prints the public approval URL before polling. JSON mode
puts that URL on stderr and one final object on stdout. No secret enters output
or argv. Unqualified status retains installation-only behavior; world-qualified
status is local-only and returns `unpaired`, `pending`, `credential`, `expired` or `revoked`,
with `serverAuthorizationChecked: false`. Inspect performs a server-authorized
read of the assigned block, not a world-wide index. Its result is
`blockExists: true|false` with `serverAuthorizationChecked: true`; it does not
return layout contents. It reuses the validated block reader; malformed stored
layouts are errors, not usable-space confirmation. Missing pairing fails `OFFICE_NOT_PAIRED`.

A stable installation UUID is independent of release receipts and binary prefixes:
one logical installation per ConfigPaths root. Only that public UUID and scope
locks live in its `office/` directory. SQLite stores opaque lifecycle hook
references, not another binding/credential registry. Missing or corrupt metadata is not automatically regenerated
inside an existing installation. One protected record per origin/world/mode/
installation/identity UUID pins the complete descriptor, immutable approval and
pending proof/deadline/cadence or paired credential/resource lease, validated
on every read. Store the pending proof before exposing an approval link. No token
belongs in config, SQLite or logs. Missing/locked storage fails closed; no sample
or plaintext fallback. Linux uses Secret Service on the session bus; macOS uses
the user Keychain. Each write requires exact readback before reporting success.
The per-scope lock serializes mutation; local status reads one protected snapshot
without creating locks. OS storage and metadata are not one atomic transaction.
Uncertain claim or publication retains the original scope for retry. Expired
pending approvals fail closed without silently requesting another grant;
paired resource access can renew its existing lease as described below. Do not delete local state as a
substitute for server revocation.

Auth exchange and refresh use fixed Firebase endpoints. ID-token payload checks
detect inconsistent scope but are not signature verification or authorization;
Firestore Rules enforce every resource request. Token refresh alone preserves the
original resource assignment and grant expiry. Identity names resolve once to an
active UUID and are rechecked before protected publication and resource use.

Office failures use the existing envelope and exit 1:
`OFFICE_DEPLOYMENT_INVALID`,
`OFFICE_CREDENTIALS_UNAVAILABLE`, `OFFICE_CREDENTIALS_INVALID`,
`OFFICE_NOT_PAIRED`, `OFFICE_PAIRING_PENDING`, `OFFICE_PAIRING_EXPIRED`,
`OFFICE_REMOTE_DENIED`, `OFFICE_REMOTE_UNCERTAIN`. Interruption uses
`OFFICE_INTERRUPTED`, exit 130. Existing identity/grammar errors retain their
owners. An unavailable claim cannot distinguish absent from denied approval.

Retained-block reassignment is supported during owner approval. Lost-credential
repair remains separate work, not an implicit M1 acceptance gate. Retirement
delivery is defined below; token refresh never extends a grant lease.
Actual native/browser/isolated-vault evidence is required.

## Explicit cancellation and reuse

`unpair` uses the existing protected scope lock and revocation writer. Pending
records use original proof; paired records refresh Auth if necessary and use
their bound agent token, without renewing the resource lease. Only a confirmed
response permits writing a secret-free `revoked` receipt. Retrying that receipt
is local and idempotent. Missing/corrupt records are not reset; network, Auth and
vault failures retain evidence and do not report cleanup complete.

For denied pending cancellation, output the original public approval link and
`OFFICE_OWNER_CANCELLATION_REQUIRED` (exit 1). The owner can cancel on that page
before approval, then the agent retries unpair. Unavailability is not evidence
that no delayed approval can occur.

Only an explicit `pair` may replace a confirmed revoked receipt with a fresh
request, proof and capability selection under the same identity UUID. Pending
intent remains immutable even after expiry. No block, notebook or profile is
deleted or inherited by re-pairing. Older companions without unpair support
fail rather than claim successful cleanup.

## Invocation-owned renewal

`inspect` refreshes Auth when necessary, then renews a paired lease with five
minutes or less remaining (including expiry) through the authenticated issuer
operation in [pairing v1](pairing-v1.md#lease-renewal). There is no scheduler,
new approval link or daily browser prompt. Local `status` never renews or makes
a network request. `pair` retains its existing acquisition behavior.

The native request supplies the protected record's exact last grant expiry.
Readback must match version, pairing, world, installation, identity, principal,
block and capabilities, and cannot roll the expiry backward. Unknown/duplicate
fields reject. The service owns lease bounds; native never computes a new expiry.
Only the validated server value is written back through the existing protected
record and readback owner. A lost response retains the old lease for retry;
another caller's newer lease is recovered without extending it again. If that
recovered lease has itself expired, resource use still fails expired; a later
invocation can renew using the recovered expiry. Each invocation is bounded,
not an automatic retry loop. Denied, malformed or uncertain renewal never
authorizes resource use or creates a replacement pairing.

## Identity retirement hooks

Core identity UUID/lifetime remains authoritative. Confirmed temporary pane loss,
explicit unbind of a temporary identity and explicit removal use the same
retirement transaction. Saved detachment is not retirement; uncertain pane
evidence must not authorize it. No command-specific Office cleanup is added to
these paths.

Before exposing a new approval link, Office saves the protected scope, registers
its opaque scope key for consumer `tmt-office`, and rechecks the active UUID.
Accessing an existing scope for pair/inspect also registers it. Registration and
retirement serialize in SQLite: active subscriptions are `registered`, retired
ones are `pending`; retirement and enqueue commit or roll back together.
Repeated registration preserves `delivered` receipts. A replacement with the
same display name has a new UUID and inherits neither scope nor hooks.

`pair` and `inspect` first attempt pending cleanup at the shared Office boundary.
`tmt office sync [--prefix <folder>] [--json]` explicitly consumes notifications
without an active identity, tmux, world selector or new approval. Local `status`
and ordinary identity commands never perform this remote work. Without an Office
invocation there is no background delivery guarantee; closing a pane is not
proof that the remote grant has already been disabled.

Delivery attempts at most 16 records within 25 seconds, least-attempted first.
Attempts are recorded before external effects so inaccessible scopes cannot
permanently starve other records. Each consumer uses the existing scope lock,
loads the protected record, validates its derived scope against the hook, and
calls the pinned issuer's `/revoke`. Pending scopes use their original proof;
paired scopes use their bound agent token, refreshing Auth only if needed for
this authority-reducing operation. No resource renewal or access is permitted
for a retired UUID.

Only an exact confirmed revocation response permits replacing the protected
phase with `revoked`, without proof or tokens. Exact vault readback precedes
SQLite acknowledgment. A crash between remote confirmation, vault publication
and acknowledgment can replay revocation; the issuer is idempotent, and a
protected terminal receipt permits acknowledgment without credentials. No SQL
transaction is held during vault or network work. Missing/locked/corrupt state,
unknown pending approval, denial and uncertainty remain pending, never silently
deleted or reported as revoked. Resources and historical identity data remain.

Explicit sync returns `{completed,failed,pending,failureCode}`; `failureCode` is
null when no attempt failed. Exit 0 requires no failures or pending work; exit 1
also covers partial batches. An initialization failure uses the normal error
envelope. Pair/inspect warn on unresolved cleanup, then independently validate
their requested scope; they cannot use an unrelated cleanup failure as authority.
Retry sync after resolving the reported obstacle, not in an unbounded loop.

Existing pre-hook vault entries enroll on their next scoped pair/inspect access.
The OS vault has no portable enumeration contract: unknown scopes retired before
enrollment cannot be reconstructed or claimed as cleaned. There is no generic
plugin runtime, daemon, automatic migration of secrets or credential-deletion
shortcut in this mechanism.
