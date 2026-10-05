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
