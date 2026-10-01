# Identity self-reported status

Status is identity-owned, transport-neutral descriptive data. It is neither
endpoint presence, permission to dispatch, nor evidence that a request is running
or complete. There is one current record per identity UUID, outside Office layout,
appearance profiles and free-form metadata. All callers use the same core service
and SQLite repository; no room-specific copies or status event log are required.

## Commands and values

```sh
tmt identity status set "Reviewing the renderer" --mood "focused" --for 60m
tmt identity status show --identity Alice --json
tmt identity status rm --identity Alice
```

Each operation accepts `--identity`. Omission uses the existing verified caller
resolver, never the first/last identity. An explicit identity works outside tmux
and without Office installed or running. Saved and active temporary identities
are eligible; reporting status does not change lifetime, binding or membership.

`set` replaces the complete record: activity is 1–160 UTF-8 bytes, optional mood
is 1–32 bytes. Both must contain non-whitespace text and no control characters.
Blankness uses Unicode `White_Space`, matching the core validator; browser
decoding must not add ECMAScript `trim()`'s additional BOM restriction.
Omitting mood clears it. Text is inert; no markup, shell or executable capability.
The existing duration syntax applies (seconds, `ms`, `s`, `m`); `--for` defaults
to 60 minutes and must be 1 second through 24 hours. No non-expiring status.

The host samples the update time and atomically stores activity, mood,
`updatedAtMs` and `expiresAtMs`. A successful later set replaces an earlier set
and renews expiry, even for identical text. This is last-writer-wins descriptive
state, not the conditional document-edit protocol. A failed validation or inactive
identity check writes nothing. `clear` is idempotent. Neither operation changes
identity creation/update time, Office profile revision, requests or replies.

`show` returns `{identityId, status}`. A present status contains `activity`, nullable
`mood`, `updatedAtMs`, `expiresAtMs`, and derived `stale`. Absence is `status: null`.
`set` returns the same shape; `clear` returns `{identityId, removed}`. Missing or
retired identities are errors, distinct from an active identity with no status.
At `now >= expiresAtMs` the record is stale. A clock earlier than the recorded
update also fails closed as stale. Reading never deletes or renews a record.

## Lifecycle and Office projection

Promotion keeps the UUID and record. Unbinding/offline saved identities keep
their status; presence is displayed independently. Retirement hides status from
active projections and prevents updates. Reusing a retired name creates a new
UUID with no inherited status. Existing requests and content are untouched.

Office reads this canonical record, not a status embedded in the map/profile.
Show one short activity/mood cue for fresh status, with reply-ready attention
taking precedence. Details show exact text, update/expiry times and a stale label;
stale text must not appear as current activity. Merely sending work never sets
status or creates typing/working indicators. Browser rendering must expire a
previously fresh cue even without another network read.
Directory/appearance observation must not reuse profile revision as a status
version: saving an avatar cannot restore an older status or erase a newer one.
Read status in a batch for the identity directory, not one query per room/actor.
Expiry rendering should schedule the next relevant deadline and resample when
the page becomes visible, not add a continuous animation or per-actor poll loop.

## Verification

Verify byte/control/duration boundaries, exact expiry and clock rollback, atomic
replacement and clear, restart persistence, temporary-to-saved promotion,
retirement/name reuse, explicit and inferred caller resolution, no tmux probe for
explicit selection, JSON/human/help output, and invalid-input preservation.
CLI-to-Office tests must prove one durable record drives both views, expiration
removes only the current cue, and reply attention wins without modifying status.
