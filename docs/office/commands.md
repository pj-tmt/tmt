# Office command experience

## Implemented local installation

The CLI exposes `office install`, `upgrade`, `status` and `uninstall`. Office is
optional and independently versioned. A public alpha companion is available
through the verified online installer; source builds and explicit local archives
can also exercise this boundary. Do not confuse successful local installation
with pairing or a running connector.

```sh
tmt office install --yes
tmt office status --json
tmt office upgrade
tmt office uninstall --yes
```

All accept `--prefix <folder>` inside the Office subtree; omission selects
`~/.local`. Use the same prefix for subsequent operations. First installation
defaults to alpha; install/upgrade retain the recorded channel unless explicitly
given `--channel stable|alpha`. Office uses immutable `tmt-office-v<version>` releases
and the shared native archive verifier. Explicit offline installation uses
`office install --yes --archive <file> --manifest <file>` with both inputs.
Without a published candidate, online installation fails rather than claiming
success. `tmt upgrade` continues to update only the CLI and its managed skills.

Explicit Office install and upgrade also manage the optional `tmt-office`,
`tmt-prop-create` and `tmt-avatar-create` skills
through the existing provider/custom-root, immutable asset, registry, drift,
backup, and refresh owner. Core `tmt install` does not expose them. Existing
conversations can read either exact source with `tmt learn --skill <name>`.
An unmanaged target is preserved; after inspection, `--force` on Office install
or upgrade creates a recoverable backup. Companion activation can complete before
skill publication fails, and that partial result does not roll back the binary.
JSON partial results retain the successful companion fields, a bounded `skills`
report, an `error`, and `skills.pendingBackup` when publication failed after a
forced backup. Office uninstall retains managed guidance.

Bare `tmt office` is an inspection-only human entry point. It reports the
installed companion and local service, then names explicit `start` and Office
skill guidance actions; it never installs, starts, pairs or opens a browser.
Its JSON document is identical to `office status --json`.

`status --json` returns `installed`, `version`, `protocolVersion`, `executable`
and token-free local service status after local ownership/integrity and handshake
verification. It never checks cloud
availability. `office start` returns a private local browser URL for the
current service start; open it locally and do not disclose the token. Uninstall
requires explicit consent and removes verified activation links only. Release
files and unrelated data remain. A partial removal reports an invalid
installation; repeat explicit uninstall to finish before reinstalling.

## Local Office

The following local service is implemented and tested in this source tree. Published
artifacts may lag until a coordinated compatible CLI/Office release; do not advertise
source-only `start` or `--local` support from an older pair. When a schema advance
requires it, install the compatible CLI before activating the companion: an older
CLI may reject the upgraded shared database. Never publish an incompatible Office
companion alone. Follow the [native installation guidance](../NATIVE-INSTALL.md)
for release status and compatibility.

The installed companion can serve its embedded Office UI and installation-owned
SQLite state without pairing, Firebase or network access:

```sh
tmt office start
tmt office layout show --json
tmt office layout apply --file layout.json --if-revision <revision> --json
tmt office profile show --local --identity Alice --json
tmt office profile apply --local --identity Alice --file profile.json --if-revision 0 --json
tmt office prop validate --file pack.tmtprop.json --json
tmt office prop preview --file pack.tmtprop.json --json
tmt office prop install --local --file pack.tmtprop.json --if-revision 0 --json
tmt office prop remove --local sha256:<digest> --if-revision 1 --json
tmt office prop list --local --json
tmt office prop show --local sha256:<digest> --json
tmt office avatar validate --file bot.tmtavatar.json --json
tmt office avatar preview --file bot.tmtavatar.json --json
tmt office avatar install --local --file bot.tmtavatar.json --if-revision 0 --json
tmt office avatar remove --local sha256:<digest> --if-revision 1 --json
tmt office avatar list --local --json
tmt office avatar show --local sha256:<digest> --json
tmt office extension validate --file definition.json --instance instance.json --json
tmt office stop
```

`layout` addresses the complete local world, not an identity or remote block.
Copy only the read result's `.layout` into the file: `{version,map,objects}`.
For the first save (revision 0), also pass `--legacy-basis <legacyBasis>` from
that same read; omit it at later revisions. The command and browser share one
atomic revision fence. A conflict never rebases automatically; preserve the
draft and reconcile against a fresh read. No running service or identity is needed.
`office block --world ...` remains the separate remote Firestore operation.

Extension preflight is read-only and checks the paired data structure, not installed
artwork or runtime host authority. See the [extension contract](../../extensions/tmt-office/contracts/extension-v1.md#authoring-preflight).

Prop and avatar `validate` also return advisory `warnings`; valid artwork still
exits successfully. Review those hints and the actual preview rather than treating
an empty warning list as visual approval. See [avatar authoring warnings](../../extensions/tmt-office/contracts/avatar-pack-v1.md#authoring-warnings).

`start` prints a loopback session URL and never opens a browser. It has no identity
selector. The service binds only `127.0.0.1`; the fragment token is removed from the
address after startup and kept only in that tab's memory. Repeating `start` reuses the
running service and token so existing tabs continue working. `stop` is an idempotent,
authenticated graceful stop. A companion upgrade never replaces a running process:
`status` reports `restartNeeded`, and `start` returns `OFFICE_RESTART_REQUIRED` until
you explicitly stop and start it. An optional `--port <number>` requests a fixed
loopback port; it conflicts with a running service on another port.

The UI instructions in this section describe the current local editor. The
[modular-cell replacement](../../extensions/tmt-office/contracts/rooms-and-walls.md) is a design
target, not yet an installed command or UI capability.

Open the full printed URL in your browser. **Directory** lists identities and
areas; **Edit layout** edits the shared map, assignments and furniture in one
Undo/Redo draft. **Save layout** writes the complete candidate; **Cancel** leaves
the saved world unchanged. Saved identities without an assigned office and
temporary contractors belong in the Lobby. Offline identities remain in the
directory but are not drawn as present.

**Remove area designation** is not terrain or content deletion: personal
residents return to Lobby; meeting rooms retain membership and resources; floor
and placements remain. Removing the primary Lobby requires a valid replacement
in the same draft. Floor or wall edits that strand objects are rejected and list
the affected placements. Move or remove those placements explicitly; Save never
silently crops them. Cancel and failed Save leave durable state unchanged.

**Meeting rooms** creates or edits canonical rooms and membership. **Save room**
is immediate and independent of layout Save/Cancel. To furnish one, create or
select a meeting area in the layout editor, choose its linked room and paint its
floor. **Add meeting set** adds a table, four chairs, plant, whiteboard,
room-scoped discussion board and broadcast station in a clear 36 × 32 rectangle.
It does not replace existing objects or send messages. Review, move or undo the placements
before **Save layout**. The whiteboard uses the meeting UUID and saves content
only when explicitly requested. Rebinding/removing an area does not retarget or
delete its existing resource objects. Discussion opens the linked room category; the
broadcaster still requires explicit audience selection and review.

The area/agent inspector lists tools in the selected area first, then common-floor
tools. Expand **Other areas** to browse remaining tools; coordinates distinguish
multiple objects with the same name. Removing an area designation moves its retained
objects into the common-floor list without changing their underlying resources.

In the local workshop candidate, selecting an agent also offers **Message**.
Selecting from a meeting scopes the direct message and history to that room;
only the selected agent receives it, not the roster.
Type a message and **Send** (or press Enter; Shift+Enter adds a newline). The agent receives it
through `tmt x listen --identity <name>` and replies with its normal request
receipt; the panel shows that durable reply separately from delivery and acknowledgment.
Sending does not type into a tmux pane. Closing pauses updates and keeps the
draft; reopening reads retained history. Refresh resumes a paused observation.
After an uncertain send, **Retry** checks acceptance and keeps the original
operation if resending is needed. Reload through the full
session URL from `office start`; the bearer token is never saved to browser storage.

Local profile commands are also one-shot and do not require the browser service. A missing
override returns `exists:false`, revision 0 and the deterministic UUID-derived catalog
default without writing. Apply accepts the exact object documented in
[`profile-v1.md`](../../extensions/tmt-office/contracts/profile-v1.md), rejects files above 8 KiB and
requires `--local`. An identical apply at the current revision is a no-op; an exact retry
at the preceding revision returns the committed snapshot. Other stale revisions return
`OFFICE_REVISION_CONFLICT`. After an uncertain write, reread before retrying.

Profiles may include `"avatarRef":"sha256:<digest>/<key>"`. Omit the member or set it to
`null` to use the saved default robot appearance. Selecting a new reference requires that
exact pack and key in the local avatar catalog; otherwise apply returns
`OFFICE_AVATAR_NOT_FOUND` without changing the profile. A currently retained reference may
still be saved while editing unrelated fields after its pack is removed or becomes corrupt.
Removal does not rewrite profiles, and reinstalling the exact pack restores custom art
without changing the profile revision.

Local prop validation and catalog commands follow the single data-only owner in
[`prop-pack-v1.md`](../../extensions/tmt-office/contracts/prop-pack-v1.md). Preview requires the
already-running local service and never starts it implicitly. The optional
`tmt-prop-create` guidance is installed through the existing Office managed-skill
path; it is not a second installer or executable extension.

Local avatar validation, catalog, expiring preview and profile selection follow
[`avatar-pack-v1.md`](../../extensions/tmt-office/contracts/avatar-pack-v1.md) and use a separate
revision/cursor namespace. The optional `tmt-avatar-create` guidance is installed with the
Office skills and explains the exact validate, preview, install and profile-apply workflow.
The browser selector
shows default, available custom and unavailable-fallback states; default palette controls are
disabled while a custom reference is selected, while identity text and `shirtMark` remain
independent overlays.

With `--json`, successful start returns `running:true`, `changed`, `reused`, `url`
and `version`; stop returns `running:false` and `changed`. Local layout results
contain nullable `worldId`, `revision`, `legacyBasis`, the complete v1 `layout`,
`updatedAtMs` and `changed`. Revision 0 is a read-only projection; the first Save
materializes the installation world, not identity-owned blocks. Profile
reads return `identityId`, `identityName`, `exists`, `revision`, `profile`, nullable
`updatedAtMs`, and the bounded literal `catalog`; apply adds transactional `changed`.
Human apply output distinguishes created, updated and unchanged results. Success exits 0. Usage, installation,
I/O and service lifecycle failures exit 1; existing
identity resolution retains its documented not-found exit. `OFFICE_PORT_UNAVAILABLE`
means the requested port could not bind, `OFFICE_SERVICE_CONFLICT` means a healthy
service owns another port, `OFFICE_RESTART_REQUIRED` requires explicit stop/start, and
`OFFICE_SERVICE_UNCERTAIN` refuses to signal a process whose receipt cannot be
authenticated. A local apply launch failure is `OFFICE_LOCAL_UNCERTAIN`: reread before
retrying because the commit outcome is not assumed.

### Status and mood

```sh
tmt identity status set "Reviewing the layout" --mood focused --for 60m --identity Alice
tmt identity status show --identity Alice --json
tmt identity status clear --identity Alice
```

These core commands also work while Office is stopped, for saved identities and
active Contractors. Opening or refreshing Office observes updates; expiry hides
the character cue without a refresh. Info retains the text and timestamps as
stale. A reply-ready cue takes priority. Status is self-reported, separate from
endpoint presence and request completion; see the [status contract](../../contracts/identity-status-v1.md).

### Office storage

```sh
tmt office storage status [--json]
tmt office storage migrate [--yes] [--json]
```

Office data starts in the shared tmt database. `status` reports where it lives,
how much there is, whether the local service is running and any retained
backups; it changes nothing. `migrate` prints the plan, including the backup
location and the free space it needs, and asks for confirmation on a terminal;
without a terminal it requires `--yes` and otherwise changes nothing. It stops the
local Office service, backs up the whole tmt database and Office configuration
under `<global>/backups/office-storage-<UTC time>/`, and moves Office data into
`<global>/office/office.db`. The move is forward-only; older tmt and tmt-office
versions cannot write migrated data, and undoing it means restoring the backup
(see [Office storage recovery](architecture.md#office-storage-recovery)). Start
the service again afterwards with `tmt office start`. When a migration is
available, `tmt office status` and `tmt office start` print a one-line hint; set
`TMT_HINTS=off` to hide it.

### Local discussion board

The optional companion also owns an installation-local discussion board. Its
one-shot commands work while the browser service is stopped:

```sh
tmt office board post --general --identity Alice --title "Review" --body "Please review." --json
tmt office board list --repo origin --view updated --limit 20 --json
tmt office board list --room "Design review" --view updated --json
tmt office board show <thread-id> --reply-limit 20 --json
tmt office board reply <thread-id> --owner --file reply.txt --json
tmt office board edit <entry-id> --identity Alice --title "Revised" --body "Updated" --if-revision 1 --json
tmt office board delete <entry-id> --owner --moderate --if-revision 1 --json
```

`--repo` resolves a named Git remote locally into a credential-free category;
it never contacts that remote. `--room` selects a canonical meeting UUID or exact
unambiguous name; it is mutually exclusive with `--general` and `--repo` for post
and list. Room categories classify discussions, not permissions or physical
occupancy, and replies retain their thread's category. Mutation
actors are either `--owner`, an explicit active identity, or a verified bound
caller when both are omitted. Bodies preserve exact text and may come from
`--body`, a regular `--file`, or `--file -` stdin. Edit may change the title and
exactly one body source together. The CLI fixes an operation ID before every
mutation dispatch, prints it in plain receipts, and includes that same ID in an
uncertain-error retry instruction. Supplying `--operation-id` up front is also
supported. An exact retry returns its original body-free receipt, while reusing
the ID for different intent is rejected.

Edits and soft deletion require the exact current positive revision. Deleted
entries keep attribution and relationships but no title or body. List and reply
cursors are invalidated by the next content-changing board mutation; restart at
the first page after `BOARD_CURSOR_STALE`. Receipts confirm the original
operation, not an entry's current content, so use `show` for the current state.

## Owner space review (web)

In an admitted private world, **Agent spaces** lists existing resource grants.
Use **Refresh spaces** for the first page and **Next spaces** to advance. These
records are not online status; lease labels reflect the last fetch. Revocation
retains content, and an expired enabled lease may later renew.

Choose **Open block** to inspect or edit the referenced space using the existing
layout editor. **Open home block** returns to your own layout. Switching spaces
discards unsaved edits; already submitted writes may still complete on their
original target. An absent layout is shown as **No saved layout yet**, not as
deleted content. This editor does not change assignment or recover credentials.

## Native pairing

Pairing requires an installed compatible companion and an unlocked OS credential
store (macOS Keychain or Linux Secret Service). No hosted Office service or
production Firebase deployment is provided. Select a world and an existing
identity explicitly outside tmux:

```sh
tmt office pair --world https://office.example/worlds/abcdefghijklmnopqrst --identity Alice
tmt office status --world https://office.example/worlds/abcdefghijklmnopqrst --identity Alice --json
tmt office inspect --world https://office.example/worlds/abcdefghijklmnopqrst --identity Alice --json
```

Open the printed approval link, sign in as the admitted world owner, recognize
the identity/installation and approve. `pair` waits up to 300 seconds; use
`--timeout 30` for a shorter observer and repeat the same command to resume the
original pending request. `--read-only` requests only layout read access. Omit
`--identity` only with verified pane context. Temporary identities are accepted
without promoting them to saved identities.

The approval form defaults to **New empty block**. To reuse a retired agent's
layout, select its **Retained block** from the revoked grant pages before
approving. The first attempt fixes this choice, including uncertain retries.
The new agent receives a new principal for that block; the previous principal
stays revoked. Profiles and notebooks do not transfer. A reserved/transferred
source may reject if already claimed; use the most recent revoked grant, not
an ancestor. The service can reclaim only an abandoned unclaimed reservation.

World-qualified `status` reports local retained state, not live authority.
`inspect` checks server access and reports only whether the assigned block exists;
it does not expose layouts or list other agents. Revocation can therefore leave
local status `credential` while inspect fails `OFFICE_REMOTE_DENIED`. A same-name
replacement has a different UUID and cannot inherit the pairing. `inspect`
renews a near-expiry or expired lease through the issuer, preserving the same
resource and permissions without daily browser approval. Local status does not
renew. Revoked or missing grants cannot be renewed; expired pending approvals
and lost credentials still need a new owner-approved request.
Identity retirement queues cleanup locally. Pair/inspect attempt queued Office
cleanup; to retry explicitly without an active identity or pane, run:

```sh
tmt office sync --json
```

Use the same `--prefix` as installation. The result reports `completed`, `failed`,
`pending` and `failureCode`; exit 0 means no remaining work or failures in that
batch. Locked credentials or uncertain remote results stay pending. Closing a
pane or running `ls` does not itself contact the service. There is no background
delivery guarantee; resolve the obstacle and retry sync before claiming remote
cleanup. Revocation retains block contents. Saved identities going offline are
not retired and do not enqueue this cleanup.
The [native pairing contract](../../extensions/tmt-office/contracts/native-pairing.md) owns exact
scope, output, errors and emulator restrictions.

Source builds also support `tmt office unpair --world <url> --identity <name>`.
It confirms revocation before retaining a secret-free receipt; an explicit pair
can then start again under the same identity. Pending cancellation may require
the owner to cancel using the original link first. Failure never permits deleting
credentials. This is separate from the proposed connector lifecycle below.

## Agent decoration (source builds)

After pairing, use the same world and identity selectors:

```sh
tmt office block show --world <url> --identity Alice --json
tmt office block apply --world <url> --identity Alice --file layout.json --if-revision 0 --json
```

`show` returns `blockId`, `revision`, readable `objects`, the curated `catalog`
and room `limits`. An absent layout has revision 0 and no objects. An optional
block ID after `show` or `apply` must equal the pairing's assignment; omitting it
selects that assignment, not another identity's room.

The input file contains only `objects`, for example:

```json
{
  "objects": [
    { "asset": "desk", "x": 4, "y": 6, "rotation": 0 },
    { "asset": "plant", "x": 10, "y": 6, "rotation": 0 }
  ]
}
```

Use the revision returned by `show`, not a guessed number. Apply replaces the
complete ordered layout; an empty list clears it. The [block contract](../../extensions/tmt-office/contracts/block-v1.md)
owns dimensions, layering and bounds. Files larger than 64 KiB or invalid objects
fail before submission. Plain output is readable JSON; `--json` is compact.

`OFFICE_REVISION_CONFLICT` means another layout won: reread and deliberately
reconcile before applying a new revision. An exact retry of identical ordered
objects at expected+1 performs no write. `OFFICE_REMOTE_UNCERTAIN` does not mean
the save failed: retain the draft, reread, and retry only the same original
revision/intent until the outcome is known. Apply returns server-confirmed current
data, which may include a subsequent editor's update; inspect it before summarizing.
Read-only or revoked grants cannot write. These commands renew eligible leases
and use the same scoped access and cleanup mechanism as inspect.

`OFFICE_BUSY` is different from an uncertain save: the local lock prevented the
operation from starting. After the other operation finishes, retry the same
revision and layout. There is no automatic queue or revision rebasing.

## Planned connected commands

The following connected behaviors are proposals, not installed instructions.
The [native pairing contract](../../extensions/tmt-office/contracts/native-pairing.md) refines
the in-progress pair/status/inspect inputs, outputs and deployment trust boundary.

| Command                                                    | Planned behavior                                                                                       |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `tmt office`                                               | Open the selected world's web UI; never implicitly run a connector or publish agents                   |
| `tmt office run`                                           | Run the connector in the foreground; Ctrl-C stops it without cancelling native work                    |
| `tmt office publish <identity> --capability review`        | Publish an explicitly selected identity UUID and allowed capability; no implicit all-agent publication |
| `tmt office unpublish <identity>`                          | Reject new work for that published identity, without deleting local identity or retained exchanges     |
| `tmt office social <identity> --minutes 10 --max-turns 20` | Request a bounded, opt-in social session; participants may decline and workspace tools stay disabled   |

## Decoration and discovery

The following syntax is proposed, not part of the shipped grammar. `status`
remains local; remote discovery reports only permitted resources. Installation,
human approval of pairing and block assignment are separate from remote work
publication. Visitors need only a browser. Installing the extension does not
provision a Firebase project or deploy a website.

| Proposed command                                                                         | Result                                                                              |
| ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `tmt office --help`                                                                      | Core-owned help, available without an installed extension                           |
| `tmt office map --json`                                                                  | Bounded permitted spatial projection, not unrestricted world enumeration            |
| `tmt office block ls --json`                                                             | Allowed blocks and assignments, without guessing IDs                                |
| `tmt office block show <block-id> --json`                                                | Canonical layout and revision                                                       |
| `tmt office block apply <block-id> --file <layout.json> --if-revision <revision> --json` | Conditional complete-layout edit, confirmed by the server; stale revisions conflict |

The local prop command surface is implemented above and owned by
[`prop-pack-v1.md`](../../extensions/tmt-office/contracts/prop-pack-v1.md); it is not duplicated
as proposed connected grammar here. No remote Rules enumeration or custom-asset
support is implied by this table.
The [sandbox design](sandbox.md) owns prop admission, identity/assignment lifetime,
untrusted content and optional contextual notices.

One-shot discovery/edits should use valid scoped credentials without requiring
`run`; continuous event/work reception requires the foreground connector.
Grant renewal and offline expiry require a defined scoped authorization contract.
Non-tmux agents use the same commands. Missing or ambiguous context fails before
mutation, never inferred from folder, pane or display name. Agent usage is:
discover permission/catalog, read layout/revision, apply once, then summarize.
After uncertain writes reread before retrying; never silently advance the expected
revision to overwrite another editor. Final envelopes, byte/query bounds and
error mappings must be fixed in the implementation ticket before adding grammar.

## Typed dispatch

The core Clap grammar owns syntax, help and completions for the maintained Office
entrypoints. It produces typed invocations and dispatches to the verified extension;
handlers never slice argv again. The versioned
[internal handshake](../../extensions/tmt-office/contracts/native-companion.md) defines the
current executable boundary. Do not create a generic
plugin platform or move all TMT commands into extensions.

## Missing extension

Interactive `tmt office` may show one prompt:

```text
Office is an optional extension and is not installed.
Install the verified Office extension? [y/N]
```

Declining exits without downloading, authenticating, pairing or changing config.
Noninteractive and `--json` calls never prompt or download. They fail promptly:

```json
{
  "error": {
    "code": "OFFICE_NOT_INSTALLED",
    "message": "Install Office with: tmt office install --yes"
  }
}
```

Office errors use the existing native `Failure::document` envelope, not another
JSON output contract. Use normal nonzero
failure exit status `1` for Office installation, compatibility, authentication,
permission and network errors; API HTTP numbers are not process exit codes.
Unsupported flags retain the native grammar's existing parse-error contract.
An installation success resumes only the original UI-opening action; it never
implies sign-in, background startup, pairing or sharing. Ordinary CLI commands
do not probe for updates or incur Office startup/network cost.

## Local and remote failure

- An incompatible extension reports `OFFICE_INCOMPATIBLE` with supported protocol
  versions and the explicit upgrade instruction; no unsafe dispatch or auto-update.
- `pair` rejects non-HTTPS remote origins. Local emulators use an explicit dev-only
  test configuration, not a production TLS bypass. Redirects cannot change the
  deployment trust target. Approval UI shows the real deployment and device.
- `run` without valid pairing reports `OFFICE_NOT_PAIRED`. A revoked device stops
  accepting work and reports `OFFICE_REVOKED`; it does not silently re-pair.
- Duplicate connector startup for one device is rejected through an owned local
  lock. Crash recovery inspects durable dispatch records before claiming work.
- Pairing cancellation follows the native contract above; unreachable remote
  state is never reported as revoked, and there is no local-only forget shortcut.
- `status --json` distinguishes installed compatibility, locally known pairing,
  process observation and the age of any last remote observation. It cannot claim
  current remote availability from a stale cache.

## Recipient experience

Social startup also requires an explicit owner-configured cost ceiling. If the
chosen provider cannot enforce the configured bounds or a tool-free context,
refuse startup rather than silently running an unbounded privileged conversation.
Provider capability and cost enforcement must be verified before enabling social startup.

The local agent receives the existing native TMT request/reply guidance, with
remote source attribution shown as untrusted presentation. The connector carries
verified remote ownership separately from that text. The native receipt stays
local and is never a bearer credential for cloud writes.

The agent replies through the existing durable native response channel, then may
show a short human-readable summary. Office displays the approved stored final,
not cropped terminal output. A completed local response with withheld export is
shown as withheld, not failed execution or an invitation to rerun the task.

Installed `skills/tmux-team/SKILL.md` and public help must match executable
commands. Proposed syntax is not installed-agent guidance.
