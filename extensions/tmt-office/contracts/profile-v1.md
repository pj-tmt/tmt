# Local presentation profile v1

Implemented for local Office in #212. Local SQLite is authoritative. Remote
publication and authorization remain M2 (#242).

## Commands and identity

```sh
tmt office profile show --local --identity Alice --json
tmt office profile apply --local --identity Alice --file profile.json --if-revision 0 --json
```

Use the existing verified caller resolver when `--identity` is omitted. Missing
or ambiguous context fails rather than guessing. Both commands require an active
existing identity. They work while the browser service is stopped. `--local` is
explicit; no implicit pairing, publication or fallback to a remote world.

## Input

Apply replaces this complete object; unknown or missing required fields reject. The optional
`avatarRef` shown below may instead be omitted or `null` to select the default robot:

```json
{
  "displayLabel": "",
  "description": "Architecture review",
  "appearance": {
    "hairStyle": "short",
    "hairColor": "ink",
    "skinTone": "medium",
    "shirtColor": "blue",
    "shirtMark": "AI"
  },
  "avatarRef": "sha256:<64 lowercase hex>/<avatar-key>"
}
```

`displayLabel` is at most 80 UTF-8 bytes; empty means the current identity name.
`description` is at most 1024 UTF-8 bytes and may contain LF and TAB. `shirtMark`
is at most 16 UTF-8 bytes, including an empty string, short text or emoji.
Other control characters reject; label and mark also reject LF and TAB. Never
truncate, interpret markup or load resources from these strings. Render them as
plain text with a bounded visual area and accessible full text. No URL, path, SVG,
executable content or arbitrary color field exists. `avatarRef` is only the immutable local
catalog grammar defined by [avatar pack v1](avatar-pack-v1.md). File input is
bounded to 8 KiB before parsing.

Catalog identifiers are case-sensitive:

- Hair styles: `short`, `bob`, `curls`, `tied`, `bald`.
- Hair colors: `ink`, `brown`, `gold`, `silver`.
- Skin tones: `light`, `warm`, `medium`, `deep`.
- Shirt colors: `blue`, `green`, `clay`, `plum`, `gold`, `ink`.

Rendering uses curated repository primitives. The catalog definition and literal
conformance vectors are shared boundaries, not independently invented adapter
inventories. A read returns allowed values so agents can discover valid options.

## Resource and defaults

Persist one independent profile row per immutable identity UUID, with revision,
canonical profile and update time. Do not embed it in layout JSON, identity
metadata, role instructions or the Markdown notebook.

Changing to a different non-null avatar reference requires an installed, valid pack and key.
Admission and the profile write occur in one immediate transaction. The exact current
reference may be retained during unrelated edits even when its pack is missing or corrupt.
Catalog removal never rewrites profiles; rendering falls back to the stored default appearance.
Reinstalling the exact bytes restores the art without a profile write, revision or timestamp
change.

Without a stored override, show returns `exists:false`, revision 0 and a
deterministic default without writing. Defaults use the UUID's first three
decoded bytes modulo the ordered hair-style, skin-tone and shirt-color catalogs;
hair color is `ink`, label/description/mark are empty. The same UUID always yields
the same default. No randomness, model call or clock is involved.

Read output contains identity ID/name, `exists`, `revision`, `profile`,
`updatedAtMs` (null for an implicit default), and the bounded `catalog`.
Human output identifies the identity, revision/default status and profile fields.
Apply adds `changed` to that canonical snapshot. Display labels never become
lookup keys; show the actual identity name alongside a customized label.

## Writes and lifetime

Reuse the existing immediate transaction, active-identity check and avatar-catalog admission.
Revisions
are JavaScript-safe positive integers for stored rows. Revision 0 creates the
first explicit override at 1, including an explicit save of the default.
At the expected revision, identical canonical content is a no-op. At expected+1,
identical content is an exact retry, preserving revision and timestamp. All other
stale writes conflict. Never automatically advance the caller's expected revision.

An uncertain write may have committed: retain the original file/draft and
revision, reread and compare. Missing/retired identities, invalid data and revision
conflicts are distinct failures using the existing Office error conventions.
No profile edit changes identity, role, permissions, layout, notes or position.
Retirement retains the row but excludes it from active projection. A replacement
with the same name receives neither the old override nor its ownership.

## Browser and verification

Add a typed profile port to the existing local runtime, sharing authentication,
origin checks and cancellation. Native and browser mutations invoke the same
domain service. Profiles and world layouts retain independent revisions.
The local directory projection includes required `lifetime: "saved" | "temporary"`
beside `online`; both come from the native identity/presence owner, never from
the profile description or the existence of a personal area. Individual profile
snapshots and mutations retain their profile-only shape.
The directory also requires nullable `selfReportedStatus`, the independent
[identity status projection](../../../contracts/identity-status-v1.md). It is not part of profile
storage, the profile revision or mutation input. Directory reads batch statuses;
an appearance save preserves the latest observed status, and a newer appearance
revision does not prevent observing a status update or clear.

Active identities without layouts appear in the avatar preview/selector;
do not create a persisted block on discovery. Reuse a curated avatar renderer for
preview and scene. Custom art owns its palette, so default appearance controls remain stored
but disabled while a custom reference is selected. `shirtMark`, identity name and display
label remain independent inert overlays. Missing or corrupt selected art shows the saved
default robot and an explicit unavailable status. Offline saved identities are not presented as online.
Draft edits do not write until Save and survive refresh/conflict/uncertainty.
Furniture selection remains independent. Movement and social sessions are #179.

Verify literal valid/invalid vectors in native and browser layers; default
read-without-write, CAS/no-op/retry, retirement/replacement, protected local HTTP
access, safe text and unchanged unrelated resources. Integrated proof uses real
CLI -> SQLite inspection -> browser display -> restart, without cloud accounts
or paid agents. Include desktop and narrow screenshots before visual acceptance.
