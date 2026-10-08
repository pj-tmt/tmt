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
[shared package](../../../../design/browser-ui/README.md). Remote has no copied
palette, header projection or shadow styling.

Static markup uses shared class/slot and native accessibility contracts.
`browser.ts` supplies explicit notice tones and visible state words; decorative marks
never establish access. Socket tests verify exact shared-plus-host CSS bytes and
unchanged CSP/types. Browser cases compare computed presentation with current browser
tokens at 1440/390 light/dark and 320 fit, retaining keyboard focus and disabled/loading
states. The card heading is the page's `h2`; the header title is its only `h1`.

## Browser entry

The human serve link names `/`; protocol readiness/status/SDK addresses retain `/r/<prefix>`.
`Pages` serves the static entry and same-origin `/sdk/landing.js` bootstrap under the existing
CSP/no-store/Host/Origin boundary. `browser.ts` owns its local pairing inspection and explicit
Connect action using the same IndexedDB record, non-extractable key and verified Session flow.
Inspection validates all stored identities, origin/canonical address, exact key pins and handle
consistency without a descriptor/admission call. Missing data, valid saved data and unavailable
or malformed storage remain distinct; saved data never proves current access.

Connect opens one fresh Session only on user action. Signed verification establishes
access with its checked time; an unchanged descriptor recheck after opaque404 remains
unconfirmed, not a signed refusal reason or evidence of revocation. Transport, stale descriptor or unverified reply remains unconfirmed.
Page-attempt fencing prevents late results repainting a departed page and new admission/recheck
requests after departure, including signing continuations. Already dispatched requests cannot be undone. No work/inventory/grant
repair, automatic re-pair or settings designation is performed. Pair success offers an ordinary
link back to `/`; its existing fragment erasure and terminal confirmation are unchanged.

The native CLI regression fetches the printed human URL as HTML and verifies protocol-base
refusal with zero request-triggered core calls. Existing Chromium owners cover exact pairing,
entry states, keyboard activation, storage/transport/pin errors, refusal, token metrics and
1440/390 light/dark captures plus a 320-pixel fit check. The SDK fixture covers complete stored-record guards
and page replacement; actual browser/terminal fixtures remain the user-flow evidence owner.
