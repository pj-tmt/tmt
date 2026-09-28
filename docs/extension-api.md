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

| Operation         | Input                                                             | Result                                                                  |
| ----------------- | ----------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `capabilities`    | `{}`                                                              | Protocol range, operations, byte limits and ordinary commands           |
| `requests.list`   | `recipientId` and/or `roomId`, optional `limit` and `before`      | `items`, `nextBefore`                                                   |
| `requests.show`   | `requestId`                                                       | Request detail including retained prompt/final state                    |
| `dispatch.show`   | `operationId`                                                     | Immutable acceptance receipt                                            |
| `dispatch.create` | `operationId`, `recipientIds`, `message`, optional `kind`, `room` | Acceptance receipt; optional independent `wake` on first direct request |
| `rooms.write`     | `roomId`, `room: {expectedRevision, name, memberIds}`             | Room resource                                                           |
| `notes.read`      | `identityId`                                                      | Saved identity's `identityId`, `name`, `content`                        |

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
