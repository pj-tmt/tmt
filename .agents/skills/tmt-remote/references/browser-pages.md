# Browser pages

The short `/pair#CODE` link loads only `/sdk/pair.js` first, which erases the
fragment before loading the SDK. `Pages` obtains `/sdk/pair-offer` from `Pairing`'s
current offer, sharing the descriptor with the control event; it exposes no code.
Old descriptor-path links refuse. The initial HTML disables Pair before the SDK
loads; `browser.ts` enables it only after validating the current offer and installing
the submit listener. Failed initialization keeps pairing unavailable. Browser tests
hold SDK/offer readiness and verify early mouse/Enter produces no native submission
or candidate, then explicit ready-state activation retains the same ceremony. They
also check fragment stripping, secret-free request URLs, owner confirmation and
retained device behavior.

`Pages` embeds checked `design/browser-ui/generated/static.css` followed by
Remote's `assets/pages.css` with compile-time `concat!(include_str!(...))`.
Cargo and installed serving never run Node or a generator. The shared package owns
browser roles, fonts/header tokens and Header/Notice/Field/Action presentation;
Remote defines host metrics, viewport layout, safe-area offsets and one window
scrollbar. Regeneration and drift checks belong to the
[shared package](../../../../design/browser-ui/README.md). Remote has no copied palette or shadow styling. Its four static templates carry the approved
inline aperture mark from the leaf’s static-host contract (#2207), pinned by a Remote-owned literal
test; Cargo does not read the leaf’s React source or fetch the mark at runtime.

Static markup uses shared class/slot and native accessibility contracts.
`browser.ts` supplies explicit notice tones and visible state words; decorative marks
never establish access. Socket tests verify exact shared-plus-host CSS bytes and
unchanged CSP/types. Browser cases compare computed presentation with current browser
tokens at 1440/390 light/dark and 320 fit, retaining keyboard focus and disabled/loading
states. The card heading is the page's `h2`; the header title is its only `h1`.

## Browser entry

The human serve link names `/`; protocol readiness/status/SDK addresses retain `/r/<prefix>`.
`Pages` serves the static entry and same-origin `/sdk/landing.js` bootstrap under the existing
CSP/no-store/Host/Origin boundary. `browser.ts` validates local pairing before network, then checks
once on page open through descriptor, verified Session and scope-free `capabilities` observation.
Manual Check again reuses the current Session. An expired Session is silently reopened once
within the manual check; an ended capabilities response is not a pairing refusal. No polling,
inventory, work or grant repair occurs.
Missing/unreadable storage sends nothing. Seven explicit icon-and-text states distinguish local
pairing, checking, connected, different machine, signed refusal and unconfirmed access. Generic
Connected uses no invented friendly name. Technical evidence and local-time checked time stay in
native Details; command Copy actions report denial honestly and leave selectable text.

Only signed refusal supports Not accepted. Opaque404 remains unconfirmed even after a
descriptor-only recheck, never another admission. Page-lifetime/signing fences prevent new
requests after departure; per-check fences prevent late painting. Pair success still links back
to `/`, with unchanged fragment erasure, terminal confirmation and disabled-until-ready controls.

The native CLI regression fetches the printed human URL as HTML and verifies protocol-base
refusal with zero request-triggered core calls. Existing Chromium owners cover exact pairing,
entry states, keyboard activation, storage/transport/pin errors, refusal, token metrics and
1440/390 light/dark captures plus a 320-pixel fit check. The SDK fixture covers complete stored-record guards
and page replacement; actual browser/terminal fixtures remain the user-flow evidence owner.

## Settings page

`/settings` uses the same embedded shared CSS, Header/Notice/Field/Action classes and
Remote host metrics; its additional stylesheet owns only settings layout and native selects.
The existing page and CSP owners remain unchanged. Remote's `management-page` owns admitted values,
frozen intent/outcome and separate current-access state; `settings-page` binds forms without
resetting unsent text during async failure/read refresh. Refresh retains the current device
page; navigation focuses and refuses to leave an unsent device name until saved or restored,
so only one bounded page of forms is retained. The separately generated
`settings-v1.js` imports the single served public SDK, without another key/channel owner.
Both generated modules must pass rebuild byte equality. The owning
[management protocol](../../../../contracts/remote-channel-v1.md#remote-settings-page)
defines default/source, capacity and one-attempt live-grant recovery behavior.

The existing Chromium pairing fixture also exercises the actual settings page: designated/
read-only values, exact custom cap/off/default source, role removal, retained drafts/caret,
explicit sending-scope enable/disable, lost self-change acknowledgments, original-only recovery,
no resend and process cleanup. Sending controls preserve device-name drafts and other grant policy.
A >25-device scenario covers later-page drafts through refresh, another committed effect,
original-receipt recovery and guarded first/next navigation.
Use `TMT_REMOTE_CAPTURE_DIR` for light/dark 1440/390 settings captures; 320 fit is asserted.
Captures include loading/read-only/disabled reasons, drafts/focus, outcome/recovery and self-change
states at 1440/390 light/dark and 320 fit. The capture index records the served state and provenance.
