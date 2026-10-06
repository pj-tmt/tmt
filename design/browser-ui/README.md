# Private browser presentation leaf

`@tmt/browser-ui` is a private tmt-ux-owned leaf. Products may depend on it; it
must not import product code. Core and the CLI must neither depend on nor embed
it. This initial source checkpoint does not establish product adoption or release
readiness. Workspace, lockfile, shared quality and packaging integration is owned
by Infra and remains pending at this checkpoint.

## Entries and generation

There is no root entry. `@tmt/browser-ui/static.css` exports checked
`generated/static.css`; `@tmt/browser-ui/static` exports the frozen
`browserUiClasses` and presentation types without React; `@tmt/browser-ui/react`
exports `BrowserHeader`, `BrowserNotice`, `BrowserField`, `BrowserAction`,
`BrowserToggle` and their prop types. The optional peers are React 19.2.8 and
lucide-react 1.52.0. Icons are caller-supplied; static serving requires neither.

The browser color, surface and shared metric roles live in the `browser` group
of `../tokens/tokens.json`. Legacy terminal/soft roles remain separately owned.
Fonts and header metrics use the existing authoritative groups. One owner
projects these values and the five ordered CSS fragments:

```sh
node design/browser-ui/scripts/generate-static-css.mjs --write
node design/browser-ui/scripts/generate-static-css.mjs --check
```

The check compares expected bytes without writing. Consumers embed checked CSS;
Cargo and installed products never run the generator. Select light/system-dark
on the document or scope `data-theme="light"`/`data-theme="dark"` on an island.
A dark island on a light page needs dark foregrounds as well as dark surfaces.
Every component is opaque, square and shadow-free. Semantic marks retain words.
Links, essential edges, selection indicators and keyboard focus are separate.

## Host metric inputs

The leaf preserves differing product metrics through explicit custom properties;
it does not guess a product profile or replace them with fallback values. The
host defines every property used by its chosen components. Header sizes are
56px/48px below the existing 479px threshold. Page positioning, safe areas,
z-index, body offset, scrolling and full attribution disclosure remain host-owned.

| Host property suffix after `--tmt-ui-host-`                                             | Existing product values retained for adoption                                                           |
| --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `header-action-gap`                                                                     | Colab 6px                                                                                               |
| `focus-offset`                                                                          | Colab 3px; Remote 4px                                                                                   |
| `toggle-focus-offset`, `toggle-padding`                                                 | Colab 2px and 7px 8px 0                                                                                 |
| `notice-mark-size`                                                                      | Colab 18px; Remote 30px                                                                                 |
| `eyebrow-line-height`                                                                   | Host's existing line height                                                                             |
| `notice-heading-size`, `notice-heading-weight`, `notice-heading-tracking`               | Colab clamp(26px, 4vw, 36px), 700, -0.03em; Remote clamp(28px, 5vw, 36px), 650, -0.025em; settings 25px |
| `notice-body-size`, `notice-body-line-height`                                           | Colab 16px / 1.55; Remote 17px / 1.6                                                                    |
| `notice-action-gap`, `notice-action-margin`                                             | Remote 12px and 24px; form placement remains host-owned                                                 |
| `action-padding`, `action-min-height`                                                   | Colab 7px 12px with existing height; Remote 10px 22px and 44px                                          |
| `action-size`, `action-weight`, `action-line-height`, `action-font`                     | Colab existing inherited type; Remote 13px / 600 mono with existing line height                         |
| `action-gap`, `action-underline-offset`                                                 | Host's existing gap; Colab underline offset 4px                                                         |
| `field-padding`, `field-size`, `field-line-height`                                      | Colab 8px / 14px / 1.5; Remote 12px 14px / 17px with existing line height                               |
| `field-label-size`, `field-label-weight`, `field-label-line-height`, `field-label-font` | Host's existing label; Remote 12px / 600 mono with existing line height                                 |

Thin notice/action borders use the accepted 1px browser metric; field/header
borders remain 1px. This supersedes earlier shadow and opacity proposals only.
No native select, listbox, page geometry or new font loading enters this leaf.

## Controlled input contracts

Header has one h1. A supplied caption requires a stable `captionId`, retaining
full DOM copy and title association; the host supplies a visible full-caption
read path. `brandLink` preserves a caller's native anchor. `disclosure` selects a
caller-owned data attribute; the leaf does not choose or operate a menu.

Notice tone, visible state label, mark and announcement are explicit. No state,
health, permission or recovery inference occurs. Marks are decorative because
visible state words carry their meaning.

Field requires stable `controlId`, distinct IDs for rendered description/error,
and an explicit `renderControl` that spreads exactly the supplied ID, class and
ARIA props onto one native input or textarea. Host IDs precede local description
and current error IDs, with ordered deduplication. Absent content contributes no
ID; error copy does not imply invalid. The host retains value, change handler,
ref, read-only/disabled/access decisions and original control identity. Update
attributes on that same control when access changes; no remount or draft reset.

Action uses an explicit native button type, retained visible label and optional
busy mark. Keep the same optional `busyMark` supplied in ready and busy states;
its reserved slot remains in flow while visibility changes, preserving button
geometry without changing host padding or typography. Without a mark, both
states retain label-only geometry. Disabled/busy blocks callback delivery. A supplied disabled reason
requires its stable ID and remains visible. `onActivate` receives the original
React MouseEvent from native button click, including keyboard-generated click;
there is no extra key handler or command DTO. Trusted-event admission remains
with the caller. Host links remain host links.

Toggle is controlled: fixed label, `aria-pressed`, independent visible checked
indicator and original activation event. It never changes its own pressed value.
Disabled styling cannot inherit hover/selected styling. Static hosts perform the
same native semantics and ID associations themselves; CSS owns no effects.

Package-local verification after owning workspace integration:

```sh
pnpm --filter @tmt/browser-ui --fail-if-no-match check
pnpm --filter @tmt/browser-ui --fail-if-no-match test
```
