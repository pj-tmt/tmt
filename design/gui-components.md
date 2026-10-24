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

Both render inside the leaf's [Shell](gui-style.md#shell): Colab through `BrowserShell`, whose
header slot its `ColabHeader` portals into, and Remote's served pages through the static
`mountShell` renderer, which `remote-client` imports from the leaf by path and bundles. The
shell reads only Remote's composed module descriptions; each product supplies its own module
menu and keeps its routing.

Framework-free hosts use the leaf's [`menu-host`](browser-ui/README.md#framework-free-menu-host)
entry; it shares BrowserMenu's controller and owns transient popup presentation only.

Colab uses `BrowserMenu` for choice/action menus and `BrowserDrawer` for shared
page drawers. For non-modal side regions the leaf provides `BrowserPanel`; hosts retain their grid,
breakpoints, content and coordination with modal surfaces. Its opt-in `keepMounted` preserves
child state in a hidden, non-interactive region while closed; the default unmounts content. The leaf's [Drawer and Menu contract](browser-ui/README.md#drawer-and-menu)
owns their presentation, placement and dismissal. Colab retains commands,
checked values, admission and draft lifetimes. The [Chips contract](browser-ui/README.md#chips) owns removable items and their
scoped search popup; page-list tokens reuse its item inside the host editor. Other
input autocomplete remains a Colab-owned [listbox](../extensions/tmt-colab/typescript/app/src/components/listbox.css);
specialized transactional and attachment-viewer dialogs retain their product
owners. These surfaces use flat styling and thin edges without hard shadows.

The leaf is presentation-only and must not import product code. Products retain
routing, page geometry, persistence, authority, sessions, drafts and operation
lifetimes. Follow the leaf's
[controlled input contracts](browser-ui/README.md#controlled-input-contracts)
for Header, Notice, Field, Action, IconAction, Toggle and the other exported
components, including their inputs and accessibility. Product integration tests retain the
sending, retry, trusted-action and cleanup boundaries; leaf tests do not replace
them. Consumer changes require the owning lead's code review and UX review.

### Consumers and build

- Vite's [token projection](tokens/tokens-plugin.ts) and Rust CLI
  [theme tests](../rust/crates/tmt-cli-style/src/theme/tests.rs) consume `tokens.json`.
- Docker stages must preserve the token and checked-CSS inputs; see the
  [E2E stages](../typescript/test/e2e/Dockerfile).
- The private `design-tokens` component attributes token changes to Colab and Remote
  through `releaseConsumers` in the [component map](../.github/components.json).

## Commands and inline code

Use the leaf's [command and inline-code presentation](browser-ui/README.md#controlled-input-contracts)
for React inputs and static classes. Products supply literal selectable text,
labels, copied bytes, actions and feedback. The leaf never reads the clipboard or
starts work. Copy success has visible **Copied.** feedback; denial leaves the
exact value available for manual selection. Hosts retain asynchronous lifetime
and permission rules. Author HTML, handbook command presentation and terminal/TUI
presentation retain their own owners.
