# Browser component contracts

The [browser style](gui-style.md) owns visual and interaction rules. The private
[shared browser leaf](browser-ui/README.md) owns component contracts, browser roles,
shared metrics, host metric inputs and the React/static entries. Use its README
when implementing or reviewing browser presentation; this guide is a pointer, not
another token schema or component inventory.

## Roles, metrics and generated CSS

The leaf's [entries and generation](browser-ui/README.md#entries-and-generation)
and [host metric inputs](browser-ui/README.md#host-metric-inputs) are authoritative
for browser projection and product inputs. Its
[generator](browser-ui/scripts/generate-static-css.mjs) projects the authoritative
[tokens](tokens/tokens.json) and leaf CSS into the checked static asset. Follow the
README's generation and equality-check commands; do not hand-edit generated CSS
or maintain a product-local palette or copy of the metric definitions.

## Current consumers and ownership

Colab and Remote both consume the leaf. Colab's React app uses its components and
static CSS; [Colab native chrome](../extensions/tmt-colab/rust/tmt-colab/src/chrome.rs)
and [Remote served pages](../extensions/tmt-remote/rust/tmt-remote/src/pages.rs)
embed the same checked static asset without requiring a React runtime.

Colab's product-owned choice/action menus, previews and dialogs also use flat
surfaces and thin shared edges, without hard shadows. See its
[action-menu styles](../extensions/tmt-colab/typescript/app/src/components/action-menu.css)
and [listbox styles](../extensions/tmt-colab/typescript/app/src/components/listbox.css).
Their placement and product actions remain Colab-owned; flat styling does not
make them shared leaf components.

The leaf is presentation-only and must not import product code. Products retain
routing, page geometry, persistence, authority, sessions, drafts and operation
lifetimes. Follow the leaf's
[controlled input contracts](browser-ui/README.md#controlled-input-contracts)
for Header, Notice, Field, Action, IconAction, Toggle and the other exported
components, including their inputs and accessibility. Product integration tests retain the
sending, retry, trusted-action and cleanup boundaries; leaf tests do not replace
them. Consumer changes require the owning lead's code review and UX review.

## Commands and inline code

Use the leaf's [command and inline-code presentation](browser-ui/README.md#controlled-input-contracts)
for React inputs and static classes. Products supply literal selectable text,
labels, copied bytes, actions and feedback. The leaf never reads the clipboard or
starts work. Copy success has visible **Copied.** feedback; denial leaves the
exact value available for manual selection. Hosts retain asynchronous lifetime
and permission rules. Author HTML, handbook command presentation and terminal/TUI
presentation retain their own owners.
