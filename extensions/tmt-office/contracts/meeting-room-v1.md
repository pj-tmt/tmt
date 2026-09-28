# Local meeting rooms v1

Local implementation, not a released feature. Meeting membership is an explicit
installation-local resource, not a personal room layout, visual proximity, online
presence or the global identity directory. Joining a room does not dispatch work.
An identity may belong to multiple meeting rooms.

## Resource and editing

`GET /api/v1/local/rooms` lists room snapshots under the existing local browser
bearer. `PUT /api/v1/local/rooms/<uuid>` also requires the exact loopback Origin
and JSON content type. Unsupported paths/fields and malformed writes fail before
storage opens. The write body is bounded to 8 KiB:

```json
{
  "expectedRevision": 0,
  "name": "Design review",
  "memberIds": ["22222222-2222-4222-8222-222222222222"]
}
```

The caller allocates one canonical non-nil room UUID. Revision zero creates;
existing rooms require the current positive revision. Names are inert display
labels, at most 80 UTF-8 bytes, containing non-whitespace text and no controls.
Names are not identity aliases or unique routing keys. HTTP selection uses room UUIDs;
CLI selection accepts an exact name only when it matches one room. Ambiguity is
an error containing candidate UUIDs, never first-match routing.
Members are stable identity UUIDs, deduplicated and sorted. Each write accepts at
most 64 supplied UUIDs before deduplication, matching the bounded request
composition audience. Empty rooms are
valid but cannot receive a request. Missing/retired identities cannot be added.

The response is `{id,name,revision,memberIds}`. Actual definition changes increment
the revision; a same-value write at the correct revision is a no-op. Definition
and member rows change atomically. A stale revision is HTTP 409
`ROOM_REVISION_CONFLICT`, inactive members are HTTP 409 `ROOM_IDENTITY_INACTIVE`,
invalid input is HTTP 400 `ROOM_INVALID`, and storage failures are HTTP 500
`STORAGE_UNAVAILABLE`. Shared HTTP framing errors remain `BAD_REQUEST`.

Room updates use conditional PUT, not a request-operation ledger. After an
uncertain save, retain the UUID, expected revision and draft; refresh to inspect
the saved roster. Never silently advance the expected revision or merge members.
Retrying the same UUID cannot create a second room. The browser keeps failed
drafts until explicit discard; room editing never automatically adopts an audience
for a pending request.

## Effective membership and dispatch fencing

Reads include active identities whether online or offline. Retirement excludes an
identity from the effective roster without deleting historical membership rows or
request receipts. Reusing the name creates a different UUID and does not rejoin.
This follows existing retained Office-resource projections rather than adding
room-specific behavior to core identity retirement.

`revision` versions explicit room definition edits, not identity lifecycle. A fan-out
therefore fences **both** that revision and the exact effective member UUID list.
The [dispatch input](dispatch-v1.md) still carries `recipientIds` and optionally:

```json
{ "room": { "kind": "roster", "roomId": "11111111-1111-4111-8111-111111111111", "revision": 3 } }
```

For a new operation, the host rereads the roster inside the same immediate
transaction as request enqueue. A missing room, changed revision or any audience
difference returns HTTP 409 `ROOM_ROSTER_CHANGED`, writing no request or operation
receipt. This includes retirement between preview and Send. The client preserves
the question, refreshes the room and requires explicit adoption and re-preview.
It never expands recipients or silently falls back to explicit-identity mode.

Direct conversations use `{kind:"direct",roomId}` with exactly one recipient.
They share the canonical membership check, request storage and operation receipt,
but not the full-roster revision fence. Unrelated members joining or leaving do
not invalidate a private send. Missing target membership rejects the new send;
room context is history classification, not an access-control boundary.

Committed operation replay is checked **before** current membership. Resubmitting
the exact original dispatch input after membership changes returns its acceptance receipt,
not a rejection that could encourage duplicate work. The room fence is included
in the operation intent digest; changing it under a used operation ID conflicts.
Explicit-identity requests omit the field (JSON null is also normalized to absent)
and preserve their existing digest and partial-unavailable semantics.

Accepted room dispatch persists the original room UUID on each canonical request
attempt (schema 26). It is historical communication context, not live membership
or an authorization grant. Request retention owns its lifetime; no room foreign
key, message-text inference or backfill from operation receipts is used. Older
requests remain unscoped. Leaving a room does not hide delivered requests or their
later responses.

## Ownership and scope

The browser's **Meeting rooms** manager uses this same conditional resource API,
including empty rooms. **Save room** is immediate and independent of layout
Save/Cancel. Physical areas link a room UUID; creating or changing a room does not
create geometry, allocate a personal office or dispatch work.

The layout editor's explicit **Add meeting set** adds ordinary furniture and
functional placements in a clear 36 × 32 area. All additions form one Undo/Redo
step and remain unsaved until layout Save. The room UUID is the default whiteboard
document reference; multiple physical areas may open the same board. No board
content is materialized by the preset or world Save. Discussion opens the room's
category in the existing board store, and broadcast requires explicit audience
selection/review. Lobby discussion remains General; scope is not an ACL.
Rebinding/removing a physical area leaves existing object bindings, room members,
content and request history intact. It never retargets an existing whiteboard or
discussion binding.

Core `room` owns room values, the repository port and exact-name/UUID resolution,
using dispatch's existing stable-ID and bounded-audience rules. Adapter `room`
owns shared wire values; `storage::room` owns
schema 24 definitions/members and coherent read snapshots. Dispatch borrows that
reader inside its existing transaction. There is no second request store or
online-presence dependency.

The storage boundary also supports atomic Join/Leave set edits. It reads and
changes the current effective roster inside one immediate transaction, reusing
the same writer as conditional full-roster updates. Duplicate joins and absent
leaves preserve the revision; concurrent joins retain both members. The CLI uses
that operation directly; HTTP retains conditional full-roster replacement.

## Local CLI

### Retirement

`tmt room retire <room>` permanently retires a communication room, not its spatial
area. The shared repository performs a revision-checked transition; HTTP uses
`POST /api/v1/local/rooms/<uuid>/retire` with `{expectedRevision}`. Room snapshots
include `retired: boolean`. An exact retirement retry returns the retained retired
snapshot without another revision change. A stale revision fails without writing.

Retirement preserves the room UUID, roster rows, spatial references, whiteboards,
discussions and delivered requests. Active lists and new room selection, membership
edits, dispatch and new spatial bindings exclude retired rooms. An existing area's
unchanged binding may remain while its furniture or geometry is edited. Retiring a
room never removes a map area or changes a saved layout.

`room show <uuid>`, `x listen --room <uuid>` and `office board list --room <uuid>`
may resolve a retired UUID for history;
retired names are not selection aliases. A newly created same-name room has a new
UUID and inherits no members or content. Existing request reply/ack operations and
committed exact dispatch and discussion-post replays remain valid. New discussion
threads require an active room; existing threads remain readable and editable.
There is no restore or cascade delete.

These source commands need neither Office installation nor a running web service:

```sh
tmt room create "Design"
tmt room ls
tmt room show <room-uuid-or-exact-name>
tmt room join <room> --identity Alice
tmt room leave <room> --identity Alice
tmt ls --room <room>
tmt x listen --room <room> --identity Alice
tmt room send <room> "Review this change" --identity Alice
tmt room broadcast <room> "The review is ready" --identity Alice
tmt talk Alice "Review this change" --room <room> --inbox --detach
```

Join/leave may omit `--identity` only when the existing verified-pane resolver
proves the caller. Explicit identities do not probe tmux. Create makes an empty
room; it does not enroll the caller or save furniture. Duplicate display names
are permitted; resolve them by UUID. Listing and membership are not authorization
boundaries. `ls --room` filters the existing presence projection and preserves
its reconciliation policy; it cannot be combined with a positional identity/pane.

JSON uses `{room:{id,name,revision,retired,memberIds}}` for create/show/join/leave/retire and
`{rooms:[...]}` for room list. Scoped `ls` preserves its existing `identities`
envelope. Success exits 0; not-found room or identity exits 3; invalid/ambiguous
selection, revision conflict and storage failure exit 1 with the normal structured
CLI error. Human output uses the shared aligned table; `-h` and `--help` use the
same grammar as parsing and completion.

Scoped listen resolves the room once and applies that UUID to both the incoming
watermark and paginated request/response query. Other rooms do not wake this
listener. It preserves existing timeout, debounce and acknowledgment behavior;
it does not join the room, acknowledge work or send anything. Scoped exchange
summaries/details include `roomId`; unscoped JSON retains its previous shape.
Participant-and-room indexes avoid scanning unrelated room history for observation.

Send queues one replyable inbox request per effective member; broadcast queues
no-reply announcements through the same owner. Neither waits for completion or
pastes into tmux. The sender is included if it belongs to the room; no implicit
audience exclusion occurs. Explicit `--identity` selects the sender, omission
uses normal verified caller resolution or records an unknown owner. Neither
requires the sender to join: room membership is communication scope, not an ACL.

Both return the [shared acceptance receipt](dispatch-v1.md#acceptance-failures-and-replay)
as JSON (no extra wrapper), or an operation ID and per-recipient table for humans.
Exit 0 means a committed receipt, not that every recipient completed work. Empty
rooms fail with `ROOM_EMPTY` and no request writes. Invalid intents, changed
rosters and conflicting retries exit 1 with the shared dispatch error code.
`--operation-id <uuid>` preserves the retry identity; use the same sender,
message and room snapshot. Each CLI invocation resolves the current active room
and composes its audience; it does not retain the previous invocation's roster.
A changed roster therefore conflicts rather than delivering to newly joined
members, and a retired room fails active selection. This differs from a browser
resubmitting its frozen dispatch input or looking up an existing receipt.
Uncertain failures expose the operation ID; do not
switch to a fresh ID solely because a receipt was not observed.

Direct `talk --room` keeps normal pane/inbox routing, timeout and detach behavior.
It resolves exactly one target, requires that target's identity to be a member,
and never expands the audience. An unbound pane or non-member fails before sending
with `ROOM_RECIPIENT_NOT_MEMBER`. The request service rechecks membership inside
the preparation transaction so a stale CLI selection cannot bypass the check.
History reads and replies do not recheck current membership.

The [Office interaction contract](rooms-and-walls.md) owns durable browser
conversations and spatial integration.

The browser room port uses `LocalRuntime` transport. `RoomPicker` explicitly
creates/edits/adopts a roster, while `IdentityChecklist` shares member selection
with individual requests. Whiteboard request state owns the frozen fence and
intent; room editing and list refresh do not update that intent implicitly.

The [whole-world map](world-v1.md) binds meeting areas to these UUIDs without owning
membership. Removing an area preserves the room; retiring a room preserves the area.
The shared request/broadcast composers adopt explicit active rosters. Remote membership
is not implied; extensions reuse this resource rather than creating another roster.
