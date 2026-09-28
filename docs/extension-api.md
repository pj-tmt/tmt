# Local extension API

Use the invoking executable supplied by extension dispatch:

```sh
printf '%s\n' '{"version":1,"operation":"capabilities","input":{}}' |
  "$TMT_EXECUTABLE" api
```

No Office installation is required. Capabilities list supported operations and
byte bounds without creating state. Close stdin within five seconds. Exit 0
means the operation returned a resource; exit 1 returns
`{"error":{"code":"...","message":"..."}}`. This is a local same-user
interface, not remote authorization. The [architecture contract](../ARCHITECTURE.md#local-extension-api-v1)
owns compatibility, transport and persistence rules.

Every request has `version`, `operation` and `input`. Writes additionally require
`identity`, an active identity UUID or name; reads omit it. Unknown request fields
are rejected. Responses reuse existing resource shapes, without a second wrapper.
Clients must tolerate additive response fields.

| Operation                | Input                                                             | Result                                                                                  |
| ------------------------ | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `capabilities`           | `{}`                                                              | Protocol range, operations, byte limits and ordinary commands                           |
| `requests.list`          | `recipientId` and/or `roomId`, optional `limit` and `before`      | `items`, `nextBefore`                                                                   |
| `requests.show`          | `requestId`                                                       | Request detail including retained prompt/final state                                    |
| `dispatch.show`          | `operationId`                                                     | Immutable acceptance receipt                                                            |
| `dispatch.create`        | `operationId`, `recipientIds`, `message`, optional `kind`, `room` | Acceptance receipt; optional independent `wake` on first direct request                 |
| `rooms.write`            | `roomId`, `room: {expectedRevision, name, memberIds}`             | Room resource                                                                           |
| `rooms.roster`           | `room` (UUID or unique exact name), optional `metadataPrefix`     | `room` resource and `members` with metadata and status                                  |
| `notes.read`             | `identityId`                                                      | Saved identity's `identityId`, `name`, `content`                                        |
| `identityHooks.register` | `consumer`, `identityId`, `reference`                             | `state`: `registered`, `pending` or `delivered`                                         |
| `identityHooks.pending`  | `consumer`, `limit` (1–16)                                        | This consumer's `hooks` (`identityId`, `reference`, `attemptCount`) and `pending` count |
| `identityHooks.attempt`  | `consumer`, `identityId`, `reference`                             | `recorded`                                                                              |
| `identityHooks.ack`      | `consumer`, `identityId`, `reference`                             | `acknowledged`                                                                          |

IDs are canonical UUIDs, except request IDs, which use TMT's `req_...` format.
`dispatch.create.kind` defaults to `request`; `announcement` does not expect a
reply. Optional room scope is `{kind:"direct",roomId}` or
`{kind:"roster",roomId,revision}`. Roster dispatch checks exact membership and
revision. Persist a new operation UUID before sending; retries must retain that
UUID and the exact normalized intent and originator. Changed intent conflicts.
An acceptance receipt is not proof of delivery or processing. An absent `wake`
on replay is intentional; an offline request remains queued without automatic
re-wake. Recover with `dispatch.show` after uncertain process completion.

For room creation use a new UUID and `expectedRevision:0`; updates use the current
revision. Refresh rather than blindly retrying a stale write. The returned resource
matches the `room` member of `tmt room show <id> --json`.

History list defaults to the canonical history page limit. Pass `nextBefore`
unchanged as the next request's `before`. Concurrent new requests above that cursor
will appear on a fresh first page; final-state changes can appear when detail is
reread. This is not a live change feed. Reads never mark incoming work as read.
Use `tmt x` and its revision cursor for attention, and the ordinary JSON commands
for identity, presence, room list/show/retire, reply and result. Notes accepts a
saved identity UUID, never a caller-selected path, and does not initialize a file.

Identity hooks are durable identity-retirement subscriptions. A consumer
(lowercase letters, digits, `-` or `_`, starting with a letter, at most 64 bytes)
registers an opaque `reference` for one identity UUID; registering after
retirement is pending at once, and repeating a registration returns the current
state. Every operation is scoped to the named consumer: pages, attempts and
acknowledgments never see another consumer's hooks, and an unknown hook returns
`HOOK_NOT_FOUND`. Record an attempt before acting, act idempotently, and
acknowledge only after the action is durable; a failure leaves the hook pending.
`HOOK_NOT_PENDING` means the identity has not retired. Delivered is terminal:
repeated attempts and acknowledgments return `false`. These operations are not
identity-attributed writes and take no `identity`. An unknown identity UUID on
registration returns `IDENTITY_NOT_FOUND`.

`rooms.roster` reads an active room's non-retired members in the room's member
order. Each member is the identity summary from `tmt identity show --json`, plus
`metadata` (only keys starting with `metadataPrefix`, a literal prefix using the
metadata key grammar) and `status` as in `tmt identity status show --json`
(`null` when absent; expired status is returned with `stale: true`). One response
comes from a single consistent snapshot. It does not include presence: join it
with `tmt ls --room <roomId> --json`. An unknown or retired room returns
`ROOM_NOT_FOUND`, and a shared name returns `ROOM_AMBIGUOUS`; select by UUID.

Example conditional room write (replace the UUIDs with actual identities):

```json
{
  "version": 1,
  "operation": "rooms.write",
  "identity": "Alice",
  "input": {
    "roomId": "00000000-0000-4000-8000-000000000001",
    "room": { "expectedRevision": 0, "name": "Review", "memberIds": [] }
  }
}
```
