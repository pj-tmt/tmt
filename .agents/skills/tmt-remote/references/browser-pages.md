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
controls at least 36px on desktop and 44px on narrow/touch screens, with flat, unframed states. Its six static templates carry the approved
inline TMT mark (one even-odd path in a 96 × 96 viewBox, currentColor fill) from the
leaf’s static-host contract (#2608), pinned by a Remote-owned literal
test; Cargo does not read the leaf’s React source or fetch the mark at runtime.

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

## Public short entries

Remote forwards the allowlisted `/colab`, `/colab/`, `/p/<id>` and `/read/<id>` entries
in place to Colab's public mount socket, with the Remote-computed `tmt-mount` header.
It owns no Colab shell, catalog, capability or CSP; no cookie/session lookup occurs.
The [contract](../../../../contracts/remote-channel-v1.md#transport-binding-loopback-http)
owns grammar, admission and reply policy. Real-door tests pin exact extension paths,
header isolation, generic unavailable refusals and unchanged SDK mapping. Browser seam
fixtures assert no Remote redirect, unchanged short URLs on reload and fragment secrecy;
Colab's integrated app-routing acceptance remains Colab-owned.

## Settings page

`/remote/settings` uses the same embedded shared CSS, Header/Notice/Field/Action classes and
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
