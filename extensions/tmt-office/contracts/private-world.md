# Private world document v1

Implemented Firestore document contract, not an HTTP envelope or
the work-dispatch protocol. Security Rules are the enforcement boundary; browser
validation provides feedback, not authority.

`worlds/{worldId}` uses a Firestore-generated 20-character alphanumeric ID and
exactly these fields:

| Field       | Value                                                                          |
| ----------- | ------------------------------------------------------------------------------ |
| `version`   | Number `1`                                                                     |
| `name`      | Nonblank string, at most 80 Unicode scalar values, no ASCII control characters |
| `ownerUid`  | Creating Google-authenticated human's Firebase UID                             |
| `createdAt` | Firestore server timestamp at creation                                         |

All fields are immutable. There is no delete or global listing.
One online transaction reads the selected ID, creates if absent, or returns the
same document when owner/name/version match. Conflicts never overwrite. The UI
keeps the same ID and name for an uncertain retry. Deduplication lasts while the
world exists; no offline write queue, expiring request record or second owner
membership document is involved. An approved user may read an absent ID for the
transaction; denied and absent worlds have the same user-facing unavailable view.

`testers/{uid}` is an operator-managed document with exactly `{ enabled: true }`
for admission. Missing, disabled or malformed documents deny world access.
Verified Google users may get their own admission document, but cannot list or
write tester documents. Admission does not grant access to other owners' worlds.
The Firebase Console operator uses IAM, not a client-side administrator role.

The [home block v1](block-v1.md) contract opens only
`worlds/{worldId}/blocks/home` to its approved world owner. The
[agent grant v1](agent-grant-v1.md) boundary also permits scoped UUID blocks and
owner revocation of trusted grants. Other blocks and `messages/{messageId}`
remain denied. World data does not authorize native agent execution.
