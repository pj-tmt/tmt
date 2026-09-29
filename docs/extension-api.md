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
| `skills.install`         | `owner`, `consent: true`, `skills`, optional `force`              | `owner`, `published` targets                                                            |
| `skills.remove`          | `owner`, `consent: true`                                          | `owner`, `removed` and `kept` targets                                                   |
| `references.resolve`     | optional `identityIds`, `roomIds` (canonical UUIDs, at most 256 in total)                 | `identities` (`id`, `found`, `name`, `lifetime`, `retired`) and `rooms` (`id`, `found`, `retired`)        |

IDs are canonical UUIDs, except request IDs, which use TMT's `req_...` format.
`dispatch.create.kind` defaults to `request`; `announcement` does not expect a
reply. Optional room scope is `{kind:"direct",roomId}` or
`{kind:"roster",roomId,revision}`. Roster dispatch checks exact membership and
revision. Persist a new operation UUID before sending; retries must retain that
UUID and the exact normalized intent and originator. Changed intent conflicts.
An acceptance receipt is not proof of delivery or processing. An absent `wake`
on replay is intentional; an offline request remains queued without automatic
re-wake. Recover with `dispatch.show` after uncertain process completion.

`skills.install` publishes an extension's agent skills into the user's provider
skill directories, so send `consent: true` only after asking the user, as
`tmt extension install` does. `owner` is the extension name; each skill is
`{name, files: [{path, content}]}` with UTF-8 `content`, a top-level
`SKILL.md`, canonical `/`-separated relative paths (no empty or
`.`-prefixed segments, no trailing `/`), at most 64 files of 1 MiB each and
16 skills per call. Core's `tmux-team` and `tmt-inbox` cannot be claimed; the
first owner of any other name keeps it. Office links core published before
owners existed belong to core too, and only owner `office` takes them over
without force. `force: true` transfers a name and backs up an unmanaged path
in the way. Errors are
`SKILL_INVALID`, `SKILL_OWNED_ELSEWHERE`, `SKILL_CONFLICT` (an unmanaged path),
`API_CONSENT_REQUIRED` and `SKILL_INSTALL_FAILED`. Repeating identical content
changes nothing. `skills.remove` removes only links that still point at the
owner's content and reports anything else at a recorded target as `kept`.
Ownership is bookkeeping between cooperating installers of one user, not
authentication: this local API cannot prove which extension is calling.

`references.resolve` answers batch reference lookups in one call. Unknown IDs
return `{id, found:false}` entries rather than errors, so an absent ID is never
confused with a failed lookup; more than 256 IDs or a non-canonical UUID is
`API_INPUT_INVALID`. Retired identities and rooms are `found` with `retired:true`.

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

## Lifecycle hooks

An extension can receive best-effort observations after core commits, once the
user runs `tmt extension hooks enable <name>`. Core then invokes the resolved
`tmt-<name>` executable with `TMT_EXECUTABLE` and `TMT_HOOK_DELIVERY=1` set:

- `__tmt-hooks 1 capabilities`: print `TMT-HOOKS/1`, then one capability token per
  line (for example `lifecycle_observations_v1`), within one second and 1 KiB.
- `__tmt-hooks 1 observe`: read `{"version":1,"events":[...]}` from stdin. Events
  are `identity.created` and `identity.retired`
  (`identityId`, `lifetime`, `retired`) and `room.created`, `room.updated` and
  `room.retired` (`roomId`, `revision`, `retired`).

- `__tmt-hooks 1 context` (capability `context_v1`): read
  `{"version":1,"identityId":"<uuid>"}` and print `{"summary":"<text>"}` (at most
  240 characters) or `{"summary":null}` within the shared 300 ms deadline. It is
  asked only for a verified, bound identity and must be read-only: do not write,
  migrate or start anything.

Summaries are untrusted informational text. TMT attributes them by extension name,
escapes them and labels them `(informational)` when they reach an agent's
context; never phrase a summary as an instruction.

Observations may be dropped, repeated or delivered after later changes; treat
them as a prompt to reconcile, not as a log. Output is ignored, and the exit
status never affects the command. All observers of one command share a 500 ms
deadline. Calls to `tmt` made while `TMT_HOOK_DELIVERY` is set emit no further
observations. Replacing or re-permissioning the executable suspends delivery
until it is enabled again.
