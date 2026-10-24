# Browser pages

The Remote-owned pairing HTML inlines `assets/pair.js` as its first synchronous script under
an exact CSP hash. It captures/erases the fragment before any script import or subresource;
`no-referrer` and `no-store` remain. `Pages` exposes only the public current descriptor through
`/sdk/pair-offer`. [Pairing](../../../../contracts/remote-channel-v1.md#pairing) owns the manual
ceremony, native `#auto=CODE` first-use flow, cancellation proof and local-process boundary.
The automatic flow labels its page “Connect this browser”, hides the pairing eyebrow and uses
“Not connected” for its blocked state; manual pairing retains its own wording.
The automatic form is hidden before paint; the SDK either reuses/cancels with the stored device
or enrolls exactly the read-only default, then replaces the same tab with its validated target.
No code enters logs, requests, ordinary output or continuation links. Used/cancelled/expired
codes show a plain refusal and a machine-side reopen command. Storage and unconfirmed
reopen failures retain the saved pairing, with no pair-again instruction or silent enrollment.
Only a genuinely absent or verified revoked/expired pairing permits enrollment. Commands use code styling and stay unbroken within the pairing status.
A same-document fragment change restarts capture/erasure and cancels the previous page attempt. Browser fixtures cover erasure before SDK loading, first enrollment,
returning-browser cancellation, another profile's replay refusal and code-free Back/Forward.

Manual initialization keeps Pair disabled until the descriptor and submit listener are ready.
Its keyboard flow keeps the named comparison region, explicit word confirmation, retained key,
non-stolen focus and Return to Remote behavior. Failed setup never enables enrollment. Existing
manual/foreign-origin/expiry/proof controls remain, with no extra authentication gate for Firebase.

`Pages` embeds checked `design/browser-ui/generated/static.css` followed by
Remote's `assets/pages.css` with compile-time `concat!(include_str!(...))`.
Cargo and installed serving never run Node or a generator. The shared package owns
browser roles, fonts/header tokens and Header/Notice/Field/Action presentation;
Remote defines host metrics, viewport layout, safe-area offsets and one window
scrollbar. Regeneration and drift checks belong to the
[shared package](../../../../design/browser-ui/README.md). Remote has no copied palette or shadow styling. Host pages use 14px body text,
controls at least 36px on desktop and 44px on narrow/touch screens, with flat, unframed states. Static hosts use the shared aurora SVG from the
leaf’s static-host contract, pinned by Remote-owned literal
tests; Cargo does not read the leaf’s React source or fetch the mark at runtime.

Static markup uses shared class/slot and native accessibility contracts.
`browser.ts` supplies explicit notice tones and visible state words; decorative marks
never establish access. Socket tests verify exact shared-plus-host CSS bytes and
unchanged CSP/types. Browser cases compare computed presentation with current browser
tokens at 1440/390 light/dark and 320 fit, retaining keyboard focus and disabled/loading
states. The card heading is the page's `h2`; the header title is its only `h1`.

## Browser entry

The human serve link names `/`; protocol readiness/status/SDK addresses retain `/r/<prefix>`.
Home and TMT settings follow the [public route contract](../../../../contracts/remote-channel-v1.md#public-browser-routes); they expose only navigation, no state or authority.
`Pages` serves Remote's entry at `/remote/` and same-origin `/sdk/landing.js` bootstrap under the existing
CSP/no-store/Host/Origin boundary. `browser.ts` validates local pairing before network, then checks
once on page open through descriptor, verified Session and scope-free `capabilities` observation.
Manual Check again reuses the current Session. An expired Session is silently reopened once
within the manual check; an ended capabilities response is not a pairing refusal. No polling,
inventory, work or grant repair occurs.
Missing/unreadable storage sends nothing. Seven explicit icon-and-text states distinguish local
pairing, checking, connected, different machine, signed refusal and unconfirmed access. Generic
Connected uses no invented friendly name. Technical evidence and local-time checked time stay in
native Details; command Copy actions report denial honestly and leave selectable text.

Only signed refusal supports Not accepted. Its body names pairing removal/expiry only for verified
revoked/expired authority; a session-ended/evicted body offers Check again without claiming pairing
loss. Opaque404 remains unconfirmed even after a
descriptor-only recheck, never another admission. Page-lifetime/signing fences prevent new
requests after departure; per-check fences prevent late painting. A valid stored pairing short-circuits manual enrollment, leaves its offer open and continues
to `/remote/`; unreadable/unconfirmed state never enrolls silently. Pair success still links back
to `/remote/`, with unchanged fragment erasure, terminal confirmation and disabled-until-ready controls.

The native CLI regression fetches the printed human URL as HTML and verifies protocol-base
refusal with zero request-triggered core calls. Existing Chromium owners cover exact pairing,
entry states, keyboard activation, storage/transport/pin errors, refusal, token metrics and
1440/390 light/dark captures plus a 320-pixel fit check. The SDK fixture covers complete stored-record guards
and page replacement; actual browser/terminal fixtures remain the user-flow evidence owner.

The landing and settings pages consume the SDK's origin-local public registry. Its list, Add,
rename/remove and confirmed collision-aware import follow the
[browser registry contract](../../../../contracts/remote-channel-v1.md#browser-remote-registry).
Handoff fragments are erased before admission. Registry changes touch only the public
`remotes-v1` key; the existing device record and non-extractable handle remain unchanged.
Current access reuses the page’s existing observation; foreign entries remain `not checked`, without
probes or a state mark. Firebase Open preserves its validated public link and does not send
catalog data. The static Remote chip/menu reuses this controller and current-origin observation, not the
Colab switcher or a default selector. Native popover focus/roles follow the shared BrowserMenu
contract; foreign Open navigates without retargeting an existing client. Settings blocks foreign
Open while a request, unknown original outcome or unsent device name remains; the existing
reason and original recovery stay on that page. Add/Manage open settings’s Remotes section.
No optional signed catalog/default read or client factory is shipped.

## The one web app and public entries

Every app route (`/`, `/settings`, `/remote/` with `/remote/devices`, `/remote/connect`, `/remote/machine` and `/remote/settings`, `/ops`, `/digest`, `/colab/…`)
is the static `assets/shell.html`; `shell-app.ts` routes inside it, adopting each page's markup
from `assets/*.html` and loading Colab's module from the composed shell's `entry`. The pairing
page stays its own document and mounts the rail only.
Remote is one route (`remote-routes.ts` is its table of paths and rows). `remote-app.ts` mounts both
pages' markup once and shows the view the path names (`data-view` on the markup), so a switch
between Remote's rows swaps no slot and loads nothing; each page's code starts when its first view
shows and shares one remotes controller. The module menu holds no position of its own. `/p/<id>` answers 308 to `/colab/p/<id>`.
Remote forwards only `/read/<id>` in place to Colab's public mount socket, with the
Remote-computed `tmt-mount` header.
For that entry it owns no Colab catalog, capability or CSP; no cookie/session lookup occurs.
The [contract](../../../../contracts/remote-channel-v1.md#transport-binding-loopback-http)
owns grammar, admission and reply policy. Real-door tests pin exact extension paths,
header isolation, generic unavailable refusals and unchanged SDK mapping. Browser seam
fixtures assert no Remote redirect, unchanged short URLs on reload and fragment secrecy;
Colab's integrated app-routing acceptance remains Colab-owned.

## Owner reads in the app

`owner-reads.ts` is the one client for the [owner browser reads](../../../../contracts/remote-channel-v1.md#owner-browser-reads):
`ownerRead(name, signal)` reads `<mount>/x/-/<name>` (the mount from the shell's `shellAddress`)
and checks the versioned envelope, so a page gets `ok` with the payload, `unavailable` with a
contract reason (an unknown one is `unknown`), `no-session` (no owner mount, or no pairing to open
one) or `unreachable` (no read, no network). The door's generic 404 means its owner session ended,
as on a door restart: the read reopens it from the browser's pairing once and reads again. Reads
that meet the same restart, such as Digest's digest and limits, share that one reopen; it stops
only when every waiting read's signal aborted. `whileVisible(intervalMs, signal, tick)` refreshes only while the route is mounted and
the document visible, at once on showing again, doubling the wait after each failed read up to
eight times. `/ops` reads `ops` every 5 s through `ops-source.ts`, repaints only when the board
changed and gives each reason one sentence with what to do; Digest uses the same client.
`test/owner-reads.test.ts` runs every vector of `owner-reads-v1.json` through it.

`ops-page.ts` owns the section menu and that read lifetime; `ops-agents.ts`,
`ops-squads.ts` and `ops-timers.ts` render the observed board through shared
`ops-parts.ts` presentation helpers. Squads shows member cards and delegates its
read-only detail dialog to `ops-drawer.ts`. A refresh updates an open detail view;
an absent agent is reported without retaining stale facts. Dismissal resolves the
current member button after refresh, with the Squads heading as its focus fallback.
Leaving the section or aborting the route closes the dialog. These views add no
agent or schedule controls, owner reads or persistence.

## Settings page

The settings views (`/remote/devices`, `/remote/machine`, `/remote/settings`) use the same embedded shared CSS, Header/Notice/Field/Action classes and
Remote host metrics; its additional stylesheet owns settings layout and static listbox geometry.
Normal settings use thin section rules, a desktop summary/control grid and a single-column
narrow layout. Each setting keeps its unchanged hint and original-result feedback beside its
Save action; loading triggers show no invented default value.
Button-triggered, named listboxes use native top-layer popovers with shared static menu CSS:
arrow/Home/End/typeahead navigation, Enter/Space choice, Escape/Tab dismissal and trigger focus
return. Choice changes stay local until Save; the display-only unset Default stays disabled.
The [settings-page contract](../../../../contracts/remote-channel-v1.md#remote-settings-page) owns second-press
consequences, draft retention and navigation guards. `settings-controls` supplies presentation;
no operation ID exists before a deliberate confirmation against the admitted target.
The existing page and CSP owners remain unchanged. Remote's `management-page` owns admitted values,
frozen intent/outcome and separate current-access state; `settings-controls` owns the two static
listboxes, while `settings-page` binds forms and inline consequences without
resetting unsent text during async failure/read refresh. Refresh retains the current device
page; navigation focuses and refuses to leave an unsent device name until saved or restored,
so only one bounded page of forms is retained. The separately generated
`settings-v1.js` imports the single served public SDK, without another key/channel owner.
Both generated modules must pass rebuild byte equality. The owning
[management protocol](../../../../contracts/remote-channel-v1.md#remote-settings-page)
defines the unlimited default/source, capacity, durable last use and atomic remove-others behavior.
Only the locally designated browser may remove devices; read-only pages show its exact designation
command. Removed rows disappear without deleting revocation tombstones.

The existing Chromium pairing fixture also exercises the actual settings page: designated/
read-only values, exact custom cap/off/default source, role removal, retained drafts/caret,
explicit sending-scope enable/disable, lost self-change acknowledgments, original-only recovery,
no resend and process cleanup. Sending controls preserve device-name drafts and other grant policy.
The single frozen original projects into one pre-mounted empty polite status slot beside its setting
or device actions; an absent row falls back above the forms. Recovery stays beside the unknown
result and reads only that original once. Its row omits a duplicate disabled reason; other rows
say “Another change is still unconfirmed. Check its original result first.” Original IDs appear only for
unknown or refused outcomes. Capacity uses “Browser change limit reached. Use the local CLI; do not
retry or reset storage.” Unchanged settings Saves stay disabled with an associated
no-change hint;
draft comparison retains unset/default provenance, and submit-time guards allocate no operation.

A >25-device scenario covers later-page drafts through refresh, another committed effect,
original-receipt recovery and guarded first/next navigation.
Use `TMT_REMOTE_CAPTURE_DIR` for light/dark 1440/390 settings captures; 320 fit is asserted.
Captures include loading/read-only/disabled reasons, drafts/focus, outcome/recovery and self-change
states at 1440/390 light/dark and 320 fit. Refresh restores its initiating keyboard control
only when the person has not moved elsewhere. Device forms and action names use the admitted
device name, never an unsent draft. The capture index records the served state and provenance.

The Firestore section is read-only: it explicitly requests the signed settings projection,
sharing status's recorded layer evidence and dated published limits. Unsupported layers use
one not-available line, even when their other prerequisites are off or unknown. The page
never displays reason codes, measured usage or an invented quota warning; ready sharing includes the
[contract's fixed limits note](../../../../contracts/remote-channel-v1.md#local-cli-discovery). Native Details
contains the wrapping budget table and its date, Pacific reset and local guard thresholds.
An optional observation failure clears stale evidence without overwriting management access,
original outcome or drafts. Fixtures cover every prerequisite; capture labels distinguish
injected tier/quota from private recorded evidence and never claim provider acceptance.

Unchanged access and Firestore projections do not repaint on unrelated draft input. Short polite
slots announce only changed settled states; the layer lists, budget and Details stay outside
those live regions. Presentation equality never suppresses signed reads or authority checks.

## Digest presentation

`digest-page` groups activity into clock-aligned four-hour (24h), daily (7d) and five-day (30d)
bars clipped to the reported window; any missing constituent keeps the entire bar hollow.
Token areas use the leaf's pastel seat palette and monotone interpolation; sampled values and
incomplete-bucket notes remain unchanged. `digest-page` derives the allowance rows, combined token chart and seat legend, activity and
message views, and coverage/method footer from `DigestSource`. Allowance uses a fixed
`digest/7d` owner read independently of the selected usage range (the 7d selection shares that
read); `limits` supplies history. Each mounted source reuses that one allowance result
across range switches (including in-flight reads). A switch reads only its selected usage
report; 7d reuses the weekly report itself. The minute refresh obtains a fresh allowance
result. There is no global cache. Limit titles come only from the explicit source `provider`,
never seat model families. Active weekly windows group by provider, duration and reset; the
newest observation reported by a current seat leads each provider, with other windows folded
and labeled by reset, reporter seats and observation age. Histories match that exact window.
Claude stays explicitly not reported until Core has a real source. Provider history and method
stay disclosures. The page owns range, scale, expanded legend and disclosure state; changed
reports restore open disclosures and summary focus, while unchanged reads retain the existing
DOM and scroll position. Failed refreshes retain the last good report for that range; an
unavailable newly selected range does not display the preceding range's observations. The
route signal owns refresh cleanup. The page uses existing owner reads and does not change allowance accounting or persistence.
