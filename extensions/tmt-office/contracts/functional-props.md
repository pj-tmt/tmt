# World capabilities and functional extensions

Status: accepted local design. The [v1 discussion, whiteboard, broadcaster, notebook and web-link bindings](extension-v1.md)
and local whiteboard editor/persistence are implemented locally. Immutable snapshot
sharing supports explicit identities and revision-fenced meeting rosters through
the inbox. The broadcaster supports explicit no-reply announcements. Broader host actions remain planned; local implementation
is not release evidence.
The [workshop references](../../../docs/office/references/workshop/README.md) own
visual intent; [Office architecture](../../../docs/office/architecture.md) records
current code ownership.

## Responsibilities

| Boundary         | Owns                                                                                      | Must not own                                              |
| ---------------- | ----------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| TMT services     | Identity, request/reply, inbox, notes, existing discussion operations                     | World rendering or extension UI                           |
| World foundation | Spaces, placed instances, selection/camera, resource references, host capability dispatch | Discussion, drawing or broadcast business rules           |
| Extensions       | Concrete views and typed interactions for boards, notebooks and broadcasters              | Another identity registry, request engine or content copy |
| Artwork packs    | Serializable pixel appearance, directions, declared customization                         | Executable code, authority or resource contents           |

Bundled does not mean core: default extensions use the same composition boundary
as later extensions. These responsibilities do not require immediate crate moves,
a marketplace, generic event bus, plugin VM or runtime JavaScript loader. Extract
interfaces from working consumers, not speculative extensibility.

Local storage remains authoritative. CLI and host APIs call the same resource
services; browser code neither spawns shell commands nor writes SQLite. Existing
identity lifecycle events remain authoritative; extensions do not reconstruct
another lifecycle by polling.

## Serializable definitions and instances

Keep three records separate:

- **Appearance:** existing immutable prop/avatar references and validated
  customization, retaining their distinct versioned contracts and catalogs.
- **Extension definition:** a versioned data-only ID, supported World API version,
  appearance reference, resource kinds, named actions and required capabilities.
  Actions select known typed operations, never shell strings, arbitrary URLs,
  JavaScript or embedded prompt instructions.
- **Placed instance:** stable instance ID, definition reference, existing
  placement values and typed resource bindings. Multiple instances may open the
  same resource. Moving, reskinning or removing one never deletes its content.

The [v1 definition schema](extension-v1.md) and literal Rust/browser conformance
vectors accompany the first consumer. Do not silently extend strict prop v1/v2 or
layout v2/v3. Reject unknown versions, actions, kinds and fields. Unavailable
extensions preserve references and show an explicit unavailable state; they never
fall back to execution or delete resources.

Appearance admission is independent from functional admission. Authoring lint
checks direction/scale, bounds, alpha edges, text regions and customization;
definition lint also checks API versions, action schemas and binding compatibility.
Lint cannot grant capabilities or certify visual quality. Preview and visual
reference review remain required for furniture and robot appearance.

## Retained wall-mounted objects

The [platform contract](rooms-and-walls.md) owns current authoring and explicit
conversion of historical mounted placements. Wall-mounted data is retained for
existing layouts, not a second authoring mode or a new decoration catalog.
Mounted and freestanding functional objects share their resource bindings and
host actions; moving, converting or removing a placement never removes its
underlying content. Admission and projection remain with the existing placement
owners, not the artwork pack.

## Interaction affordance

The World adds a shared interaction layer to available extension instances;
artwork does not bake in a claim that an object is usable. Interactive objects
must be recognizable before hover: a crisp teal docking outline and a small
high-contrast action marker may deliberately contrast with the warm pixel room.
The shared overlay paints luminous corner brackets above the sprite, so custom
art cannot cover the affordance. No glow filter is needed for this static cue.
Hover or keyboard focus strengthens the outline and shows the action label,
such as "Open discussion board". Idle badges stay within the object's horizontal
footprint instead of overlapping adjacent objects with full-width labels. The
active label expands above its neighbors, wraps to its measured text height and
uses the same rectangles and stacking order for painting and picking. Pressing
the label retains its target; starting a pan clears the transient hover treatment.
Selection, furniture placement and interaction
use distinct treatments. Color alone is not the signal.
Accessible HUD entries use short object names and retain explicit action names for
assistive technology. They remain usable when the spatial renderer is unavailable.

Derive availability from the admitted binding and registered host handler, never
from the sprite, resource presence or an invented notification count. Unavailable
instances retain their appearance with an explicit unavailable marker and reason;
they do not show an enabled action. Decorative props receive no action marker.
Use the same dispatch path for scene activation and accessible HUD controls.

Keep these cues static while idle, with no continuous pulse, particles, blur or
animation loop. Pointer/focus transitions invalidate the scene on demand. Verify
discoverability at fitted zoom and on touch screens, keyboard parity, unavailable
states, drag-versus-click behavior and that closing a panel preserves the camera.

## Host capability facade

World calls typed TMT operations, not parsed CLI text. Initial capability families
are identity discovery, resource read/conditional update, discussion read/post/reply,
request submission/result lookup, and room delivery. The local typed HTTP adapter
now exposes retained request history, exact detail and acceptance-receipt lookup;
the local direct chat composes these reads with shared inbox dispatch. `tmt talk <target>` is
a behavioral counterpart, not a generic command-execution endpoint.

The host authenticates callers and resolves stable identity/resource IDs. Retryable
writes carry an operation ID; conditional updates also carry an expected revision.
Reuse existing receipts, request states and structured errors. Discovery describes
availability, not permission. Unsupported actions stay unavailable.

Preserve loopback session, Origin checks, bounded inputs and authorization. Host
settings/permission panels and remote controls can follow; importing a world or
definition never automatically grants host access. No unrestricted filesystem
reader, remote shell or unauthenticated endpoint is implied.

## Discussion-board extension

The physical lobby noticeboard opens a floating panel over the unchanged world;
there is no global Board tab. Desktop uses a thread list and selected discussion.
Narrow screens use one pane at a time with explicit Back navigation. Closing
restores focus to the entry point and preserves camera/selection. Keyboard and
renderer-failure entry points reach the same extension.

Categories are General and existing repository IDs. All is a combined read view,
not another stored category; add a supported paginated host query before exposing
it. Default ordering is latest activity, with bounded pagination. Unread indicators
require real read-state support, not static badges or inferred request ack state.

Reuse current threads, replies, author attribution, revision conflicts, retry
receipts and moderation. Human and agent posts share content. Posting is not
dispatch: Ask agent explicitly selects identities or a room and submits a reference
through the request service. The reference is the canonical thread UUID accepted
by `tmt office board show`, not a new URI grammar or frozen content snapshot.
Copy reference sends nothing. Failures preserve drafts;
retries do not duplicate accepted posts. No voting, karma or second content store.

## Whiteboard extension

Build a small TMT-styled whiteboard, not an Excalidraw core integration. First tools:
selection, freehand pen, text/sticky notes, simple shapes/arrows, erase and undo/redo.
One versioned scene resource stores stable element IDs and structured values.
Text is inert; arbitrary SVG/HTML and remote embedded resources are not admitted.
The pixel-world renderer does not own document editing. Load the editor on demand.

Save with conditional revisions and retain drafts on conflict. First delivery does
not require real-time multiplayer editing or CRDTs. Agents access the same resource
through typed operations; edits replacing current human work require visible
proposal/acceptance rather than a silent overwrite.

Select -> annotate -> choose agent(s) **or** a meeting room -> preview -> send.
Destination modes are explicit, not an ambiguous union. Capture an immutable
revision, selected element IDs, structured scene and image snapshot. Replies retain
that reference after later edits. Copy reference addresses the same revision,
contains no session token and dispatches nothing. Missing/expired snapshots fail
explicitly rather than resolving to current content; retention respects live references.

Verify image delivery and agent access end to end. A text inbox does not prove
vision support. Receivers without image support get structured elements and an
explicit limitation. Opening, selecting and drawing never dispatch work.

## Broadcaster and shared delivery

The broadcaster is a spatial composer over the canonical request service.
The [local dispatch capability](dispatch-v1.md) now supports explicit-recipient
inbox requests, no-reply announcements and immutable multi-recipient operation receipts. The
[meeting-room resource](meeting-room-v1.md) supplies real revision-fenced rosters.
The spatial composer shares audience, review, room fencing and retry ownership
with whiteboard requests; only message formatting and response policy differ.
Compose over `RequestService` and its transaction ports; do not clone request SQL/state policy,
reuse discussion-post receipts or infer membership from all active identities.
Preview the message and exact resolved recipients before explicit Send. Select
identities or an existing room, not every global identity implicitly. Freeze and
deduplicate recipients per operation. Show accepted requests and per-recipient
failures; retries reuse receipts instead of resending successes. Membership changes
after preview require a new preview, not hidden recipient expansion.

Offline targets follow the durable inbox contract; queue acceptance is not delivery
or completion. Ask agent requests a response. Broadcast is an explicitly labeled
announcement. Never invent automatic replies or acknowledgments. The canonical
service marks its final as not required and settles recipient attention only after
explicit ack. Follow the dispatch contract for exact request/room wire shapes.

## Acceptance and sequence

1. Finish modular room/lobby art, placement customization, compact overlay HUD and
   authoring checks; inspect rendered desktop/narrow views against references.
2. Integrate the existing discussion view as the first spatial extension. Prove
   camera/focus preservation, keyboard access, draft retention, pagination and
   failure/retry. Opening or removing the prop must not mutate discussion content.
3. Extract the smallest instance/action/capability interface demonstrated by that
   integration. Test malformed/unsupported definitions and unavailable handlers.
4. Add request/reference actions and broadcaster over the canonical request service,
   including the missing room membership and fan-out receipt boundary. Prove
   recipient scope, duplicate suppression, partial failure and offline behavior.
5. Add the simple whiteboard: scene round trips, conditional writes, undo/redo,
   immutable snapshots, agent-readable references, response association and no
   implicit dispatch.
6. Update canonical creation/Office skills with implemented schemas and commands.
   Verify serialization, rejection, resource survival after extension removal,
   lazy loading, idle rendering and complete resource disposal.

Keep local builds/reviews ahead of publication. This design does not authorize
production access, releases, host installation or public ticket writes.
