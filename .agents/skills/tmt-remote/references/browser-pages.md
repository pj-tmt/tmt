# Browser pages

The short `/pair#CODE` link loads only `/sdk/pair.js` first, which erases the
fragment before loading the SDK. `Pages` obtains `/sdk/pair-offer` from `Pairing`'s
current offer, sharing the descriptor with the control event; it exposes no code.
Old descriptor-path links refuse. Browser tests check stripping before the SDK
request, secret-free request URLs, owner confirmation and retained device behavior.

The embedded same-origin stylesheet projects the shared design tokens with system
font fallbacks and light/dark scheme preference under the contract-defined CSP. Every
page opens with Colab's header (mark, product, page title; the `header` token group), as
static CSS: `tests/pages.rs` fails when its metrics drift from `design/tokens/tokens.json`,
and `pairing.spec.ts` compares the rendered header with those tokens at 1440 and 390,
in both schemes. The card heading is the page's `h2`; the header title is its only `h1`.

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
