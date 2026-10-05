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

## Settings page draft

`/settings` uses the existing page/CSP/token owners for a static draft while shared #1797
presentation exports remain pending. Remote's `management-page` owns admitted values,
frozen intent/outcome and separate current-access state; `settings-page` binds forms without
resetting unsent text during async failure/read refresh. Refresh retains the current device
page; navigation focuses and refuses to leave an unsent device name until saved or restored,
so only one bounded page of forms is retained. The separately generated
`settings-v1.js` imports the single served public SDK, without another key/channel owner.
Both generated modules must pass rebuild byte equality. The owning
[management protocol](../../../../contracts/remote-channel-v1.md#remote-settings-page-draft)
defines default/source, capacity and one-attempt live-grant recovery behavior.

The existing Chromium pairing fixture also exercises the actual settings page: designated/
read-only values, exact custom cap/off/default source, role removal, retained drafts/caret,
lost self-rename/revoke acknowledgments, original-only recovery, no resend and process cleanup.
A >25-device scenario covers later-page drafts through refresh, another committed effect,
original-receipt recovery and guarded first/next navigation.
Use `TMT_REMOTE_CAPTURE_DIR` for light/dark 1440/390 settings captures; 320 fit is asserted.
These static-draft checks do not substitute for shared adoption, UX or complete feature gates.
